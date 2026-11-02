// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P05 adapter: `CopyElement` for a plate or a member, committed as the next revision.
//!
//! The copy gets a fresh identity and the source's authored inputs, so its element-local body is
//! the source's body: the derived mesh and the stored quantity records are the same bytes and the
//! mesh digest is shared. The source prims are not written (the footprint says so and is checked
//! against the layer engine's own write set).
//!
//! Honesty limits: the mesh is still stored once per element in the derived layer, so "shared" is
//! the equal content digest, not one stored asset; replay of an `op_id` is decided from the head
//! model (the copy is a replay when the head already holds the copy of the source's current
//! state), as in `edit`. The layer engine's `copy_plan` remaps nothing beyond the definition links
//! the copy keeps and the level assignment (D2), which the copy shares: it stays on the level of
//! its source. The duplicate joins no assembly and takes no connections (DV-REL-011).
//! Other unsupported relationships still refuse the copy.

use crate::definition::material_lookup;
use crate::plate::{decode, evaluate_decoded, part_mesh_change, prior_op_with, replay_summary};
use crate::publication::{PublicationTail, prepare_and_publish};
use beamcraft_engine::footprint::Footprint;
use beamcraft_host::revision::{CommitHook, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{
    Change, Element, Placement as LayerPlacement, schema_identity,
};
use beamcraft_layer_engine::index::{ElementId as LayerElementId, LinkTarget};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};
use session::Refusal;
use session::pipeline::{
    Base, RevisionSummary, control, digest_of, host_uuid, integrity, load_base, open_base,
    summarize,
};
pub use steel::engine::part::PartRecord;
use steel::engine::transaction::PriorOp;
use steel::layer as steel_layer;
use steel::model::command::CopyElement;

/// Inputs of one `copy_element` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct CopyRequest {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; a copy in such a store is `NOT_FOUND`.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: CopyElement,
}

/// Result of `copy_element`.
#[derive(Debug, Clone, PartialEq)]
pub struct Copied {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The copy as it is in the committed model.
    pub element: PartRecord,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
}

pub(crate) fn not_found() -> Refusal {
    Refusal::from(CommandError::new(
        ErrorCode::NotFound,
        "source_id",
        "the source element is not in the head revision",
    ))
}

/// The authored changes one copied part needs: its class, its create and its mesh. Shared by
/// the single copy (P05) and the array copies (T03), which author the same per-copy records.
pub(crate) fn creation_changes(copy: &PartRecord) -> Result<Vec<Change>, Refusal> {
    let id = copy.id().to_string();
    let placement = copy.placement();
    let [qx, qy, qz, qw] = placement.orientation_xyzw;
    Ok(vec![
        Change::PartClass {
            id: id.clone(),
            class: copy.class(),
        },
        Change::Create(Element {
            id,
            name: copy.name().to_owned(),
            shape: steel_layer::part_shape(copy)?,
            placement: LayerPlacement {
                translation_mm: placement.translation_mm,
                orientation_xyzw: [qx, qy, qz, qw],
            },
            material: copy.material(),
        }),
        part_mesh_change(copy)?,
    ])
}

/// Copies one element and publishes the next revision, or replays the operation.
///
/// Steps, each failing closed: admit the request; open and verify the head; find the source;
/// build the copy's authored state (the body is the source's); decide replay from the host
/// ledger; refuse a copy identity that is already in use; author the new element with U02; check
/// the layer engine's writes against the declared footprint; commit through U06/U07 with the
/// strict verifier.
pub fn copy_element(
    store: &RevisionStore,
    request: &CopyRequest,
    hook: &mut dyn CommitHook,
) -> Result<Copied, Refusal> {
    copy_with_mode(store, request, hook, false)
}

/// Copies a part as an instance of its canonical shared definition (P11).
pub fn copy_instance(
    store: &RevisionStore,
    request: &CopyRequest,
    hook: &mut dyn CommitHook,
) -> Result<Copied, Refusal> {
    copy_with_mode(store, request, hook, true)
}

fn copy_with_mode(
    store: &RevisionStore,
    request: &CopyRequest,
    hook: &mut dyn CommitHook,
    instance: bool,
) -> Result<Copied, Refusal> {
    let command = &request.command;
    command.validate_arguments()?;
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
    let (plates, members, definitions, levels, instances) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, members) = evaluate_decoded(&decoded)?;
        if decoded
            .plates
            .iter()
            .any(|(p, _)| p.id == command.source_id)
            && !plates.iter().any(|p| p.authored.id == command.source_id)
        {
            return Err(CommandError::new(
                ErrorCode::DependencyUnavailable,
                "source_id",
                "copying an unsupported plate is not implemented",
            )
            .into());
        }
        (
            plates,
            members,
            decoded.definitions,
            decoded.levels,
            decoded.instances,
        )
    };
    // The copy checks that the pinned definitions still resolve, as every create does.
    let materials = material_lookup(&definitions);

    let canonical = match instances.definition(command.source_id) {
        Ok(id) => id,
        Err(e) if e.code == ErrorCode::NotFound => command.source_id,
        Err(e) => return Err(e.into()),
    };
    let copy = if let Some(source) = PartRecord::find(&plates, &members, command.source_id) {
        source.copied(command, instance)?
    } else if crate::weld::is_weld(&base_request, &command.source_id, &control)? {
        // A weld always has element links (its `connects` record), so a single-element copy
        // would drop them (DV-REL-011).
        return Err(CommandError::new(
            ErrorCode::DependencyConflict,
            "source_id",
            "a weld has element links that a copy does not carry over",
        )
        .with_rule("DV-REL-011")
        .into());
    } else if crate::bolt::is_bolt(&base_request, &command.source_id, &control)? {
        // Same for a bolt group: its `connects` record is not carried over by a copy.
        return Err(CommandError::new(
            ErrorCode::DependencyConflict,
            "source_id",
            "a bolt group has element links that a copy does not carry over",
        )
        .with_rule("DV-REL-011")
        .into());
    } else if crate::level::is_level(&base_request, &command.source_id, &control)? {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "source_id",
            "a level is not copied",
        )
        .into());
    } else if crate::coordination::pinned_owns_element(store, &command.source_id)? {
        // A pinned coordination grid is foreign bytes: the refusal precedes the local miss
        // (PC-OWN-002), exactly as for the dedicated grid commands.
        return Err(Refusal {
            rule_id: Some("PC-OWN-002"),
            code: ErrorCode::ForeignReadOnly,
            message: "the source belongs to a pinned coordination project and is read-only",
        });
    } else {
        return Err(not_found());
    };
    let (profile, material) = (copy.profile(), copy.material());
    if let Some(digest) = &material {
        materials(digest)?;
    }
    let head_copy = PartRecord::find(&plates, &members, command.new_id);

    let op_host = host_uuid(&request.op_id.to_string())?;
    let fingerprint = if instance {
        steel::model::instance::CopyInstance {
            copy: command.clone(),
        }
        .content_fingerprint()?
    } else {
        command.content_fingerprint()?
    };
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        // The head already holds this copy exactly when the retry is the same request.
        match &head_copy {
            Some(held)
                if held.same_state(&copy)
                    && instances.is_instance(command.new_id) == instance
                    && (!instance || instances.definition(command.new_id)? == canonical) =>
            {
                Ok(fingerprint)
            }
            _ => digest_of(b"no accepted request matches the head state"),
        }
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            return Ok(Copied {
                replayed: true,
                element: head_copy.ok_or_else(|| {
                    integrity("replayed element is absent from the head revision")
                })?,
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
    if instance && instances.links().count() >= steel::model::instance::MAX_PART_INSTANCES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "instances",
            "too many part instances",
        )
        .into());
    }
    if head_copy.is_some() {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "new_id",
            "the copy identity is already in use",
        )
        .into());
    }

    // The copy stays on the source's level (D2).
    let level = levels.level_of(&command.source_id);
    let mut footprint = Footprint::for_copy(command.source_id, command.new_id, profile, material)?;
    if canonical != command.source_id {
        footprint = footprint.union(Footprint::for_copy(
            canonical,
            command.new_id,
            profile,
            material,
        )?)?;
    }
    if let Some(level) = level {
        footprint = footprint.union(Footprint::for_level(command.new_id, Some(level))?)?;
    }
    let schema = schema_identity()?;
    let (mut author, opened) =
        crate::scope::open_author(store, base_request, false, schema, &control)?;
    // Links the adapter does not carry over must not be dropped silently: the layer engine plans
    // the copy, dropping assembly membership and connections as required by DV-REL-011.
    // Other unsupported links refuse the operation.
    let index = author.element_index(&opened.context, &control)?;
    let plan = index.copy_plan(
        &LayerElementId::parse(&command.source_id.to_string())?,
        &LayerElementId::parse(&command.new_id.to_string())?,
    )?;
    let is_level_link = |l: &beamcraft_layer_engine::index::PlannedLink| {
        l.name == "bim:level" && matches!(l.target, LinkTarget::Element(_))
    };
    if plan
        .links
        .iter()
        .any(|l| !matches!(l.target, LinkTarget::Definition(_)) && !is_level_link(l))
    {
        return Err(CommandError::new(
            ErrorCode::DependencyConflict,
            "source_id",
            "the source has element links that a copy does not carry over yet",
        )
        .with_rule("DV-REL-011")
        .into());
    }
    let planned_level = plan
        .links
        .iter()
        .find(|l| is_level_link(l))
        .map(|l| match &l.target {
            LinkTarget::Element(t) => t.to_string(),
            _ => String::new(),
        });
    if planned_level != level.map(|l| l.to_string()) {
        return Err(integrity("the level of the source differs between readers"));
    }
    let mut changes = creation_changes(&copy)?;
    if instance {
        for change in &mut changes {
            if let Change::Create(element) = change {
                element.shape = steel_layer::instance(copy.instance_kind(), canonical.to_string());
                element.material = None;
            }
        }
    }
    if let Some(level) = level {
        changes.push(Change::Level {
            id: command.new_id.to_string(),
            level: Some(level.to_string()),
        });
    }
    let (committed, changed_layers) = prepare_and_publish(
        &mut author,
        &opened,
        changes,
        &footprint,
        &before,
        PublicationTail {
            store,
            control: &control,
            schema,
            fingerprint,
            project: &project,
            workspace: &workspace,
            revision_id: &request.revision_id,
            op: op_host,
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    Ok(Copied {
        replayed: false,
        element: copy,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
    })
}
