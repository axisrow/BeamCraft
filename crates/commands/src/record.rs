// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 recorded-input creation, listing and delete routing, through the same atomic
//! revision pipeline as the annotation commands.
//!
//! A record is an authored element of kind `record` with no engineering body: a declared
//! joint intent or the gap requirement of one declared weld connection. Nothing in the
//! check harness writes records; only this explicit command creates them, and only
//! `delete-element` removes them. The declared scope and connection ids are stored
//! verbatim and may point at elements that stop existing later — the check reports such a
//! record as `target_unresolved` (contract section 4.3), and nothing auto-retargets it.
use crate::assembly::read_prims;
use crate::plate::{check_footprint, prior_op_with, replay_summary};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore};
use beamcraft_layer_engine::authored::{
    Change, Element, Prepare, Shape, schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Prim, Reader};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use beamcraft_model::record::{CreateRecord, Record};
use session::Refusal;
use session::pipeline::{
    Base, RevisionSummary, base_of, control, host_uuid, integrity, load_base, open_base, summarize,
};
use std::collections::{BTreeMap, BTreeSet};
use steel::engine::transaction::PriorOp;

/// Trusted context for a record creation, using the same identities as level creation.
pub type RecordContext = crate::level::LevelContext;

/// The committed or replayed result of a record creation.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordChange {
    /// Whether the operation was already committed.
    pub replayed: bool,
    /// Revision holding the result.
    pub revision: RevisionSummary,
    /// Declared writes, absent on replay.
    pub footprint: Option<Footprint>,
    /// Layers whose bytes changed.
    pub changed_layers: Vec<String>,
    /// The records at the resulting head, in identity order.
    pub records: Vec<Record>,
}

fn snapshot(request: OpenRequest, c: &Control) -> Result<Vec<Prim>, Refusal> {
    validate_layers(request.clone(), schema_identity()?, c)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, c)?;
    read_prims(&reader, &opened.context, c)
}

/// The records of the verified head, in identity order. Empty for a store with no revision.
pub fn records(store: &RevisionStore) -> Result<Vec<Record>, Refusal> {
    let revision = match store.open() {
        Ok(r) => r,
        Err(RevisionError::NoRevision) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    records_revision(&revision)
}

/// The records of one already verified revision, without reopening the head.
pub fn records_revision(
    revision: &beamcraft_host::revision::Revision,
) -> Result<Vec<Record>, Refusal> {
    Ok(beamcraft_layer_engine::authored::record::read_records(
        &snapshot(open_base(&base_of(revision)?)?, &control())?,
    )?)
}

/// Whether a verified snapshot has this owned record identity (for `delete-element`).
pub(crate) fn is_record(
    request: &OpenRequest,
    id: &ElementId,
    c: &Control,
) -> Result<bool, Refusal> {
    Ok(read_records(&snapshot(request.clone(), c)?)?
        .iter()
        .any(|r| r.id == *id))
}

fn read_records(prims: &[Prim]) -> Result<Vec<Record>, Refusal> {
    beamcraft_layer_engine::authored::record::read_records(prims).map_err(Refusal::from)
}

fn replay(
    store: &RevisionStore,
    context: &RecordContext,
    fingerprint: ContentDigest,
) -> Result<Option<RecordChange>, Refusal> {
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let op = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op, || Ok(fingerprint))?;
    if let PriorOp::Accepted { .. } = prior {
        let Some((_, sequence, _)) = &recorded else {
            return Err(integrity("accepted record operation has no revision"));
        };
        let revision = store.open_at(*sequence)?;
        let evidence = crate::command_evidence::read(&revision)?
            .ok_or_else(|| integrity("record operation has no request evidence"))?;
        if evidence.fingerprint != fingerprint {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        let listed = read_records(&snapshot(open_base(&base)?, &control())?)?;
        return Ok(Some(RecordChange {
            replayed: true,
            revision: replay_summary(&head, recorded)?,
            footprint: None,
            changed_layers: Vec::new(),
            records: listed,
        }));
    }
    Ok(None)
}

/// Create a record atomically, or replay the exact prior creation under the same operation
/// id. The declared ids are stored verbatim; their existence is not required here and is
/// judged at check time.
pub fn create_record(
    store: &RevisionStore,
    context: &RecordContext,
    command: &CreateRecord,
    hook: &mut dyn CommitHook,
) -> Result<RecordChange, Refusal> {
    command.validate_arguments()?;
    let fingerprint = command.content_fingerprint()?;
    if let Some(replayed) = replay(store, context, fingerprint)? {
        return Ok(replayed);
    }
    let c = control();
    let (_, base) = load_base(store, &context.project, &context.workspace)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let request = open_base(&base)?;
    let prims = snapshot(request.clone(), &c)?;
    let listed = read_records(&prims)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let op_host = host_uuid(&context.op_id.to_string())?;
    let elements = prims
        .iter()
        .filter_map(|p| p.element_id.as_deref())
        .map(ElementId::parse)
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|_| integrity("invalid element identity"))?;
    let footprint = beamcraft_engine::record::create(command, &listed, &elements)?;
    let schema = schema_identity()?;
    let (mut author, opened) = crate::scope::open_author(store, request, false, schema, &c)?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::Create(Element {
                id: command.record.id.to_string(),
                name: command.record.name.clone(),
                shape: Shape::Record(command.record.clone()),
                placement: beamcraft_layer_engine::authored::Placement {
                    translation_mm: [0.0; 3],
                    orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                },
                material: None,
            })],
        },
        &c,
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
            revision_id: &context.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("unseen record creation reported as replayed"));
    }
    Ok(RecordChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        records: records(store)?,
    })
}
