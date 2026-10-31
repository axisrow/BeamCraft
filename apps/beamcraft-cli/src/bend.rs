// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A10 adapter: `BendPlate` for a plate, committed as the next revision.
//!
//! One owned bend operation record (fold or roll, F04 section 7 amendment A-2) is appended to
//! the target's authored state as seven parallel USD attributes, the bent body is re-evaluated
//! by the ordinary replay, and the mesh and quantity records are rewritten in the same
//! transaction, so the stored quantities are current again the moment the command is accepted.
//! One accepted command is one revision and therefore one undo unit (P07). Every instance of a
//! shared canonical plate takes the same records, as a cut does.
//!
//! Honesty limits: replay of an `op_id` is decided from the head model, as in `edit`. A bend
//! record is removed by undoing its command; there is no removal command in this slice.

use crate::definition::material_lookup;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    decode, evaluate_decoded, host_uuid, integrity, load_base, mesh_change, open_base,
    plate_record, summarize,
};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::command::evaluate_plate;
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::query::committed::PlateRecord;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{Author, Change, EditTargets, Prepare, schema_identity};
use beamcraft_model::command::BendPlate;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

fn not_found() -> PlateError {
    PlateError::from(beamcraft_model::command::CommandError::new(
        ErrorCode::NotFound,
        "target_id",
        "the element is not in the head revision",
    ))
}

fn reuse() -> PlateError {
    PlateError::from(beamcraft_model::command::CommandError::new(
        ErrorCode::OpIdReuse,
        "op_id",
        "operation id was accepted with different content",
    ))
}

/// Inputs of one `bend_plate` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct BendRequest {
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
    pub command: BendPlate,
}

/// Result of one `bend_plate` command.
#[derive(Debug, Clone, PartialEq)]
pub struct BendApplied {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The target plate from the accepted revision, including on replay.
    pub plate: Box<PlateRecord>,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
}

/// Runs one `BendPlate` command and publishes the next revision, or replays the operation.
///
/// The plumbing follows `apply_cut`: open and verify the head, resolve the target, decide
/// replay from the accepted request evidence, apply through the engine admission, re-evaluate
/// the bent body, author the bend attributes and the rebuilt mesh (also on every instance that
/// shares the plate's definition) and commit as one transaction.
pub fn bend_plate_record(
    store: &RevisionStore,
    request: &BendRequest,
    hook: &mut dyn CommitHook,
) -> Result<BendApplied, PlateError> {
    let command = &request.command;
    command.validate_arguments()?;
    let fingerprint = command.content_fingerprint()?;
    // A replay must be recognizable before anything else is read.
    if let Some(replayed) = replay(store, request, fingerprint)? {
        return Ok(replayed);
    }
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    if head.is_none() {
        return Err(not_found());
    }
    if let Some(replayed) = replay(store, request, fingerprint)? {
        return Ok(replayed);
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
    let (plates, definitions, instances) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, _) = evaluate_decoded(&decoded)?;
        (plates, decoded.definitions, decoded.instances)
    };
    let Some(record) = plates.iter().find(|r| r.authored.id == command.target_id) else {
        return Err(not_found());
    };
    // Every operation id of the project, from all record vocabularies and their retirement
    // histories: a retired identity is never reusable, as in `apply_cut`.
    let used: Vec<OpId> = plates
        .iter()
        .flat_map(|r| {
            r.authored
                .cuts
                .iter()
                .map(|c| c.operation_id)
                .chain(r.authored.hole_records.iter().map(|c| c.operation_id))
                .chain(r.authored.bends.iter().map(|c| c.operation_id))
        })
        .collect();
    let outcome = beamcraft_engine::bend::bend_plate(command, Some(&record.authored), &|id| {
        used.contains(id)
    })?;
    let materials = material_lookup(&definitions);
    let material = outcome
        .authored
        .material
        .as_ref()
        .map(&materials)
        .transpose()?;
    let body = evaluate_plate(&outcome.authored)?;
    let quantities = body.quantities_for(material)?;
    let element = PlateRecord {
        authored: outcome.authored.clone(),
        body,
        quantities,
    };
    let mut footprint = outcome.footprint.clone();
    footprint = footprint.union(instances.dependencies(command.target_id)?)?;
    let mut changes = vec![
        Change::Bends {
            id: command.target_id.to_string(),
            bends: element.authored.bends.clone(),
        },
        mesh_change(
            &element.authored.id,
            &element.body.solid,
            &plate_record(&element),
        )?,
    ];
    // Every instance of a shared definition takes the same records and the same body.
    let mut others: Vec<PlateRecord> = Vec::new();
    for id in instances.group(command.target_id)? {
        if id == command.target_id {
            continue;
        }
        footprint = footprint.union(Footprint::for_update(id, false, false, true, false, None)?)?;
        let mut authored = plates
            .iter()
            .find(|p| p.authored.id == id)
            .ok_or_else(|| integrity("shared bend plate is not evaluated"))?
            .authored
            .clone();
        authored.bends = element.authored.bends.clone();
        let body = evaluate_plate(&authored)?;
        let material = authored.material.as_ref().map(&materials).transpose()?;
        let quantities = body.quantities_for(material)?;
        let other = PlateRecord {
            authored,
            body,
            quantities,
        };
        changes.push(mesh_change(
            &other.authored.id,
            &other.body.solid,
            &plate_record(&other),
        )?);
        others.push(other);
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
    let changed = changed_layers(&prepared, &before);
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
            op: host_uuid(&request.op_id.to_string())?,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(BendApplied {
        replayed: false,
        plate: Box::new(element),
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers: changed,
    })
}

/// The accepted command evidence of this `op_id`, when the operation was already committed.
fn replay(
    store: &RevisionStore,
    request: &BendRequest,
    fingerprint: ContentDigest,
) -> Result<Option<BendApplied>, PlateError> {
    let op = host_uuid(&request.op_id.to_string())?;
    let Some((evidence, revision)) = crate::command_evidence::accepted(store, &op)? else {
        return Ok(None);
    };
    if evidence.fingerprint != fingerprint {
        return Err(reuse());
    }
    let old = crate::plate::inspect_revision(&revision)?;
    let plate = old
        .plates
        .iter()
        .find(|p| p.authored.id == request.command.target_id)
        .ok_or_else(|| integrity("accepted bend target missing from its revision"))?
        .clone();
    if !plate
        .authored
        .bends
        .iter()
        .any(|r| r.operation_id == request.command.operation_id)
    {
        return Err(integrity("accepted bend record missing from its revision"));
    }
    Ok(Some(BendApplied {
        replayed: true,
        plate: Box::new(plate),
        revision: old.revision,
        footprint: None,
        changed_layers: Vec::new(),
    }))
}
