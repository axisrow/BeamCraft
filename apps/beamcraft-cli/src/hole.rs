// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A04/A04.2 adapter: `AddHole`, `UpdateHole` and `RemoveHole` on a plate or member, committed
//! as one revision.
//!
//! The command appends one operation record (F04 section 4, the F06 member-hole amendment) to
//! the target's authored state; the engine replays the base shape with every record in ordinal
//! order and re-certifies the body. The adapter writes the complete record set with U02,
//! rewrites the derived mesh, checks the layer engine's writes against the declared footprint
//! and commits through U06/U07 with the strict verifier. Replay of an `operation_id` is decided
//! from immutable committed evidence. A later record edit or deletion cannot turn an accepted
//! request into a second mutation. The explicit source revision is returned on replay. One
//! implicit workspace, in-process layer engine, as in `plate`. Plate records (A04) and member
//! records (A04.2) live on their own elements, so `update-hole`/`remove-hole` never conflate
//! them: the record identity is checked across the whole project, and the kind guards of the
//! engine keep the polygon cuts (A13) out of both.

use crate::definition::{material_lookup, profile_lookup};
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    decode, evaluate_decoded, host_uuid, integrity, load_base, member_record, mesh_change,
    open_base, plate_record, prior_op_with, replay_summary, summarize,
};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::member::{evaluate_member, member_quantities};
use beamcraft_engine::query::committed::{MemberRecord, PlateRecord};
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{Author, Change, EditTargets, Prepare, schema_identity};
use beamcraft_model::command::{AddHole, CommandError};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

/// Inputs of one `add_hole` call beyond the command itself. The `op_id` is the record's
/// `operation_id` for creation; an update uses a separate logical operation identity.
#[derive(Debug, Clone)]
pub struct AddHoleRequest {
    /// The logical operation and the record identity.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; an add-hole of such a store is `NOT_FOUND`.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: AddHole,
}

/// The hole target as it is after the command.
#[derive(Debug, Clone, PartialEq)]
pub enum HoleElement {
    /// A plate with its hole records (A04).
    Plate(Box<PlateRecord>),
    /// A beam or a column with its member-hole records (A04.2).
    Member(Box<MemberRecord>),
}

/// Result of `add_hole`.
#[derive(Debug, Clone, PartialEq)]
pub struct AddedHole {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The target from the accepted revision, including on replay.
    pub element: HoleElement,
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

/// Adds one through-hole record to a plate or member and publishes the next revision, or
/// replays the operation.
///
/// Steps, each failing closed: admit the arguments; open and verify the head; find the target
/// (an authored-only unsupported plate is `DEPENDENCY_UNAVAILABLE`); decide replay from the
/// host ledger and the head's records; check the record id across every element of the project;
/// replay the records with the kernel and re-certify; author the records and the mesh with U02;
/// check the layer engine's writes against the declared footprint; commit through U06/U07 with
/// the strict verifier (the stored quantity records must match the kernel's). A refusal writes
/// nothing.
pub fn add_hole_command(
    store: &RevisionStore,
    request: &AddHoleRequest,
    hook: &mut dyn CommitHook,
) -> Result<AddedHole, PlateError> {
    apply_hole(store, request, hook, false, false)
}
/// Replace one hole record in a canonical instance group, in one atomic revision.
/// `request.op_id` is a new logical request identity; command.operation_id names the record.
pub fn update_hole_command(
    store: &RevisionStore,
    request: &AddHoleRequest,
    hook: &mut dyn CommitHook,
) -> Result<AddedHole, PlateError> {
    apply_hole(store, request, hook, true, false)
}
/// Removal context with an independent logical request ID.
#[derive(Debug, Clone)]
pub struct RemoveHoleRequest {
    /// Logical request UUID.
    pub op_id: OpId,
    /// New revision UUID.
    pub revision_id: RevisionId,
    /// Trusted actor.
    pub actor: Uuid7,
    /// Store project.
    pub project: ProjectId,
    /// Store workspace.
    pub workspace: WorkspaceId,
    /// Target and record UUID.
    pub command: beamcraft_model::command::RemoveHole,
}
/// Same immutable response shape as other hole record changes; no live ordinal is invented.
pub type RemovedHole = AddedHole;
/// Remove and retire a record in one canonical-instance-group revision and undo unit.
pub fn remove_hole_command(
    store: &RevisionStore,
    request: &RemoveHoleRequest,
    hook: &mut dyn CommitHook,
) -> Result<RemovedHole, PlateError> {
    let request = AddHoleRequest {
        op_id: request.op_id,
        revision_id: request.revision_id,
        actor: request.actor.clone(),
        project: request.project,
        workspace: request.workspace,
        command: AddHole {
            target_id: request.command.target_id,
            operation_id: request.command.operation_id,
            hole: beamcraft_model::command::HoleSpec::Rectangle {
                corner_mm: [0.0; 2],
                width_mm: 1.0,
                height_mm: 1.0,
            },
        },
    };
    apply_hole(store, &request, hook, false, true)
}
fn apply_hole(
    store: &RevisionStore,
    request: &AddHoleRequest,
    hook: &mut dyn CommitHook,
    update: bool,
    remove: bool,
) -> Result<AddedHole, PlateError> {
    let command = &request.command;
    command.validate_arguments()?;
    let _ = command.polygon()?;
    let edit = beamcraft_model::command::UpdateHole {
        hole: command.clone(),
    };
    let removal = beamcraft_model::command::RemoveHole {
        target_id: command.target_id,
        operation_id: command.operation_id,
    };
    let fingerprint = if remove {
        removal.content_fingerprint()?
    } else if update {
        edit.content_fingerprint()?
    } else {
        command.content_fingerprint()?
    };
    let op = host_uuid(&request.op_id.to_string())?;
    if let Some((captured, revision)) = crate::command_evidence::accepted(store, &op)? {
        if captured.fingerprint != fingerprint {
            return Err(PlateError::new(
                ErrorCode::OpIdReuse,
                "operation identity was accepted with different arguments",
            ));
        }
        let inspected = crate::plate::inspect_revision(&revision)?;
        let element = if let Some(p) = inspected
            .plates
            .iter()
            .find(|p| p.authored.id == command.target_id)
        {
            HoleElement::Plate(Box::new(p.clone()))
        } else if let Some(m) = inspected
            .members
            .iter()
            .find(|m| m.authored.id == command.target_id)
        {
            HoleElement::Member(Box::new(m.clone()))
        } else {
            return Err(integrity("accepted hole target missing from its revision"));
        };
        return Ok(AddedHole {
            replayed: true,
            element,
            revision: inspected.revision,
            footprint: None,
            changed_layers: Vec::new(),
        });
    }
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    if head.is_none() {
        return Err(not_found());
    }
    let before: BTreeMap<String, beamcraft_model::digest::ContentDigest> = base
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
    let (plates, members, definitions, instances, history, hole_history, member_hole_history) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, members) = evaluate_decoded(&decoded)?;
        if decoded
            .plates
            .iter()
            .any(|(p, _)| p.id == command.target_id)
            && !plates.iter().any(|p| p.authored.id == command.target_id)
            && members.iter().all(|m| m.authored.id != command.target_id)
        {
            return Err(CommandError::new(
                ErrorCode::DependencyUnavailable,
                "target_id",
                "editing an unsupported plate is not implemented",
            )
            .into());
        }
        (
            plates,
            members,
            decoded.definitions,
            decoded.instances,
            decoded.cut_history,
            decoded.hole_history,
            decoded.member_hole_history,
        )
    };
    let head_element =
        if let Some(record) = plates.iter().find(|r| r.authored.id == command.target_id) {
            HoleElement::Plate(Box::new(record.clone()))
        } else if let Some(record) = members.iter().find(|r| r.authored.id == command.target_id) {
            HoleElement::Member(Box::new(record.clone()))
        } else {
            return Err(not_found());
        };

    // Replay and reuse, decided from the head model as in `edit`: the head holds the identical
    // record exactly when the retry is the same request.
    let op_host = host_uuid(&request.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        let (evidence, _) = crate::command_evidence::accepted(store, &op_host)?
            .ok_or_else(|| integrity("accepted hole operation has no evidence"))?;
        Ok(evidence.fingerprint)
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            return Ok(AddedHole {
                replayed: true,
                element: head_element,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
            });
        }
        PriorOp::Accepted { .. } => {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "operation_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        PriorOp::Unseen => {}
    }

    // The record id is unused in the project, not only on the target (F04 section 4): the
    // id namespaces of the A04/A04.2 hole records, the A13 polygon cuts and the A03 cut
    // records are one project-wide space, so a record on any plate or member also refuses
    // the id here.
    let used = |id: OpId| {
        hole_history.values().any(|h| h.retired.contains(&id))
            || member_hole_history
                .values()
                .any(|h| h.retired.contains(&id))
            || history.values().any(|h| h.retired.contains(&id))
            || plates.iter().any(|p| {
                p.authored.hole_records.iter().any(|r| r.operation_id == id)
                    || p.authored.cuts.iter().any(|c| c.operation_id == id)
            })
            || members.iter().any(|m| {
                m.authored.cuts.iter().any(|c| c.operation_id == id)
                    || m.authored.hole_records.iter().any(|r| r.operation_id == id)
            })
    };
    let materials = material_lookup(&definitions);
    let canonical = instances.definition(command.target_id)?;
    let (element, changes) = match &head_element {
        HoleElement::Plate(head_plate) => {
            let material = head_plate
                .authored
                .material
                .as_ref()
                .map(&materials)
                .transpose()?;
            let (authored, body) = if remove {
                beamcraft_engine::command::remove_hole(&head_plate.authored, &removal)?
            } else if update {
                beamcraft_engine::command::update_hole(&head_plate.authored, &edit)?
            } else {
                beamcraft_engine::command::add_hole_after(
                    &head_plate.authored,
                    command,
                    &used,
                    hole_history
                        .get(&command.target_id)
                        .map_or(0, |h| usize::from(h.next_ordinal)),
                )?
            };
            let quantities = body.quantities_for(material)?;
            let next = PlateRecord {
                authored,
                body,
                quantities,
            };
            let mut others = Vec::new();
            for id in instances.group(command.target_id)? {
                if id == command.target_id {
                    continue;
                }
                let mut authored = plates
                    .iter()
                    .find(|p| p.authored.id == id)
                    .ok_or_else(|| integrity("shared hole plate is not evaluated"))?
                    .authored
                    .clone();
                authored.hole_records = next.authored.hole_records.clone();
                others.push(PlateRecord {
                    authored,
                    body: next.body.clone(),
                    quantities: next.quantities,
                });
            }
            let mut changes = vec![
                Change::HoleRecords {
                    id: canonical.to_string(),
                    records: next.authored.hole_records.clone(),
                },
                mesh_change(&next.authored.id, &next.body.solid, &plate_record(&next))?,
            ];
            for other in &others {
                changes.push(mesh_change(
                    &other.authored.id,
                    &other.body.solid,
                    &plate_record(other),
                )?);
            }
            (HoleElement::Plate(Box::new(next)), changes)
        }
        HoleElement::Member(head_member) => {
            // A04.2: the member records live on the member; the engine admits the record and
            // the replay here refuses the whole command when the hole no longer fits.
            let outcome = if remove {
                beamcraft_engine::member_hole::remove_member_hole(&removal, &head_member.authored)?
            } else if update {
                beamcraft_engine::member_hole::update_member_hole(&edit, &head_member.authored)?
            } else {
                beamcraft_engine::member_hole::add_member_hole_after(
                    command,
                    &head_member.authored,
                    &used,
                    member_hole_history
                        .get(&command.target_id)
                        .map_or(0, |h| usize::from(h.next_ordinal)),
                )?
            };
            let profile = profile_lookup(&definitions)(&outcome.authored.profile)?;
            let body = evaluate_member(&outcome.authored, profile)?;
            let quantities = member_quantities(
                &body,
                outcome
                    .authored
                    .material
                    .as_ref()
                    .map(&materials)
                    .transpose()?,
            )?;
            let next = MemberRecord {
                authored: outcome.authored.clone(),
                body,
                quantities,
            };
            let mut others = Vec::new();
            for id in instances.group(command.target_id)? {
                if id == command.target_id {
                    continue;
                }
                let mut authored = members
                    .iter()
                    .find(|m| m.authored.id == id)
                    .ok_or_else(|| integrity("shared hole member is not evaluated"))?
                    .authored
                    .clone();
                authored.hole_records = next.authored.hole_records.clone();
                others.push(MemberRecord {
                    authored,
                    body: next.body.clone(),
                    quantities: next.quantities,
                });
            }
            let mut changes = vec![
                Change::MemberHoles {
                    id: canonical.to_string(),
                    records: next.authored.hole_records.clone(),
                },
                mesh_change(&next.authored.id, &next.body.solid, &member_record(&next))?,
            ];
            for other in &others {
                changes.push(mesh_change(
                    &other.authored.id,
                    &other.body.solid,
                    &member_record(other),
                )?);
            }
            (HoleElement::Member(Box::new(next)), changes)
        }
    };
    let footprint = {
        let mut footprint =
            Footprint::for_update(command.target_id, false, false, true, false, None)?
                .union(instances.dependencies(command.target_id)?)?;
        for id in instances.group(command.target_id)? {
            footprint =
                footprint.union(Footprint::for_update(id, false, false, true, false, None)?)?;
        }
        footprint
    };
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
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes,
        },
        &control,
    )?;
    check_footprint(&prepared, &footprint)?;
    let changed_layers = changed_layers(&prepared, &before);
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
    Ok(AddedHole {
        replayed: false,
        element,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
    })
}
