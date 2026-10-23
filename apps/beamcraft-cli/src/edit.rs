// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P04 adapter: `UpdateElement` for a plate or a member, committed as the next revision.
//!
//! The patch is typed (label, material reference, a plate's thickness, the placement translation,
//! the D2 level assignment)
//! and the adapter writes only what the patch changes: a rename touches the authored layer alone,
//! a material change rewrites the quantity record next to an unchanged mesh, a thickness change
//! rebuilds the mesh and the records, and a move (P08) rewrites the placement alone: the
//! element-local body, its mesh and its stored quantities are not copied or rebuilt. The engine
//! decides which groups changed and the layer engine's own write set is checked against the
//! footprint it declares.
//!
//! Honesty limits: replay of an `op_id` is decided from the head model (the update is a replay
//! when the head already holds the patched state), not from a stored canonical request; a later
//! edit of the same element therefore turns a retry into `OP_ID_REUSE`. A stored canonical
//! request (F04 section 3) replaces this. One implicit workspace, in-process layer engine, as in
//! `plate`.

use crate::definition::material_lookup;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, PublishRequest, RevisionSummary,
    check_footprint, control, decode, digest_of, evaluate_decoded, host_uuid, integrity, load_base,
    member_record, mesh_change, open_base, plate_record, prior_op_with, publish_revision,
    replay_summary, summarize,
};
use beamcraft_engine::command::evaluate_plate;
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::member::member_quantities;
use beamcraft_engine::query::committed::{MemberRecord, PlateRecord};
use beamcraft_engine::transaction::PriorOp;
use beamcraft_engine::update::{UpdateEffect, effect, patch_member, patch_plate};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, MaterialOpinion, Placement as LayerPlacement, Prepare,
    schema_identity,
};
use beamcraft_model::command::{CommandError, UpdateElement};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

/// Inputs of one `update_element` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct UpdateRequest {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; an update of such a store is `NOT_FOUND`. A
    /// store with a revision ignores it (see `plate::ensure_project`).
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: UpdateElement,
}

/// The element as it is after the update.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdatedElement {
    /// A plate.
    Plate(Box<PlateRecord>),
    /// A beam or a column.
    Member(Box<MemberRecord>),
}

/// Result of `update_element`.
#[derive(Debug, Clone, PartialEq)]
pub struct Updated {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The element after the update (the head's element for a replay).
    pub element: UpdatedElement,
    /// The groups the patch changes.
    pub effect: UpdateEffect,
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

/// Updates one element and publishes the next revision, or replays the operation.
///
/// Steps, each failing closed: admit the patch; open and verify the head; find the target; apply
/// the patch and re-evaluate what it changes (the pinned material resolved from the head's
/// definitions); decide replay from the host ledger; author only the changed groups with U02;
/// check the layer engine's writes against the declared footprint; commit through U06/U07 with
/// the strict verifier (the stored quantity records must match the kernel's).
pub fn update_element(
    store: &RevisionStore,
    request: &UpdateRequest,
    hook: &mut dyn CommitHook,
) -> Result<Updated, PlateError> {
    let command = &request.command;
    command.validate_arguments()?;
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    if head.is_none() {
        return Err(not_found());
    }
    let before: BTreeMap<String, ContentDigest> = base
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
    let (plates, members, definitions, levels) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, members) = evaluate_decoded(&decoded)?;
        (plates, members, decoded.definitions, decoded.levels)
    };
    let materials = material_lookup(&definitions);
    let patch_effect = effect(&command.patch);
    let new_material = command.patch.material.flatten();

    // The element after the patch, its body and its quantities.
    enum Target<'a> {
        Plate(&'a PlateRecord),
        Member(&'a MemberRecord),
    }
    let target = plates
        .iter()
        .find(|r| r.authored.id == command.target_id)
        .map(Target::Plate)
        .or_else(|| {
            members
                .iter()
                .find(|r| r.authored.id == command.target_id)
                .map(Target::Member)
        })
        .ok_or_else(|| match command.patch.level {
            // A known element that cannot hold a level is an unsupported assignment, not a
            // missing element.
            Some(level) => levels
                .assign(command.target_id, level)
                .err()
                .map_or_else(not_found, PlateError::from),
            None => not_found(),
        })?;
    let (head_element, next) = match target {
        Target::Plate(head) => {
            let authored = patch_plate(&head.authored, &command.patch)?;
            let material = authored.material.as_ref().map(&materials).transpose()?;
            let body = if patch_effect.shape {
                evaluate_plate(&authored)?
            } else {
                head.body.clone()
            };
            let quantities = body.quantities_for(material)?;
            (
                UpdatedElement::Plate(Box::new(head.clone())),
                UpdatedElement::Plate(Box::new(PlateRecord {
                    authored,
                    body,
                    quantities,
                })),
            )
        }
        Target::Member(head) => {
            let authored = patch_member(&head.authored, &command.patch)?;
            let material = authored.material.as_ref().map(&materials).transpose()?;
            let quantities = member_quantities(&head.body, material)?;
            (
                UpdatedElement::Member(Box::new(head.clone())),
                UpdatedElement::Member(Box::new(MemberRecord {
                    authored,
                    body: head.body.clone(),
                    quantities,
                })),
            )
        }
    };
    let head_level = levels.level_of(&command.target_id);
    let next_level = command.patch.level.unwrap_or(head_level);
    let same_state = |a: &UpdatedElement, b: &UpdatedElement| match (a, b) {
        (UpdatedElement::Plate(a), UpdatedElement::Plate(b)) => a.authored == b.authored,
        (UpdatedElement::Member(a), UpdatedElement::Member(b)) => a.authored == b.authored,
        _ => false,
    } && head_level == next_level;

    let op_host = host_uuid(&request.op_id.to_string())?;
    let fingerprint = command.content_fingerprint()?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        // The head already holds the patched state exactly when the retry is the same request.
        if same_state(&head_element, &next) {
            Ok(fingerprint)
        } else {
            digest_of(b"no accepted request matches the head state")
        }
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            return Ok(Updated {
                replayed: true,
                element: head_element,
                effect: patch_effect,
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
    if same_state(&head_element, &next) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "changes",
            "the patch changes nothing",
        )
        .into());
    }

    let mut footprint = Footprint::for_update(
        command.target_id,
        patch_effect.name,
        patch_effect.material,
        patch_effect.shape,
        patch_effect.placement,
        new_material,
    )?;
    // The level decision needs the model (kind of the target, existence and kind of the level);
    // it runs after replay and no-op detection, so a retry never fails on its own result.
    if let Some(level) = command.patch.level {
        footprint = footprint.union(levels.assign(command.target_id, level)?.footprint)?;
    }
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
    let mut changes = Vec::new();
    if let Some(name) = &command.patch.name {
        changes.push(Change::Label {
            id: id_text.clone(),
            name: name.clone(),
        });
    }
    if let Some(material) = command.patch.material {
        changes.push(Change::Material {
            id: id_text.clone(),
            opinion: material.map_or(MaterialOpinion::Empty, MaterialOpinion::Target),
        });
    }
    if let Some(mm) = command.patch.thickness_mm {
        changes.push(Change::Thickness {
            id: id_text.clone(),
            mm,
        });
    }
    if patch_effect.placement {
        let placement = match &next {
            UpdatedElement::Plate(r) => r.authored.placement,
            UpdatedElement::Member(r) => r.authored.placement,
        };
        let [qx, qy, qz, qw] = placement.orientation_xyzw;
        changes.push(Change::Placement {
            id: id_text.clone(),
            placement: LayerPlacement {
                translation_mm: placement.translation_mm,
                orientation_xyzw: [qx, qy, qz, qw],
            },
        });
    }
    if let Some(level) = command.patch.level {
        changes.push(Change::Level {
            id: id_text,
            level: level.map(|l| l.to_string()),
        });
    }
    if patch_effect.material || patch_effect.shape {
        changes.push(match &next {
            UpdatedElement::Plate(r) => {
                mesh_change(&r.authored.id, &r.body.solid, &plate_record(r))?
            }
            UpdatedElement::Member(r) => {
                mesh_change(&r.authored.id, &r.body.solid, &member_record(r))?
            }
        });
    }
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes,
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
    Ok(Updated {
        replayed: false,
        element: next,
        effect: patch_effect,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
    })
}
