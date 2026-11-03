// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P04 adapter: `UpdateElement` for a plate or a member, committed as the next revision.
//!
//! The patch is typed (label, material reference, a plate's thickness, a plate's thickness side,
//! the placement translation, the placement rotation, the D2 level assignment)
//! and the adapter writes only what the patch changes: a rename touches the authored layer alone,
//! a material change rewrites the quantity record next to an unchanged mesh, a thickness or
//! thickness-side change rebuilds the mesh and the records, a move (P08) rewrites the placement
//! alone: the element-local body, its mesh and its stored quantities are not copied or rebuilt. An
//! orientation change rewrites the placement alone and keeps the local body. The engine
//! decides which groups changed; the layer engine's write set is checked against its footprint.
//!
//! Honesty limits: replay of an `op_id` is decided from the head model (the update is a replay
//! when the head already holds the patched state), not from a stored canonical request; a later
//! edit of the same element therefore turns a retry into `OP_ID_REUSE`. A stored canonical
//! request (F04 section 3) replaces this. One implicit workspace, in-process layer engine, as in
//! `plate`.

use crate::definition::{material_lookup, profile_lookup};
use crate::plate::{decode, evaluate_decoded, part_mesh_change, prior_op_with, replay_summary};
use crate::publication::{PublicationTail, prepare_and_publish};
use beamcraft_engine::footprint::Footprint;
use beamcraft_host::revision::{CommitHook, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{
    Change, MaterialOpinion, Opinion, Placement as LayerPlacement, schema_identity,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};
use session::Refusal;
use session::pipeline::{
    Base, RevisionSummary, control, digest_of, host_uuid, integrity, load_base, open_base,
    summarize,
};
use std::collections::BTreeMap;
use steel::engine::committed::PlateRecord;
pub use steel::engine::part::PartRecord;
use steel::engine::plate_draft::{PlateDraft, UnsupportedPlate, create_plate_draft};
use steel::engine::transaction::PriorOp;
use steel::engine::update::{UpdateEffect, effect};
use steel::layer as steel_layer;
use steel::model::command::UpdateElement;

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
    /// store with a revision ignores it (see `session::pipeline::ensure_project`).
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: UpdateElement,
}

/// The element an update reports: with its evaluated body when the kernel evaluates it, or as
/// authored-only data (unsupported or degenerate, P-Degenerate) when it does not.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdatedPart {
    /// The element with its evaluated body and quantities.
    Evaluated(PartRecord),
    /// The retained authored-only state; no derived data exist for the element.
    AuthoredOnly(Box<UnsupportedPlate>),
}

/// Result of `update_element`.
#[derive(Debug, Clone, PartialEq)]
pub struct Updated {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The element after the update (the head's element for a replay).
    pub element: UpdatedPart,
    /// The groups the patch changes.
    pub effect: UpdateEffect,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
}

fn not_found() -> Refusal {
    Refusal::from(CommandError::new(
        ErrorCode::NotFound,
        "target_id",
        "the element is not in the head revision",
    ))
}

/// The retained authored-only state of a plate the caller resolved as non-evaluated. An element
/// that evaluates here contradicts that resolution and is an integrity error.
fn retained_state(
    authored: &steel::engine::command::AuthoredPlate,
    materials: steel::engine::command::MaterialLookup<'_>,
) -> Result<UnsupportedPlate, Refusal> {
    match create_plate_draft(&authored.to_command(), &|_| false, materials)? {
        PlateDraft::Unsupported(record) => Ok(*record),
        PlateDraft::Evaluated(_) => Err(integrity("an authored-only element evaluates")),
    }
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
) -> Result<Updated, Refusal> {
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
    let (plates, members, definitions, levels, instances, authored_head) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, members) = evaluate_decoded(&decoded)?;
        // The plate the kernel cannot evaluate (unsupported or degenerate, P-Degenerate): the
        // patch applies to its authored record and the result is reclassified below, so an
        // edit repairs the element instead of refusing it.
        let authored_head = decoded
            .plates
            .iter()
            .find(|(p, _)| p.id == command.target_id)
            .filter(|_| !plates.iter().any(|p| p.authored.id == command.target_id))
            .map(|(p, _)| p.clone());
        (
            plates,
            members,
            decoded.definitions,
            decoded.levels,
            decoded.instances,
            authored_head,
        )
    };
    let materials = material_lookup(&definitions);
    let patch_effect = effect(&command.patch);
    let new_material = command.patch.material.flatten();

    // The P-Degenerate update (A-2 D-7): an edit of a plate whose body the kernel cannot
    // evaluate. The patch rewrites the authored record; the reclassified result either repairs
    // the element (its body and quantities are computed and stored) or keeps it authored-only.
    // A hard refusal of the patched state still refuses the whole command, so nothing is
    // dropped or half-written.
    if let Some(authored) = authored_head {
        let head_level = levels.level_of(&command.target_id);
        // Shared-definition edits need the evaluated bodies of the group; the kernel evaluates
        // none of them here, so a group larger than the element itself refuses. Copies come
        // from evaluated records only, so a group is expected to hold the element alone.
        let shared = patch_effect.material || patch_effect.shape;
        if shared && instances.group(command.target_id)?.len() > 1 {
            return Err(CommandError::new(
                ErrorCode::DependencyUnavailable,
                "target_id",
                "instances cannot share an element without evaluated geometry",
            )
            .into());
        }
        let next = steel::engine::update::patch_plate(&authored, &command.patch)?;
        let unchanged = next == authored && head_level == command.patch.level.unwrap_or(head_level);
        let op_host = host_uuid(&request.op_id.to_string())?;
        let fingerprint = command.content_fingerprint()?;
        let head_revision = head
            .as_ref()
            .map(|revision| revision.info().revision().as_str().to_owned());
        // Replay follows the documented UpdateElement contract; a relative transform (a mirror
        // here) replays from history as in the evaluated path below.
        let relative = patch_effect.rotate || patch_effect.mirror || patch_effect.move_mm;
        let (prior, recorded) = prior_op_with(store, &op_host, || {
            if unchanged {
                Ok(fingerprint)
            } else {
                digest_of(b"no accepted request matches the head state")
            }
        })?;
        let replayed = match prior {
            PriorOp::Accepted { fingerprint: seen } => {
                seen == fingerprint
                    || (relative
                        && recorded.as_ref().is_some_and(|(revision, ..)| {
                            head_revision.as_deref() == Some(revision.as_str())
                        }))
            }
            PriorOp::Unseen => false,
        };
        if replayed {
            // The head's own retained state, reclassified from its authored inputs.
            let retained = retained_state(&authored, &materials)?;
            return Ok(Updated {
                replayed: true,
                element: UpdatedPart::AuthoredOnly(Box::new(retained)),
                effect: patch_effect,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
            });
        }
        if let PriorOp::Accepted { .. } = prior {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        if unchanged {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "changes",
                "the patch changes nothing",
            )
            .into());
        }
        let element = match create_plate_draft(&next.to_command(), &|_| false, &materials)? {
            PlateDraft::Evaluated(outcome) => {
                UpdatedPart::Evaluated(PartRecord::Plate(Box::new(PlateRecord {
                    authored: outcome.authored,
                    body: outcome.body,
                    quantities: outcome.quantities,
                })))
            }
            PlateDraft::Unsupported(record) => UpdatedPart::AuthoredOnly(Box::new(*record)),
        };
        let canonical = instances.definition(command.target_id)?;
        let id_text = command.target_id.to_string();
        let definition_text = canonical.to_string();
        let mut changes = Vec::new();
        if let Some(class) = command.patch.class {
            changes.push(Change::PartClass {
                id: id_text.clone(),
                class,
            });
        }
        if let Some(name) = &command.patch.name {
            changes.push(Change::Label {
                id: id_text.clone(),
                name: name.clone(),
            });
        }
        if let Some(material) = command.patch.material {
            changes.push(Change::Material {
                id: definition_text.clone(),
                opinion: material.map_or(MaterialOpinion::Empty, MaterialOpinion::Target),
            });
        }
        if let Some(mm) = command.patch.plate.thickness_mm {
            changes.push(steel_layer::thickness(definition_text.clone(), mm));
        }
        if let Some(points) = &command.patch.plate.contour_points_mm {
            changes.push(steel_layer::contour(
                definition_text.clone(),
                points.clone(),
            ));
        }
        if let Some(corners) = &command.patch.plate.corners {
            changes.push(steel_layer::corners(
                definition_text.clone(),
                corners.clone(),
            ));
        }
        if let Some(side) = command.patch.plate.thickness_side {
            changes.push(steel_layer::thickness_side(definition_text.clone(), side));
        }
        // The mirror re-encodes the authored records of the degenerate element, exactly as it
        // re-encodes an evaluated one; the reclassification above decided the derived writes.
        if patch_effect.mirror {
            if let Some(side) = next.thickness_side {
                changes.push(steel_layer::thickness_side(definition_text.clone(), side));
            }
            changes.push(steel_layer::cuts(
                definition_text.clone(),
                next.cuts.clone(),
            ));
        }
        if patch_effect.placement || patch_effect.orientation {
            let placement = &next.placement;
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
        if let UpdatedPart::Evaluated(record @ PartRecord::Plate(_)) = &element {
            changes.push(part_mesh_change(record)?);
        }
        let mut footprint = Footprint::for_update(
            command.target_id,
            patch_effect.name || patch_effect.class,
            patch_effect.material,
            patch_effect.shape,
            patch_effect.placement || patch_effect.orientation,
            new_material,
        )?;
        if let Some(level) = command.patch.level {
            footprint = footprint.union(levels.assign(command.target_id, level)?.footprint)?;
        }
        let schema = schema_identity()?;
        let (mut author, opened) =
            crate::scope::open_author(store, base_request, false, schema, &control)?;
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
        return Ok(Updated {
            replayed: false,
            element,
            effect: patch_effect,
            revision: summarize(&committed.info),
            footprint: Some(footprint),
            changed_layers,
        });
    }

    // The element after the patch, its body and its quantities.
    let head_element = PartRecord::find(&plates, &members, command.target_id).ok_or_else(|| {
        match command.patch.level {
            // A known element that cannot hold a level is an unsupported assignment, not a
            // missing element.
            Some(level) => levels
                .assign(command.target_id, level)
                .err()
                .map_or_else(not_found, Refusal::from),
            None => not_found(),
        }
    })?;
    let next = head_element.patched(
        &command.patch,
        patch_effect.shape,
        &materials,
        profile_lookup(&definitions),
    )?;
    let head_level = levels.level_of(&command.target_id);
    let next_level = command.patch.level.unwrap_or(head_level);
    let same_state = |a: &PartRecord, b: &PartRecord| a.same_state(b) && head_level == next_level;

    let op_host = host_uuid(&request.op_id.to_string())?;
    let fingerprint = command.content_fingerprint()?;
    // A relative transform (T03) cannot recompute its would-be result from the head it already
    // turned or mirrored, so the state comparison below never holds for one. Its replay is
    // decided from history instead: the head must still be the revision the operation itself
    // committed; any later edit makes the retry `OP_ID_REUSE`.
    let relative = patch_effect.rotate || patch_effect.mirror || patch_effect.move_mm;
    let head_revision = head
        .as_ref()
        .map(|revision| revision.info().revision().as_str().to_owned());
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        // The head already holds the patched state exactly when the retry is the same request.
        if same_state(&head_element, &next) {
            Ok(fingerprint)
        } else {
            digest_of(b"no accepted request matches the head state")
        }
    })?;
    let replayed = match prior {
        PriorOp::Accepted { fingerprint: seen } => {
            seen == fingerprint
                || (relative
                    && recorded.as_ref().is_some_and(|(revision, ..)| {
                        head_revision.as_deref() == Some(revision.as_str())
                    }))
        }
        PriorOp::Unseen => false,
    };
    if replayed {
        return Ok(Updated {
            replayed: true,
            element: UpdatedPart::Evaluated(head_element),
            effect: patch_effect,
            revision: replay_summary(&head, recorded)?,
            footprint: None,
            changed_layers: Vec::new(),
        });
    }
    if let PriorOp::Accepted { .. } = prior {
        return Err(CommandError::new(
            ErrorCode::OpIdReuse,
            "op_id",
            "operation id was accepted with different content",
        )
        .into());
    }
    if same_state(&head_element, &next) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "changes",
            "the patch changes nothing",
        )
        .into());
    }

    let canonical = instances.definition(command.target_id)?;
    let shared = patch_effect.material || patch_effect.shape;
    let mut shared_records = Vec::new();
    if shared {
        let patch = steel::model::command::ElementPatch {
            material: command.patch.material,
            plate: command.patch.plate.clone(),
            member: command.patch.member.clone(),
            ..Default::default()
        };
        for id in instances.group(command.target_id)? {
            if id == command.target_id {
                continue;
            }
            let head = PartRecord::find(&plates, &members, id)
                .ok_or_else(|| integrity(next.unevaluated_instance()))?;
            let record = next
                .shared_with(&head, &patch)?
                .ok_or_else(|| integrity(next.unevaluated_instance()))?;
            shared_records.push(record);
        }
    }
    let mut footprint = Footprint::for_update(
        command.target_id,
        patch_effect.name || patch_effect.class,
        patch_effect.material,
        patch_effect.shape,
        patch_effect.placement || patch_effect.orientation,
        new_material,
    )?;
    if let Some(profile) = command.patch.pinned_profile() {
        footprint = footprint.union(Footprint::for_profile_change(command.target_id, profile)?)?;
    }
    if shared {
        footprint = footprint.union(instances.dependencies(command.target_id)?)?;
        for id in instances.group(command.target_id)? {
            if id != command.target_id {
                footprint = footprint.union(Footprint::for_update(
                    id,
                    false,
                    patch_effect.material,
                    patch_effect.shape,
                    false,
                    new_material,
                )?)?;
            }
        }
    }
    // The level decision needs the model (kind of the target, existence and kind of the level);
    // it runs after replay and no-op detection, so a retry never fails on its own result.
    if let Some(level) = command.patch.level {
        footprint = footprint.union(levels.assign(command.target_id, level)?.footprint)?;
    }
    let schema = schema_identity()?;
    let (mut author, opened) =
        crate::scope::open_author(store, base_request, false, schema, &control)?;
    let id_text = command.target_id.to_string();
    let definition_text = canonical.to_string();
    let mut changes = Vec::new();
    if let Some(class) = command.patch.class {
        changes.push(Change::PartClass {
            id: id_text.clone(),
            class,
        });
    }
    if let Some(name) = &command.patch.name {
        changes.push(Change::Label {
            id: id_text.clone(),
            name: name.clone(),
        });
    }
    if let Some(material) = command.patch.material {
        changes.push(Change::Material {
            id: definition_text.clone(),
            opinion: material.map_or(MaterialOpinion::Empty, MaterialOpinion::Target),
        });
    }
    if let Some(mm) = command.patch.plate.thickness_mm {
        changes.push(steel_layer::thickness(definition_text.clone(), mm));
    }
    // The contour is a shape input of the canonical definition, like the thickness: the
    // instances of the group read their bodies from it and take mesh updates below.
    if let Some(points) = &command.patch.plate.contour_points_mm {
        changes.push(steel_layer::contour(
            definition_text.clone(),
            points.clone(),
        ));
    }
    if let Some(corners) = &command.patch.plate.corners {
        changes.push(steel_layer::corners(
            definition_text.clone(),
            corners.clone(),
        ));
    }
    if let Some(side) = command.patch.plate.thickness_side {
        changes.push(steel_layer::thickness_side(definition_text.clone(), side));
    }
    // A member path change (P12) is authored on the canonical part, like a thickness change:
    // the instances of the group read their shape inputs from it.
    if let (Some(axis), true) = (next.axis_points_mm(), patch_effect.shape) {
        changes.push(steel_layer::axis(definition_text.clone(), axis.to_vec()));
        // A bent member carries no authored roll: the polybeam sweep has no roll input, so
        // the old straight member's opinion is removed instead of being silently ignored.
        if axis.len() > 2 {
            changes.push(steel_layer::roll(definition_text.clone(), Opinion::Remove));
        }
    }
    if let Some(digest) = command.patch.member.profile {
        changes.push(steel_layer::profile(definition_text.clone(), digest));
    }
    // A mirror (T03) re-encodes the local body: the plate's side flips and the cuts of either
    // part kind reflect, both authored on the canonical part.
    if patch_effect.mirror {
        if let Some(side) = next.thickness_side() {
            changes.push(steel_layer::thickness_side(definition_text.clone(), side));
        }
        changes.push(steel_layer::cuts(
            definition_text.clone(),
            next.cuts().to_vec(),
        ));
    }
    if patch_effect.placement || patch_effect.orientation {
        let placement = next.placement();
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
        changes.push(part_mesh_change(&next)?);
    }
    for record in &shared_records {
        changes.push(part_mesh_change(record)?);
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
    Ok(Updated {
        replayed: false,
        element: UpdatedPart::Evaluated(next),
        effect: patch_effect,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
    })
}
