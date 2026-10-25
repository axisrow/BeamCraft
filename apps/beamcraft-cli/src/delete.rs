// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P08 adapter: `DeleteElement` for a plate, a member, an A01 assembly, an A06 weld or a D2 level,
//! committed as the next revision.
//!
//! The delete removes the whole element, its authored prim and its result prims, and nothing else:
//! the other elements, the unknown content and the immutable definitions it pinned stay. Before any
//! layer is touched the layer engine's element index decides whether something still links to the
//! element ([`Author::check_delete`]): an incoming relationship is `DEPENDENCY_CONFLICT`, an index
//! that cannot prove the links complete is `DEPENDENCY_UNAVAILABLE`, and there is no cascade and no
//! dangling link. The layer engine's write set is checked against the footprint the engine declares.
//!
//! Honesty limits: replay of an `op_id` is decided from the head model (the delete is a replay when
//! the element is already absent and the ledger proves the operation committed), not from a stored
//! canonical request. The element relationship authored today is the A01 `part-of` record, so an
//! assembly with parts cannot be deleted; deleting a part removes its own record with it. One
//! implicit workspace, in-process layer
//! engine, as in `plate`.

use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    decode, digest_of, evaluate_decoded, host_uuid, integrity, load_base, open_base, prior_op_with,
    replay_summary, summarize,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{Author, Change, EditTargets, Prepare, schema_identity};
use beamcraft_layer_engine::index::ElementId as LayerElementId;
use beamcraft_model::command::{CommandError, DeleteElement};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};

/// Inputs of one `delete_element` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct DeleteRequest {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; a delete in such a store is `NOT_FOUND`.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: DeleteElement,
}

/// What was deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletedKind {
    /// A plate.
    Plate,
    /// A beam or a column.
    Member,
    /// An assembly (A01); refused while it still has parts.
    Assembly,
    /// A weld (A06), a `connection` element; deleting it removes its `connects` record.
    Weld,
    /// A bolt group (A05), a `connection` element; deleting it removes its `connects` record.
    Bolt,
    /// A level (D2); refused while a plate, beam or column is assigned to it.
    Level,
}

/// Result of `delete_element`.
#[derive(Debug, Clone, PartialEq)]
pub struct Deleted {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The kind of the deleted element (`None` for a replay: it is no longer in the head).
    pub kind: Option<DeletedKind>,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
}

fn not_found() -> PlateError {
    PlateError::from(CommandError::new(
        ErrorCode::NotFound,
        "target_id",
        "the element is not in the head revision",
    ))
}

/// Deletes one element and publishes the next revision, or replays the operation.
///
/// Steps, each failing closed: open and verify the head; find the target; decide replay from the
/// host ledger; build the layer engine's element index and refuse a delete that would leave a
/// dangling link; author the removal with U02; check the layer engine's writes against the declared
/// footprint; commit through U06/U07 with the strict verifier.
pub fn delete_element(
    store: &RevisionStore,
    request: &DeleteRequest,
    hook: &mut dyn CommitHook,
) -> Result<Deleted, PlateError> {
    let command = &request.command;
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    if head.is_none() {
        return Err(not_found());
    }
    let before: std::collections::BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let base_request = open_base(&base)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let (plate_ids, members) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (_, members) = evaluate_decoded(&decoded)?;
        (
            decoded
                .plates
                .iter()
                .map(|(p, _)| p.id)
                .collect::<std::collections::BTreeSet<_>>(),
            members,
        )
    };
    let kind = if plate_ids.contains(&command.target_id) {
        Some(DeletedKind::Plate)
    } else if members.iter().any(|r| r.authored.id == command.target_id) {
        Some(DeletedKind::Member)
    } else if crate::assembly::is_assembly(&base_request, &command.target_id, &control)? {
        Some(DeletedKind::Assembly)
    } else if crate::weld::is_weld(&base_request, &command.target_id, &control)? {
        Some(DeletedKind::Weld)
    } else if crate::bolt::is_bolt(&base_request, &command.target_id, &control)? {
        Some(DeletedKind::Bolt)
    } else if crate::level::is_level(&base_request, &command.target_id, &control)? {
        Some(DeletedKind::Level)
    } else {
        None
    };

    let op_host = host_uuid(&request.op_id.to_string())?;
    let fingerprint = command.content_fingerprint()?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        // The head holds the result of this delete exactly when the element is absent.
        if kind.is_none() {
            Ok(fingerprint)
        } else {
            digest_of(b"no accepted request matches the head state")
        }
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            return Ok(Deleted {
                replayed: true,
                kind: None,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
            });
        }
        PriorOp::Accepted { .. } => {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        PriorOp::Unseen => {}
    }
    let Some(kind) = kind else {
        return Err(not_found());
    };

    let footprint = Footprint::for_delete(command.target_id)?;
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        base_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let id_text = command.target_id.to_string();
    let index = author.element_index(&opened.context, &control)?;
    author.check_delete(&index, &LayerElementId::parse(&id_text)?, &control)?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::Delete { id: id_text }],
        },
        &control,
    )?;
    check_footprint(&prepared, &footprint)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &footprint,
            fingerprint,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &request.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(Deleted {
        replayed: false,
        kind: Some(kind),
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
    })
}
