// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 helper creation and reads through verified USD revisions.

use crate::assembly::read_prims;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, base_of, check_footprint,
    control, digest_of, host_uuid, integrity, load_base, open_base, prior_op_with, property,
    replay_summary, summarize, text,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::helper::Helpers;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Placement as LayerPlacement, Prepare, Shape,
    schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Prim, Reader};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use beamcraft_model::{command::CommandError, helper::Helper};
use std::collections::BTreeMap;

/// Identities of one T02 call beyond the command itself.
#[derive(Debug, Clone)]
pub struct HelperContext {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; its first revision records it.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
}

/// Result of one T02 creation.
#[derive(Debug, Clone, PartialEq)]
pub struct HelperChange {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The helpers of the resulting head.
    pub helpers: Vec<Helper>,
}

/// The kinds, helpers and helper records of a pinned set of prims. Elements of foreign or unknown
/// kinds count as used identities and nothing else.
pub(crate) fn helpers_of(prims: &[Prim]) -> Result<Helpers, PlateError> {
    let mut helpers = Helpers::default();
    for p in prims {
        let Some(id) = p
            .element_id
            .as_deref()
            .and_then(|s| ElementId::parse(s).ok())
        else {
            continue;
        };
        if property(p, "bim:elementKind").and_then(text) == Some("helper")
            && property(p, "bim:geometryMode").and_then(text) == Some("parametric")
            && property(p, "bim:schemaVersion").and_then(text) == Some("0.1.0")
            && p.active
            && property(p, "bim:isDeactivated") != Some(&beamcraft_layer_engine::Value::Bool(true))
            && !p.sources.is_empty()
            && p.sources.iter().all(|s| s.owned)
        {
            helpers.insert(beamcraft_layer_engine::authored::helper::read(p)?)?;
        } else {
            helpers.insert_other(id);
        }
    }
    Ok(helpers)
}

pub(crate) fn model_of(request: &OpenRequest, control: &Control) -> Result<Helpers, PlateError> {
    // Run structural validation before interpreting any helper geometry.
    validate_layers(request.clone(), schema_identity()?, control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), control)?;
    helpers_of(&read_prims(&reader, &opened.context, control)?)
}

/// Helpers of a pinned model in stable identity order.
pub fn views(helpers: &Helpers) -> Vec<Helper> {
    helpers.helpers().cloned().collect()
}

/// Whether the pinned model holds a helper of this identity (for `delete-element`).
pub(crate) fn is_helper(
    request: &OpenRequest,
    id: &ElementId,
    control: &Control,
) -> Result<bool, PlateError> {
    validate_layers(request.clone(), schema_identity()?, control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), control)?;
    // Deactivated helpers remain deletable, although queries exclude them from snapping.
    Ok(read_prims(&reader, &opened.context, control)?
        .iter()
        .any(|p| {
            p.element_id.as_deref() == Some(id.to_string().as_str())
                && property(p, "bim:elementKind").and_then(text) == Some("helper")
                && property(p, "bim:schemaVersion").and_then(text) == Some("0.1.0")
                && !p.sources.is_empty()
                && p.sources.iter().all(|s| s.owned)
        }))
}

/// Reads helpers from exactly one verified revision, retaining its pin for consumers.
pub fn read_helpers(
    store: &RevisionStore,
) -> Result<(Option<RevisionSummary>, Helpers), PlateError> {
    let revision = match store.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => return Ok((None, Helpers::default())),
        Err(error) => return Err(error.into()),
    };
    let helpers = model_of(&open_base(&base_of(&revision)?)?, &control())?;
    Ok((Some(summarize(revision.info())), helpers))
}

/// Helpers of the verified head, in stable identity order.
pub fn helpers(store: &RevisionStore) -> Result<Vec<Helper>, PlateError> {
    Ok(views(&read_helpers(store)?.1))
}

/// The helper `id` as the operation committed at `sequence` created it: present in that revision
/// and absent (as any element) from the revision before it. `None` when that revision did not
/// create such a helper, so the operation id belongs to another request.
fn created_by(
    store: &RevisionStore,
    sequence: u64,
    id: &ElementId,
    control: &Control,
) -> Result<Option<Helper>, PlateError> {
    let at = store.open_at(sequence)?;
    let model = model_of(&open_base(&base_of(&at)?)?, control)?;
    let Some(record) = model.helper(id).cloned() else {
        return Ok(None);
    };
    if sequence > 1 {
        let before = store.open_at(sequence - 1)?;
        if model_of(&open_base(&base_of(&before)?)?, control)?.contains(id) {
            return Ok(None);
        }
    }
    Ok(Some(record))
}

/// Creates one helper and publishes the next revision (the first one records the project), or
/// replays the operation.
///
/// Steps, each failing closed: admit the command; open and verify the head; decide replay from
/// the host ledger and the head state; refuse a used identity; author the helper with U02; check
/// the layer engine's writes against the declared footprint; commit through U06/U07 with the
/// strict verifier.
pub fn create_helper(
    store: &RevisionStore,
    context: &HelperContext,
    command: &Helper,
    hook: &mut dyn CommitHook,
) -> Result<HelperChange, PlateError> {
    command.validate()?;
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let base_request = open_base(&base)?;
    let model = model_of(&base_request, &control)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let op_host = host_uuid(&context.op_id.to_string())?;
    // The ledger proves that the operation committed; what it committed is read from its own
    // revision, never from the head, so another helper that happens to match is not a replay.
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    let accepted = match (&prior, &recorded) {
        (PriorOp::Accepted { .. }, Some((_, sequence, _))) => {
            created_by(store, *sequence, &command.id, &control)?
        }
        _ => None,
    };
    let same = accepted.as_ref().is_some_and(|r| r == command);
    match prior {
        PriorOp::Accepted { .. } if same => {
            return Ok(HelperChange {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                helpers: views(&model),
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
    let creation = model.create(command)?;
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
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::Create(Element {
                id: command.id.to_string(),
                name: command.name.clone(),
                shape: Shape::Helper(command.geometry.clone()),
                placement: LayerPlacement {
                    translation_mm: [0.0; 3],
                    orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                },
                material: None,
            })],
        },
        &control,
    )?;
    check_footprint(&prepared, &creation)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &creation,
            fingerprint: command.content_fingerprint()?,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &context.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(HelperChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(creation),
        changed_layers,
        helpers: helpers(store)?,
    })
}
