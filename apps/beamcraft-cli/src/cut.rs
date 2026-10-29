// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A03 adapter: `CutByPlane` for a plate or a member, committed as the next revision.
//!
//! One owned `plane_cut` operation record (F04 section 4, amendment A-2) is appended to the
//! target's authored state as five parallel USD attributes, the body is re-evaluated by the
//! ordinary replay, and the mesh and quantity records are rewritten in the same transaction, so
//! the stored quantities are current again the moment the command is accepted. One accepted
//! command is one revision and therefore one undo unit (the workspace snapshot undo of P07).
//!
//! Feature stability is the kernel's: the faces of the base body keep their ids, the cap face is
//! `cut:f.p<ordinal>`, and a feature the plane removes or splits no longer resolves (the
//! explicit loss; `feature-lookup` reports it as not found).
//!
//! Honesty limits: replay of an `op_id` is decided from the head model (the command is a replay
//! when the head already holds the appended record), as in `edit`. One implicit workspace,
//! in-process layer engine, as in `plate`.
//!
//! The same adapter also runs the A13 fabrication commands (`a13_record`): line and polygon
//! cuts, a cut by a part, fitting an end to a part, chamfers, split and combine. Each derives
//! its record or state change at admission through `beamcraft_engine::fabricate`; a split
//! creates the second element in the same transaction and a combine deletes the consumed
//! member, and each is one revision and one undo unit.
//!
//! Replay reads immutable accepted request evidence and its exact source revision before
//! consulting the live target. Later edits, record removal or target deletion cannot make
//! the original command mutate again.

use crate::definition::{material_lookup, profile_lookup};
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    decode, digest_of, evaluate_decoded, host_uuid, integrity, load_base, member_record,
    mesh_change, open_base, plate_record, prior_op_with, replay_summary, summarize,
};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::command::evaluate_plate;
use beamcraft_engine::cut::{CutByPlaneOutcome, CutTarget};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::member::{evaluate_member, member_quantities};
use beamcraft_engine::query::committed::{MemberRecord, PlateRecord};
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, MemberKind, Placement, Prepare, Shape, schema_identity,
};
use beamcraft_layer_engine::index::ElementId as LayerElementId;
use beamcraft_model::command::{
    ChamferPart, CombineParts, CommandError, CutByLine, CutByPart, CutByPlane, CutByPolygon,
    CutRecord, FitEndToPart, SplitPart,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

fn not_found() -> PlateError {
    PlateError::from(CommandError::new(
        ErrorCode::NotFound,
        "target_id",
        "the element is not in the head revision",
    ))
}

fn unsupported_target() -> PlateError {
    PlateError::from(CommandError::new(
        ErrorCode::DependencyUnavailable,
        "target_id",
        "cutting an unsupported element is not implemented",
    ))
}

fn reuse() -> PlateError {
    PlateError::from(CommandError::new(
        ErrorCode::OpIdReuse,
        "op_id",
        "operation id was accepted with different content",
    ))
}

/// Inputs of one `cut_by_plane` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct CutRequest {
    /// The logical operation. A retry with the same id and identical content is a replay.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; a cut of such a store is `NOT_FOUND`.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: CutByPlane,
}

/// The cut element as it is after the command.
#[derive(Debug, Clone, PartialEq)]
pub enum CutElement {
    /// A plate.
    Plate(Box<PlateRecord>),
    /// A beam or a column.
    Member(Box<MemberRecord>),
}

/// Result of `cut_by_plane`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The element from the accepted revision, including on replay.
    pub element: CutElement,
    /// The ordinal the record took.
    pub ordinal: u16,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
}

/// Cuts one element and publishes the next revision, or replays the operation.
///
/// Steps, each failing closed: admit the command; open and verify the head; find the target and
/// check the record identity across the project; append the record (the engine decides the
/// ordinal and the footprint); re-evaluate the whole body from the authored state (the pinned
/// material resolved from the head's definitions); decide replay from the host ledger; author
/// the cut attributes and the rebuilt mesh with U02; commit through U06/U07 with the strict
/// verifier of `plate`.
pub fn cut_by_plane_record(
    store: &RevisionStore,
    request: &CutRequest,
    hook: &mut dyn CommitHook,
) -> Result<Cut, PlateError> {
    apply_cut(store, request, hook, Action::Create)
}
/// Replace an existing cut with immutable retry evidence and one atomic linked-group update.
pub fn update_cut_record(
    store: &RevisionStore,
    request: &CutRequest,
    hook: &mut dyn CommitHook,
) -> Result<Cut, PlateError> {
    apply_cut(store, request, hook, Action::Update)
}
/// Context and typed arguments for removal of one existing cut record.
#[derive(Debug, Clone)]
pub struct RemoveCutRequest {
    /// Logical operation identity, independent of the retired record identity.
    pub op_id: OpId,
    /// Next revision identity.
    pub revision_id: RevisionId,
    /// Trusted actor.
    pub actor: Uuid7,
    /// Store project context.
    pub project: ProjectId,
    /// Store workspace context.
    pub workspace: WorkspaceId,
    /// Target and owned record.
    pub command: beamcraft_model::command::RemoveCut,
}
/// Removal response: there is no live ordinal to return for a deleted record.
#[derive(Debug, Clone, PartialEq)]
pub struct RemovedCut {
    /// True for an accepted immutable request replay.
    pub replayed: bool,
    /// Part after removal, from the accepted revision on replay.
    pub element: CutElement,
    /// Accepted revision.
    pub revision: RevisionSummary,
    /// Actual admitted footprint, absent for a replay.
    pub footprint: Option<Footprint>,
    /// Written layers, empty for a replay.
    pub changed_layers: Vec<String>,
}
/// Remove, rebuild and publish the full canonical instance group atomically.
pub fn remove_cut_record(
    store: &RevisionStore,
    request: &RemoveCutRequest,
    hook: &mut dyn CommitHook,
) -> Result<RemovedCut, PlateError> {
    let context = CutRequest {
        op_id: request.op_id,
        revision_id: request.revision_id,
        actor: request.actor.clone(),
        project: request.project,
        workspace: request.workspace,
        command: CutByPlane {
            target_id: request.command.target_id,
            operation_id: request.command.operation_id,
            point_mm: [0.0; 3],
            normal: [1.0, 0.0, 0.0],
            keep: beamcraft_model::command::KeepSide::Negative,
        },
    };
    let r = apply_cut(store, &context, hook, Action::Remove)?;
    Ok(RemovedCut {
        replayed: r.replayed,
        element: r.element,
        revision: r.revision,
        footprint: r.footprint,
        changed_layers: r.changed_layers,
    })
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Create,
    Update,
    Remove,
}
fn replay_cut(
    store: &RevisionStore,
    request: &CutRequest,
    fingerprint: ContentDigest,
    remove: bool,
) -> Result<Option<Cut>, PlateError> {
    let op = host_uuid(&request.op_id.to_string())?;
    let Some((evidence, revision)) = crate::command_evidence::accepted(store, &op)? else {
        return Ok(None);
    };
    if evidence.fingerprint != fingerprint {
        return Err(reuse());
    }
    let old = crate::plate::inspect_revision(&revision)?;
    let element = if let Some(p) = old
        .plates
        .iter()
        .find(|p| p.authored.id == request.command.target_id)
    {
        CutElement::Plate(Box::new(p.clone()))
    } else if let Some(m) = old
        .members
        .iter()
        .find(|m| m.authored.id == request.command.target_id)
    {
        CutElement::Member(Box::new(m.clone()))
    } else {
        return Err(integrity("accepted cut target missing from its revision"));
    };
    // Removal exposes no ordinal; its public response only names the retired UUID.
    let ordinal = if remove {
        0
    } else {
        element
            .cuts()
            .iter()
            .find(|r| r.operation_id == request.command.operation_id)
            .ok_or_else(|| integrity("accepted cut record missing from its revision"))?
            .ordinal
    };
    Ok(Some(Cut {
        replayed: true,
        element,
        ordinal,
        revision: old.revision,
        footprint: None,
        changed_layers: Vec::new(),
    }))
}
fn apply_cut(
    store: &RevisionStore,
    request: &CutRequest,
    hook: &mut dyn CommitHook,
    action: Action,
) -> Result<Cut, PlateError> {
    let update = action == Action::Update;
    let remove = action == Action::Remove;
    let command = &request.command;
    command.validate_arguments()?;
    let updated = beamcraft_model::command::UpdateCut { cut: *command };
    let removal = beamcraft_model::command::RemoveCut {
        target_id: command.target_id,
        operation_id: command.operation_id,
    };
    let fingerprint = if remove {
        removal.content_fingerprint()?
    } else if update {
        updated.content_fingerprint()?
    } else {
        command.content_fingerprint()?
    };
    if let Some(replayed) = replay_cut(store, request, fingerprint, remove)? {
        return Ok(replayed);
    }
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    if let Some(replayed) = replay_cut(store, request, fingerprint, remove)? {
        return Ok(replayed);
    }
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
    let (plates, members, definitions, instances, history, hole_history) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, members) = evaluate_decoded(&decoded)?;
        // A stored element the evaluator refused is a known, unsupported target, not a
        // missing one (as `edit` orders it).
        if decoded
            .plates
            .iter()
            .any(|(p, _)| p.id == command.target_id)
            && plates.iter().all(|p| p.authored.id != command.target_id)
            && members.iter().all(|m| m.authored.id != command.target_id)
        {
            return Err(unsupported_target());
        }
        (
            plates,
            members,
            decoded.definitions,
            decoded.instances,
            decoded.cut_history,
            decoded.hole_history,
        )
    };
    let head_plate = plates.iter().find(|r| r.authored.id == command.target_id);
    let head_member = members.iter().find(|r| r.authored.id == command.target_id);
    let head_element = match (head_plate, head_member) {
        (Some(record), _) => CutElement::Plate(Box::new(record.clone())),
        (None, Some(record)) => CutElement::Member(Box::new(record.clone())),
        (None, None) => return Err(not_found()),
    };
    let used: Vec<OpId> = plates
        .iter()
        .flat_map(|r| r.authored.cuts.iter())
        .chain(members.iter().flat_map(|r| r.authored.cuts.iter()))
        .map(|c| c.operation_id)
        .chain(
            plates
                .iter()
                .flat_map(|p| p.authored.hole_records.iter().map(|r| r.operation_id)),
        )
        .chain(
            hole_history
                .values()
                .flat_map(|h| h.retired.iter().copied()),
        )
        .chain(history.values().flat_map(|h| h.retired.iter().copied()))
        .collect();
    let op_host = host_uuid(&request.op_id.to_string())?;

    let (plate, member) = match &head_element {
        CutElement::Plate(record) => (Some(&record.authored), None),
        CutElement::Member(record) => (None, Some(&record.authored)),
    };
    let outcome: CutByPlaneOutcome = if remove {
        beamcraft_engine::cut::remove_cut(&removal, plate, member)?
    } else if update {
        beamcraft_engine::cut::update_cut(&updated, plate, member)?
    } else {
        beamcraft_engine::cut::cut_by_plane_after(
            command,
            plate,
            member,
            &|id: &OpId| used.contains(id),
            history
                .get(&command.target_id)
                .map_or(0, |h| h.next_ordinal),
        )?
    };
    // The element after the replay, with fresh quantities from the same evaluation.
    let materials = material_lookup(&definitions);
    let element = match &outcome.target {
        CutTarget::Plate(authored) => {
            let body = evaluate_plate(authored)?;
            let quantities =
                body.quantities_for(authored.material.as_ref().map(&materials).transpose()?)?;
            CutElement::Plate(Box::new(PlateRecord {
                authored: authored.clone(),
                body,
                quantities,
            }))
        }
        CutTarget::Member(authored) => {
            let profile = profile_lookup(&definitions)(&authored.profile)?;
            let body = evaluate_member(authored, profile)?;
            let quantities = member_quantities(
                &body,
                authored.material.as_ref().map(&materials).transpose()?,
            )?;
            CutElement::Member(Box::new(MemberRecord {
                authored: authored.clone(),
                body,
                quantities,
            }))
        }
    };

    let canonical = instances.definition(command.target_id)?;
    let mut footprint = outcome
        .footprint
        .clone()
        .union(instances.dependencies(command.target_id)?)?;
    let mut others = Vec::new();
    for id in instances.group(command.target_id)? {
        if id == command.target_id {
            continue;
        }
        footprint = footprint.union(Footprint::for_update(id, false, false, true, false, None)?)?;
        let other = match &element {
            CutElement::Plate(r) => {
                let mut authored = plates
                    .iter()
                    .find(|p| p.authored.id == id)
                    .ok_or_else(|| integrity("shared cut plate is not evaluated"))?
                    .authored
                    .clone();
                authored.cuts = r.authored.cuts.clone();
                CutElement::Plate(Box::new(PlateRecord {
                    authored,
                    body: r.body.clone(),
                    quantities: r.quantities,
                }))
            }
            CutElement::Member(r) => {
                let mut authored = members
                    .iter()
                    .find(|p| p.authored.id == id)
                    .ok_or_else(|| integrity("shared cut member is not evaluated"))?
                    .authored
                    .clone();
                authored.cuts = r.authored.cuts.clone();
                CutElement::Member(Box::new(MemberRecord {
                    authored,
                    body: r.body.clone(),
                    quantities: r.quantities,
                }))
            }
        };
        others.push(other);
    }
    let id_text = canonical.to_string();
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
    let record = |element: &CutElement| match element {
        CutElement::Plate(r) => mesh_change(&r.authored.id, &r.body.solid, &plate_record(r)),
        CutElement::Member(r) => mesh_change(&r.authored.id, &r.body.solid, &member_record(r)),
    };
    let mut changes = vec![
        Change::Cuts {
            id: id_text,
            cuts: element.cuts().to_vec(),
        },
        record(&element)?,
    ];
    for other in &others {
        changes.push(record(other)?);
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
    Ok(Cut {
        replayed: false,
        element,
        ordinal: outcome.ordinal,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
    })
}

impl CutElement {
    /// The plane-cut records of the element.
    pub fn cuts(&self) -> &[CutRecord] {
        match self {
            Self::Plate(r) => &r.authored.cuts,
            Self::Member(r) => &r.authored.cuts,
        }
    }
}

/// The authored state of one element, for probe comparisons and change authoring.
#[derive(Debug, Clone, PartialEq)]
enum AuthoredState {
    /// A plate's authored state.
    Plate(beamcraft_engine::command::AuthoredPlate),
    /// A member's authored state.
    Member(beamcraft_engine::member::AuthoredMember),
}

/// The authored state a cut target carries, without evaluating a body.
fn state_of_target(target: CutTarget) -> AuthoredState {
    match target {
        CutTarget::Plate(authored) => AuthoredState::Plate(authored),
        CutTarget::Member(authored) => AuthoredState::Member(authored),
    }
}

/// The authored state a split outcome carries.
fn state_of_part(part: beamcraft_engine::fabricate::AuthoredPart) -> AuthoredState {
    match part {
        beamcraft_engine::fabricate::AuthoredPart::Plate(authored) => {
            AuthoredState::Plate(authored)
        }
        beamcraft_engine::fabricate::AuthoredPart::Member(authored) => {
            AuthoredState::Member(authored)
        }
    }
}

/// One A13 fabrication command (A13): the engine call, the record vocabulary and the resulting
/// changes differ per variant, the adapter plumbing does not.
#[derive(Debug, Clone, PartialEq)]
pub enum A13Command {
    /// A cut with the plane through a line and an axis.
    Line(CutByLine),
    /// A through-cut with a polygon in the plate's local XY.
    Polygon(CutByPolygon),
    /// A through-cut with another plate's prism.
    Part(CutByPart),
    /// A trim flush with another plate's nearest face.
    Fit(FitEndToPart),
    /// A chamfer of one plate contour vertex.
    Chamfer(ChamferPart),
    /// A split of one element in two at a plane.
    Split(SplitPart),
    /// A combine of two collinear members into the survivor.
    Combine(CombineParts),
}

/// Inputs of one A13 `a13_record` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct A13Request {
    /// The logical operation. A retry with the same id and identical content is a replay.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; such a store is `NOT_FOUND`.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: A13Command,
}

/// Result of one A13 command.
#[derive(Debug, Clone, PartialEq)]
pub struct A13Result {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The affected elements in head order: the target (or survivor), then a split's created
    /// element.
    pub elements: Vec<CutElement>,
    /// The element a combine deleted.
    pub deleted: Option<ElementId>,
    /// The ordinal the record took, when the command appends one.
    pub ordinal: Option<u16>,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
}

/// Runs one A13 fabrication command and publishes the next revision, or replays the operation.
///
/// The plumbing follows `cut_by_plane_record`: open and verify the head, resolve the targets,
/// decide replay by probing the engine admission against the head state, apply for real,
/// re-evaluate every affected body, author the record and mesh changes (and a create, an axis
/// and placement change or a delete for split and combine) and commit as one transaction.
pub fn a13_record(
    store: &RevisionStore,
    request: &A13Request,
    hook: &mut dyn CommitHook,
) -> Result<A13Result, PlateError> {
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
    let (plates, members, definitions, instances, history, hole_history) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, members) = evaluate_decoded(&decoded)?;
        (
            plates,
            members,
            decoded.definitions,
            decoded.instances,
            decoded.cut_history,
            decoded.hole_history,
        )
    };
    let find = |id: ElementId| -> Option<CutElement> {
        plates
            .iter()
            .find(|r| r.authored.id == id)
            .map(|r| CutElement::Plate(Box::new(r.clone())))
            .or_else(|| {
                members
                    .iter()
                    .find(|r| r.authored.id == id)
                    .map(|r| CutElement::Member(Box::new(r.clone())))
            })
    };
    let present = |id: ElementId| find(id).is_some();
    let plate_of = |id: ElementId| -> Option<beamcraft_engine::command::AuthoredPlate> {
        match find(id) {
            Some(CutElement::Plate(record)) => Some(record.authored),
            _ => None,
        }
    };
    let member_of = |id: ElementId| -> Option<beamcraft_engine::member::AuthoredMember> {
        match find(id) {
            Some(CutElement::Member(record)) => Some(record.authored),
            _ => None,
        }
    };
    // The elements a replay predicate compares and the ids the commands touch.
    let (primary_id,) = match &request.command {
        A13Command::Line(c) => (c.target_id,),
        A13Command::Polygon(c) => (c.target_id,),
        A13Command::Part(c) => (c.target_id,),
        A13Command::Fit(c) => (c.target_id,),
        A13Command::Chamfer(c) => (c.target_id,),
        A13Command::Split(c) => (c.target_id,),
        A13Command::Combine(c) => (c.survivor_id,),
    };
    // Every operation id of the project, from both record vocabularies and their retirement
    // histories: a retired identity is never reusable, as in `apply_cut`.
    let used: Vec<OpId> = plates
        .iter()
        .flat_map(|r| {
            r.authored
                .cuts
                .iter()
                .map(|c| c.operation_id)
                .chain(r.authored.hole_records.iter().map(|c| c.operation_id))
        })
        .chain(
            members
                .iter()
                .flat_map(|r| r.authored.cuts.iter().map(|c| c.operation_id)),
        )
        .chain(
            hole_history
                .values()
                .flat_map(|h| h.retired.iter().copied()),
        )
        .chain(history.values().flat_map(|h| h.retired.iter().copied()))
        .collect();
    let used_contains = |id: &OpId| used.contains(id);
    let no_used = |_: &OpId| false;
    let no_id_used = |_: &ElementId| false;
    let head_primary = find(primary_id);
    // The record identity the command appends, when it appends one.
    let record_op = match &request.command {
        A13Command::Line(c) => Some(c.operation_id),
        A13Command::Polygon(c) => Some(c.operation_id),
        A13Command::Part(c) => Some(c.operation_id),
        A13Command::Fit(c) => Some(c.operation_id),
        A13Command::Chamfer(c) => Some(c.operation_id),
        A13Command::Split(c) => Some(c.operation_id),
        A13Command::Combine(_) => None,
    };
    // The ordinal a retry must rebuild exactly: the head record carrying this command's
    // identity, whose ordinal the live records alone do not imply (the persistent counter may
    // hold retired gaps). A fresh command's probe discards its result anyway.
    let probe_next = record_op
        .and_then(|operation| match head_primary.as_ref() {
            Some(CutElement::Plate(r)) => r
                .authored
                .cuts
                .iter()
                .find(|c| c.operation_id == operation)
                .map(|c| c.ordinal)
                .or_else(|| {
                    r.authored
                        .hole_records
                        .iter()
                        .find(|c| c.operation_id == operation)
                        .and_then(|c| u16::try_from(c.ordinal).ok())
                }),
            Some(CutElement::Member(r)) => r
                .authored
                .cuts
                .iter()
                .find(|c| c.operation_id == operation)
                .map(|c| c.ordinal),
            None => None,
        })
        .unwrap_or(0);
    // The head states for the probe: on a retry the primary element's own last record carries
    // the command's identity, and the probe must rebuild exactly it, so that record is removed
    // from the probe's input first.
    let strip =
        |authored: &mut beamcraft_engine::command::AuthoredPlate, id: ElementId, through: bool| {
            if id != primary_id {
                return;
            }
            let Some(operation) = record_op else {
                return;
            };
            if through
                && authored
                    .hole_records
                    .last()
                    .is_some_and(|r| r.operation_id == operation)
            {
                authored.hole_records.pop();
            } else if authored
                .cuts
                .last()
                .is_some_and(|r| r.operation_id == operation)
            {
                authored.cuts.pop();
            }
        };
    let strip_member = |authored: &mut beamcraft_engine::member::AuthoredMember, id: ElementId| {
        if id != primary_id {
            return;
        }
        let Some(operation) = record_op else {
            return;
        };
        if authored
            .cuts
            .last()
            .is_some_and(|r| r.operation_id == operation)
        {
            authored.cuts.pop();
        }
    };
    let probe_plate_of = |id: ElementId| -> Option<beamcraft_engine::command::AuthoredPlate> {
        match find(id) {
            Some(CutElement::Plate(record)) => {
                let mut authored = record.authored.clone();
                strip(&mut authored, id, true);
                Some(authored)
            }
            _ => None,
        }
    };
    let probe_member_of = |id: ElementId| -> Option<beamcraft_engine::member::AuthoredMember> {
        match find(id) {
            Some(CutElement::Member(record)) => {
                let mut authored = record.authored.clone();
                strip_member(&mut authored, id);
                Some(authored)
            }
            _ => None,
        }
    };

    // The probe: what the command would produce from the head with the identity checks
    // relaxed. Comparing its authored states with the head decides replay without the ledger.
    #[allow(
        clippy::large_enum_variant,
        reason = "a short-lived local probe of one or two authored states"
    )]
    #[derive(Debug, Clone, PartialEq)]
    enum Probe {
        /// The single element after the command.
        One(AuthoredState),
        /// A split's kept and created states.
        Split(AuthoredState, AuthoredState),
    }
    let state_of = |element: &CutElement| match element {
        CutElement::Plate(r) => AuthoredState::Plate(r.authored.clone()),
        CutElement::Member(r) => AuthoredState::Member(r.authored.clone()),
    };
    let probe = (|| -> Option<Probe> {
        Some(match &request.command {
            A13Command::Line(c) => Probe::One(
                match beamcraft_engine::fabricate::cut_by_line(
                    c,
                    probe_plate_of(c.target_id).as_ref(),
                    probe_member_of(c.target_id).as_ref(),
                    &no_used,
                    probe_next,
                ) {
                    Ok(outcome) => state_of_target(outcome.target),
                    Err(_) => return None,
                },
            ),
            A13Command::Fit(c) => Probe::One(
                match beamcraft_engine::fabricate::fit_end_to_part(
                    c,
                    probe_plate_of(c.target_id).as_ref(),
                    probe_member_of(c.target_id).as_ref(),
                    plate_of(c.tool_id).as_ref(),
                    &no_used,
                    probe_next,
                ) {
                    Ok(outcome) => state_of_target(outcome.target),
                    Err(_) => return None,
                },
            ),
            A13Command::Chamfer(c) => Probe::One(
                match beamcraft_engine::fabricate::chamfer_part(
                    c,
                    probe_plate_of(c.target_id).as_ref(),
                    probe_member_of(c.target_id).as_ref(),
                    &no_used,
                    probe_next,
                ) {
                    Ok(outcome) => state_of_target(outcome.target),
                    Err(_) => return None,
                },
            ),
            A13Command::Polygon(c) => Probe::One(
                match beamcraft_engine::fabricate::cut_by_polygon(
                    c,
                    probe_plate_of(c.target_id).as_ref(),
                    probe_member_of(c.target_id).as_ref(),
                    &no_used,
                    probe_next,
                ) {
                    Ok(outcome) => AuthoredState::Plate(outcome.authored),
                    Err(_) => return None,
                },
            ),
            A13Command::Part(c) => Probe::One(
                match beamcraft_engine::fabricate::cut_by_part(
                    c,
                    probe_plate_of(c.target_id).as_ref(),
                    probe_member_of(c.target_id).as_ref(),
                    plate_of(c.tool_id).as_ref(),
                    &no_used,
                    probe_next,
                ) {
                    Ok(outcome) => AuthoredState::Plate(outcome.authored),
                    Err(_) => return None,
                },
            ),
            A13Command::Split(c) => {
                let outcome = beamcraft_engine::fabricate::split_part(
                    c,
                    probe_plate_of(c.target_id).as_ref(),
                    probe_member_of(c.target_id).as_ref(),
                    &no_used,
                    &no_id_used,
                    probe_next,
                )
                .ok()?;
                Probe::Split(state_of_part(outcome.kept), state_of_part(outcome.created))
            }
            A13Command::Combine(_) => return None,
        })
    })();
    // The head holds this operation's result exactly when the probe reproduces it. A combine
    // is decided like a delete: a retry's consumed member is already gone, so the effect is
    // held when the consumed identity is absent from the head.
    let holds = match (&probe, &request.command) {
        (_, A13Command::Combine(c)) => !present(c.consumed_id),
        (Some(Probe::One(after)), _) => head_primary
            .as_ref()
            .is_some_and(|head| state_of(head) == *after),
        (Some(Probe::Split(kept, created)), A13Command::Split(c)) => {
            // A committed split holds both halves, and both must match the probe: a changed
            // new_name or a target that gained a further record changes the created copy and
            // is a reuse refusal, as for any other changed content.
            head_primary
                .as_ref()
                .is_some_and(|head| state_of(head) == *kept)
                && find(c.new_id).is_some_and(|existing| state_of(&existing) == *created)
        }
        _ => false,
    };
    let op_host = host_uuid(&request.op_id.to_string())?;
    let fingerprint = match &request.command {
        A13Command::Line(c) => c.content_fingerprint()?,
        A13Command::Polygon(c) => c.content_fingerprint()?,
        A13Command::Part(c) => c.content_fingerprint()?,
        A13Command::Fit(c) => c.content_fingerprint()?,
        A13Command::Chamfer(c) => c.content_fingerprint()?,
        A13Command::Split(c) => c.content_fingerprint()?,
        A13Command::Combine(c) => c.content_fingerprint()?,
    };
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        if holds {
            Ok(fingerprint)
        } else {
            digest_of(b"no accepted request matches the head state")
        }
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            let elements = match &request.command {
                A13Command::Split(c) => match (&head_primary, find(c.new_id)) {
                    (Some(head), Some(created)) => vec![head.clone(), created],
                    _ => return Err(integrity("a split replay lost its halves")),
                },
                _ => match &head_primary {
                    Some(head) => vec![head.clone()],
                    None => return Err(not_found()),
                },
            };
            let deleted = match &request.command {
                A13Command::Combine(c) => Some(c.consumed_id),
                _ => None,
            };
            return Ok(A13Result {
                replayed: true,
                elements,
                deleted,
                ordinal: None,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
            });
        }
        PriorOp::Accepted { .. } => return Err(reuse()),
        PriorOp::Unseen => {}
    }

    // The real admission, the re-evaluated bodies and the layer changes of one transaction.
    let materials = material_lookup(&definitions);
    let evaluate_state = |authored: AuthoredState| -> Result<CutElement, PlateError> {
        match authored {
            AuthoredState::Plate(a) => {
                let body = evaluate_plate(&a)?;
                let quantities =
                    body.quantities_for(a.material.as_ref().map(&materials).transpose()?)?;
                Ok(CutElement::Plate(Box::new(PlateRecord {
                    authored: a,
                    body,
                    quantities,
                })))
            }
            AuthoredState::Member(a) => {
                let profile = profile_lookup(&definitions)(&a.profile)?;
                let body = evaluate_member(&a, profile)?;
                let quantities =
                    member_quantities(&body, a.material.as_ref().map(&materials).transpose()?)?;
                Ok(CutElement::Member(Box::new(MemberRecord {
                    authored: a,
                    body,
                    quantities,
                })))
            }
        }
    };
    let mut changes: Vec<Change> = Vec::new();
    let mut elements: Vec<CutElement> = Vec::new();
    let mut deleted: Option<ElementId> = None;
    let mut ordinal: Option<u16> = None;
    let mut footprint;
    match &request.command {
        A13Command::Line(c) => {
            let outcome = beamcraft_engine::fabricate::cut_by_line(
                c,
                plate_of(c.target_id).as_ref(),
                member_of(c.target_id).as_ref(),
                &used_contains,
                history.get(&c.target_id).map_or(0, |h| h.next_ordinal),
            )?;
            let (id_text, cuts, authored) = match &outcome.target {
                CutTarget::Plate(a) => (
                    a.id.to_string(),
                    a.cuts.clone(),
                    AuthoredState::Plate(a.clone()),
                ),
                CutTarget::Member(a) => (
                    a.id.to_string(),
                    a.cuts.clone(),
                    AuthoredState::Member(a.clone()),
                ),
            };
            let element = evaluate_state(authored)?;
            footprint = outcome
                .footprint
                .clone()
                .union(instances.dependencies(c.target_id)?)?;
            ordinal = Some(outcome.ordinal);
            changes.push(Change::Cuts {
                id: id_text.clone(),
                cuts: cuts.clone(),
            });
            changes.push(mesh_change_of(&element)?);
            let group = instances.group(c.target_id)?;
            let others: Vec<CutElement> = group
                .iter()
                .filter(|id| **id != c.target_id)
                .map(|id| shared_with_cuts(&plates, &members, *id, &element))
                .collect::<Result<_, _>>()?;
            elements.push(element);
            for (id, other) in group.iter().filter(|id| **id != c.target_id).zip(others) {
                footprint = footprint
                    .union(Footprint::for_update(*id, false, false, true, false, None)?)?;
                changes.push(mesh_change_of(&other)?);
                elements.push(other);
            }
        }
        A13Command::Fit(c) => {
            let outcome = beamcraft_engine::fabricate::fit_end_to_part(
                c,
                plate_of(c.target_id).as_ref(),
                member_of(c.target_id).as_ref(),
                plate_of(c.tool_id).as_ref(),
                &used_contains,
                history.get(&c.target_id).map_or(0, |h| h.next_ordinal),
            )?;
            let (id_text, cuts, authored) = match &outcome.target {
                CutTarget::Plate(a) => (
                    a.id.to_string(),
                    a.cuts.clone(),
                    AuthoredState::Plate(a.clone()),
                ),
                CutTarget::Member(a) => (
                    a.id.to_string(),
                    a.cuts.clone(),
                    AuthoredState::Member(a.clone()),
                ),
            };
            let element = evaluate_state(authored)?;
            footprint = outcome
                .footprint
                .clone()
                .union(instances.dependencies(c.target_id)?)?;
            ordinal = Some(outcome.ordinal);
            changes.push(Change::Cuts {
                id: id_text.clone(),
                cuts: cuts.clone(),
            });
            changes.push(mesh_change_of(&element)?);
            let group = instances.group(c.target_id)?;
            let others: Vec<CutElement> = group
                .iter()
                .filter(|id| **id != c.target_id)
                .map(|id| shared_with_cuts(&plates, &members, *id, &element))
                .collect::<Result<_, _>>()?;
            elements.push(element);
            for (id, other) in group.iter().filter(|id| **id != c.target_id).zip(others) {
                footprint = footprint
                    .union(Footprint::for_update(*id, false, false, true, false, None)?)?;
                changes.push(mesh_change_of(&other)?);
                elements.push(other);
            }
        }
        A13Command::Chamfer(c) => {
            let outcome = beamcraft_engine::fabricate::chamfer_part(
                c,
                plate_of(c.target_id).as_ref(),
                member_of(c.target_id).as_ref(),
                &used_contains,
                history.get(&c.target_id).map_or(0, |h| h.next_ordinal),
            )?;
            let (id_text, cuts, authored) = match &outcome.target {
                CutTarget::Plate(a) => (
                    a.id.to_string(),
                    a.cuts.clone(),
                    AuthoredState::Plate(a.clone()),
                ),
                CutTarget::Member(a) => (
                    a.id.to_string(),
                    a.cuts.clone(),
                    AuthoredState::Member(a.clone()),
                ),
            };
            let element = evaluate_state(authored)?;
            footprint = outcome
                .footprint
                .clone()
                .union(instances.dependencies(c.target_id)?)?;
            ordinal = Some(outcome.ordinal);
            changes.push(Change::Cuts {
                id: id_text.clone(),
                cuts: cuts.clone(),
            });
            changes.push(mesh_change_of(&element)?);
            let group = instances.group(c.target_id)?;
            let others: Vec<CutElement> = group
                .iter()
                .filter(|id| **id != c.target_id)
                .map(|id| shared_with_cuts(&plates, &members, *id, &element))
                .collect::<Result<_, _>>()?;
            elements.push(element);
            for (id, other) in group.iter().filter(|id| **id != c.target_id).zip(others) {
                footprint = footprint
                    .union(Footprint::for_update(*id, false, false, true, false, None)?)?;
                changes.push(mesh_change_of(&other)?);
                elements.push(other);
            }
        }
        A13Command::Polygon(c) => {
            let outcome = beamcraft_engine::fabricate::cut_by_polygon(
                c,
                plate_of(c.target_id).as_ref(),
                member_of(c.target_id).as_ref(),
                &used_contains,
                hole_history.get(&c.target_id).map_or(0, |h| h.next_ordinal),
            )?;
            let authored = outcome.authored.clone();
            let element = evaluate_state(AuthoredState::Plate(authored.clone()))?;
            footprint = outcome
                .footprint
                .clone()
                .union(instances.dependencies(c.target_id)?)?;
            ordinal = None;
            changes.push(Change::HoleRecords {
                id: authored.id.to_string(),
                records: authored.hole_records.clone(),
            });
            changes.push(mesh_change_of(&element)?);
            let group = instances.group(c.target_id)?;
            let others: Vec<CutElement> = group
                .iter()
                .filter(|id| **id != c.target_id)
                .map(|id| shared_with_holes(&plates, *id, &element))
                .collect::<Result<_, _>>()?;
            elements.push(element);
            for (id, other) in group.iter().filter(|id| **id != c.target_id).zip(others) {
                footprint = footprint
                    .union(Footprint::for_update(*id, false, false, true, false, None)?)?;
                changes.push(mesh_change_of(&other)?);
                elements.push(other);
            }
        }
        A13Command::Part(c) => {
            let outcome = beamcraft_engine::fabricate::cut_by_part(
                c,
                plate_of(c.target_id).as_ref(),
                member_of(c.target_id).as_ref(),
                plate_of(c.tool_id).as_ref(),
                &used_contains,
                hole_history.get(&c.target_id).map_or(0, |h| h.next_ordinal),
            )?;
            let authored = outcome.authored.clone();
            let element = evaluate_state(AuthoredState::Plate(authored.clone()))?;
            footprint = outcome
                .footprint
                .clone()
                .union(instances.dependencies(c.target_id)?)?;
            changes.push(Change::HoleRecords {
                id: authored.id.to_string(),
                records: authored.hole_records.clone(),
            });
            changes.push(mesh_change_of(&element)?);
            let group = instances.group(c.target_id)?;
            let others: Vec<CutElement> = group
                .iter()
                .filter(|id| **id != c.target_id)
                .map(|id| shared_with_holes(&plates, *id, &element))
                .collect::<Result<_, _>>()?;
            elements.push(element);
            for (id, other) in group.iter().filter(|id| **id != c.target_id).zip(others) {
                footprint = footprint
                    .union(Footprint::for_update(*id, false, false, true, false, None)?)?;
                changes.push(mesh_change_of(&other)?);
                elements.push(other);
            }
        }
        A13Command::Split(c) => {
            if instances.group_size(c.target_id)? > 1 {
                return Err(PlateError::from(CommandError::new(
                    ErrorCode::DependencyConflict,
                    "target_id",
                    "the element shares an instance definition; a split is per element",
                )));
            }
            let outcome = beamcraft_engine::fabricate::split_part(
                c,
                plate_of(c.target_id).as_ref(),
                member_of(c.target_id).as_ref(),
                &used_contains,
                &|id: &ElementId| present(*id),
                history.get(&c.target_id).map_or(0, |h| h.next_ordinal),
            )?;
            footprint = outcome.footprint.clone();
            ordinal = Some(outcome.ordinal);
            let kept_state = state_of_part(outcome.kept);
            let created_state = state_of_part(outcome.created);
            let kept = evaluate_state(kept_state.clone())?;
            let created = evaluate_state(created_state.clone())?;
            let (kept_cuts, kept_id) = match &kept_state {
                AuthoredState::Plate(a) => (a.cuts.clone(), a.id.to_string()),
                AuthoredState::Member(a) => (a.cuts.clone(), a.id.to_string()),
            };
            changes.push(Change::Cuts {
                id: kept_id,
                cuts: kept_cuts,
            });
            changes.push(Change::Create(created_element(&created_state)?));
            changes.push(mesh_change_of(&kept)?);
            changes.push(mesh_change_of(&created)?);
            elements.push(kept);
            elements.push(created);
        }
        A13Command::Combine(c) => {
            for id in [c.survivor_id, c.consumed_id] {
                if instances.group_size(id)? > 1 {
                    return Err(PlateError::from(CommandError::new(
                        ErrorCode::DependencyConflict,
                        "consumed_id",
                        "a member shares an instance definition; a combine is per element",
                    )));
                }
            }
            let outcome = beamcraft_engine::fabricate::combine_parts(
                c,
                member_of(c.survivor_id).as_ref(),
                member_of(c.consumed_id).as_ref(),
                &used_contains,
            )?;
            footprint = outcome.footprint.clone();
            let authored = outcome.survivor.clone();
            let before = member_of(c.survivor_id);
            let moved = before.is_some_and(|head| {
                head.placement.translation_mm != authored.placement.translation_mm
            });
            let survivor = evaluate_state(AuthoredState::Member(authored.clone()))?;
            changes.push(Change::Axis {
                id: authored.id.to_string(),
                points_mm: authored.axis_points_mm,
            });
            if moved {
                changes.push(Change::Placement {
                    id: authored.id.to_string(),
                    placement: Placement {
                        translation_mm: authored.placement.translation_mm,
                        orientation_xyzw: authored.placement.orientation_xyzw,
                    },
                });
            }
            changes.push(mesh_change_of(&survivor)?);
            changes.push(Change::Delete {
                id: c.consumed_id.to_string(),
            });
            elements.push(survivor);
            deleted = Some(c.consumed_id);
        }
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
    if let Some(consumed) = deleted {
        let index = author.element_index(&opened.context, &control)?;
        author.check_delete(
            &index,
            &LayerElementId::parse(&consumed.to_string())?,
            &control,
        )?;
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
    Ok(A13Result {
        replayed: false,
        elements,
        deleted,
        ordinal,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
    })
}

/// The mesh change of one element, for either kind.
fn mesh_change_of(element: &CutElement) -> Result<Change, PlateError> {
    match element {
        CutElement::Plate(r) => mesh_change(&r.authored.id, &r.body.solid, &plate_record(r)),
        CutElement::Member(r) => mesh_change(&r.authored.id, &r.body.solid, &member_record(r)),
    }
}

/// A shared-instance group member re-authored with the canonical part's cut records, carrying
/// the canonical's fresh body and quantities, exactly as `apply_cut` re-authors its own groups.
fn shared_with_cuts(
    plates: &[beamcraft_engine::query::committed::PlateRecord],
    members: &[beamcraft_engine::query::committed::MemberRecord],
    id: ElementId,
    canonical: &CutElement,
) -> Result<CutElement, PlateError> {
    match canonical {
        CutElement::Plate(r) => {
            let mut authored = plates
                .iter()
                .find(|p| p.authored.id == id)
                .ok_or_else(|| integrity("shared instance plate is not evaluated"))?
                .authored
                .clone();
            authored.cuts = r.authored.cuts.clone();
            Ok(CutElement::Plate(Box::new(PlateRecord {
                authored,
                body: r.body.clone(),
                quantities: r.quantities,
            })))
        }
        CutElement::Member(r) => {
            let mut authored = members
                .iter()
                .find(|m| m.authored.id == id)
                .ok_or_else(|| integrity("shared instance member is not evaluated"))?
                .authored
                .clone();
            authored.cuts = r.authored.cuts.clone();
            Ok(CutElement::Member(Box::new(MemberRecord {
                authored,
                body: r.body.clone(),
                quantities: r.quantities,
            })))
        }
    }
}

/// The same re-authoring for the through-cut vocabulary of a polygon or part cut: a polygon
/// cut always targets a plate, so the canonical element is one.
fn shared_with_holes(
    plates: &[beamcraft_engine::query::committed::PlateRecord],
    id: ElementId,
    canonical: &CutElement,
) -> Result<CutElement, PlateError> {
    let CutElement::Plate(r) = canonical else {
        return Err(integrity("a polygon cut canonical is not a plate"));
    };
    let mut authored = plates
        .iter()
        .find(|p| p.authored.id == id)
        .ok_or_else(|| integrity("shared instance plate is not evaluated"))?
        .authored
        .clone();
    authored.hole_records = r.authored.hole_records.clone();
    Ok(CutElement::Plate(Box::new(PlateRecord {
        authored,
        body: r.body.clone(),
        quantities: r.quantities,
    })))
}

/// The layer-engine `Create` element of a split's created half.
fn created_element(state: &AuthoredState) -> Result<Element, PlateError> {
    let pose = |placement: beamcraft_model::command::Placement| Placement {
        translation_mm: placement.translation_mm,
        orientation_xyzw: placement.orientation_xyzw,
    };
    match state {
        AuthoredState::Plate(authored) => Ok(Element {
            id: authored.id.to_string(),
            name: authored.name.clone(),
            placement: pose(authored.placement),
            material: authored.material,
            shape: Shape::Plate {
                contour_mm: authored.contour_points_mm.clone(),
                thickness_mm: authored.thickness_mm,
                thickness_side: authored.thickness_side,
                corners: authored.corners.clone(),
                holes: authored
                    .holes_mm
                    .iter()
                    .map(|hole| hole.iter().map(|[x, y]| [*x, *y, 0.0]).collect())
                    .collect(),
                hole_records: authored.hole_records.clone(),
                cuts: authored.cuts.clone(),
            },
        }),
        AuthoredState::Member(authored) => Ok(Element {
            id: authored.id.to_string(),
            name: authored.name.clone(),
            placement: pose(authored.placement),
            material: authored.material,
            shape: Shape::Member {
                kind: match authored.kind {
                    beamcraft_model::command::MemberKind::Beam => MemberKind::Beam,
                    beamcraft_model::command::MemberKind::Column => MemberKind::Column,
                },
                axis_mm: authored.axis_points_mm.clone(),
                profile: authored.profile,
                roll_rad: authored.roll_rad,
                curved: authored.curved,
                placement: authored.section_placement,
                cuts: authored.cuts.clone(),
            },
        }),
    }
}
