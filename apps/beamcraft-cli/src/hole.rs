// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A04 adapter: `AddHole` on a plate, committed as the next revision.
//!
//! The command appends one operation record (F04 section 4) to the target plate's authored
//! state; the engine replays the base shape with every hole in ordinal order and re-certifies
//! the body. The adapter writes the complete record set with U02, rewrites the derived mesh,
//! checks the layer engine's writes against the declared footprint and commits through U06/U07
//! with the strict verifier. Honesty limits: replay of an `operation_id` is decided from the
//! head model (the command is a replay when the head already holds the identical record), not
//! from a stored canonical request; a record edited by a later command turns a retry into
//! `OP_ID_REUSE`. A stored canonical request (F04 section 3) replaces this, as in `edit`. One
//! implicit workspace, in-process layer engine, as in `plate`.

use crate::definition::material_lookup;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    decode, digest_of, evaluate_decoded, host_uuid, integrity, load_base, mesh_change, open_base,
    plate_record, prior_op_with, replay_summary, summarize,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::command::add_hole;
use beamcraft_engine::footprint::Footprint;
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
/// `operation_id`: the logical operation of this command.
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

/// Result of `add_hole`.
#[derive(Debug, Clone, PartialEq)]
pub struct AddedHole {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The plate after the operation (the head's plate for a replay).
    pub plate: PlateRecord,
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

/// Adds one through-hole record to a plate and publishes the next revision, or replays the
/// operation.
///
/// Steps, each failing closed: admit the arguments; open and verify the head; find the target
/// plate (a member target is `UNSUPPORTED_GEOMETRY`, an authored-only unsupported plate is
/// `DEPENDENCY_UNAVAILABLE`); decide replay from the host ledger and the head's records; check
/// the record id across every plate of the project; replay the records with the kernel and
/// re-certify; author the records and the mesh with U02; check the layer engine's writes
/// against the declared footprint; commit through U06/U07 with the strict verifier (the stored
/// quantity records must match the kernel's). A refusal writes nothing.
pub fn add_hole_command(
    store: &RevisionStore,
    request: &AddHoleRequest,
    hook: &mut dyn CommitHook,
) -> Result<AddedHole, PlateError> {
    let command = &request.command;
    command.validate_arguments()?;
    let polygon = command.polygon()?;
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
    let (plates, members, definitions) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, members) = evaluate_decoded(&decoded)?;
        if decoded
            .plates
            .iter()
            .any(|(p, _)| p.id == command.target_id)
            && !plates.iter().any(|p| p.authored.id == command.target_id)
        {
            return Err(CommandError::new(
                ErrorCode::DependencyUnavailable,
                "target_id",
                "editing an unsupported plate is not implemented",
            )
            .into());
        }
        (plates, members, decoded.definitions)
    };
    let head_plate = plates
        .iter()
        .find(|r| r.authored.id == command.target_id)
        .ok_or_else(|| {
            if members
                .iter()
                .any(|m: &MemberRecord| m.authored.id == command.target_id)
            {
                // F04 section 7: a hole in a beam flange or web is unsupported until the
                // kernel can cut a prism along the member's faces.
                CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "target_id",
                    "a hole in a beam flange or web is not supported",
                )
                .into()
            } else {
                not_found()
            }
        })?;

    // Replay and reuse, decided from the head model as in `edit`: the head holds the identical
    // record exactly when the retry is the same request.
    let fingerprint = command.content_fingerprint()?;
    let op_host = host_uuid(&request.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        match head_plate
            .authored
            .hole_records
            .iter()
            .find(|r| r.operation_id == command.operation_id)
        {
            Some(record) if record.polygon_mm == polygon => Ok(fingerprint),
            _ => digest_of(b"no accepted request matches the head state"),
        }
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            return Ok(AddedHole {
                replayed: true,
                plate: head_plate.clone(),
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
    // id namespaces of the A04 hole records and of the A03 cut records are one project-wide
    // space, so a cut record on any plate or member also refuses the id here.
    let used = |id: OpId| {
        plates.iter().any(|p| {
            p.authored.hole_records.iter().any(|r| r.operation_id == id)
                || p.authored.cuts.iter().any(|c| c.operation_id == id)
        }) || members
            .iter()
            .any(|m| m.authored.cuts.iter().any(|c| c.operation_id == id))
    };
    let materials = material_lookup(&definitions);
    let material = head_plate
        .authored
        .material
        .as_ref()
        .map(&materials)
        .transpose()?;
    let (authored, body) = add_hole(&head_plate.authored, command, &used)?;
    let quantities = body.quantities_for(material)?;
    let next = PlateRecord {
        authored,
        body,
        quantities,
    };
    let footprint = Footprint::for_update(command.target_id, false, false, true, false, None)?;
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
    let changes = vec![
        Change::HoleRecords {
            id: command.target_id.to_string(),
            records: next.authored.hole_records.clone(),
        },
        mesh_change(&next.authored.id, &next.body.solid, &plate_record(&next))?,
    ];
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
    Ok(AddedHole {
        replayed: false,
        plate: next,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
    })
}
