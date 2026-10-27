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

use crate::definition::{material_lookup, profile_lookup};
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    decode, digest_of, evaluate_decoded, host_uuid, integrity, load_base, member_record,
    mesh_change, open_base, plate_record, prior_op_with, replay_summary, summarize,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::command::evaluate_plate;
use beamcraft_engine::cut::{CutByPlaneOutcome, CutTarget, cut_by_plane};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::member::{evaluate_member, member_quantities};
use beamcraft_engine::query::committed::{MemberRecord, PlateRecord};
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{Author, Change, EditTargets, Prepare, schema_identity};
use beamcraft_model::command::{CommandError, CutByPlane, CutRecord};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};
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
    /// The element after the cut (the head's element for a replay).
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
    let (plates, members, definitions) = {
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
        (plates, members, decoded.definitions)
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
        .collect();
    // The op-id decision runs before the record admission (F04 section 8 order): the head holds
    // this cut exactly when its last record is this command's record, which is how a retried
    // request differs from a second cut reusing a record identity.
    let op_host = host_uuid(&request.op_id.to_string())?;
    let fingerprint = command.content_fingerprint()?;
    let holds = |head: &CutElement| match head.cuts().last() {
        Some(record) => {
            record.operation_id == command.operation_id
                && record.point_mm == command.point_mm
                && record.normal == command.normal
                && record.keep == command.keep
        }
        None => false,
    };
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        if holds(&head_element) {
            Ok(fingerprint)
        } else {
            digest_of(b"no accepted request matches the head state")
        }
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            let ordinal = head_element.cuts().last().map_or(0, |c| c.ordinal);
            return Ok(Cut {
                replayed: true,
                element: head_element,
                ordinal,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
            });
        }
        PriorOp::Accepted { .. } => {
            return Err(reuse());
        }
        PriorOp::Unseen => {}
    }

    let (plate, member) = match &head_element {
        CutElement::Plate(record) => (Some(&record.authored), None),
        CutElement::Member(record) => (None, Some(&record.authored)),
    };
    let outcome: CutByPlaneOutcome =
        cut_by_plane(command, plate, member, &|id: &OpId| used.contains(id))?;
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

    let id_text = command.target_id.to_string();
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
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![
                Change::Cuts {
                    id: id_text.clone(),
                    cuts: element.cuts().to_vec(),
                },
                record(&element)?,
            ],
        },
        &control,
    )?;
    check_footprint(&prepared, &outcome.footprint)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &outcome.footprint,
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
        footprint: Some(outcome.footprint),
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
