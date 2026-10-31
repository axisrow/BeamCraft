// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 headless construction object creation and verified listing, through the same atomic
//! revision pipeline as the grid commands.
use crate::assembly::read_prims;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    host_uuid, integrity, load_base, open_base, prior_op_with, replay_summary, summarize,
};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Prepare, Shape, construction as storage, schema_identity,
    validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Prim, Reader};
use beamcraft_model::command::{CommandError, CreateConstructionObject};
use beamcraft_model::construction::ConstructionObject;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::BTreeSet;

/// Trusted context for a construction object creation, using the same identities as level
/// creation.
pub type ConstructionContext = crate::level::LevelContext;

/// The committed or replayed result of a construction object creation.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstructionChange {
    /// Whether the operation was already committed.
    pub replayed: bool,
    /// Revision holding the result.
    pub revision: RevisionSummary,
    /// Declared writes, absent on replay.
    pub footprint: Option<Footprint>,
    /// Layers whose bytes changed.
    pub changed_layers: Vec<String>,
    /// The verified construction objects at the resulting head.
    pub constructions: Vec<ConstructionObject>,
}
fn snapshot(request: OpenRequest, c: &Control) -> Result<Vec<Prim>, PlateError> {
    validate_layers(request.clone(), schema_identity()?, c)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, c)?;
    read_prims(&reader, &opened.context, c)
}
/// Construction objects of the verified head, in identity order. Empty for a store with no
/// revision.
pub fn constructions(store: &RevisionStore) -> Result<Vec<ConstructionObject>, PlateError> {
    let revision = match store.open() {
        Ok(r) => r,
        Err(RevisionError::NoRevision) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(storage::read_constructions(&snapshot(
        open_base(&crate::plate::base_of(&revision)?)?,
        &control(),
    )?)?)
}
/// Whether a verified snapshot has this owned construction object identity (for
/// `delete-element`).
pub(crate) fn is_construction(
    request: &OpenRequest,
    id: &ElementId,
    c: &Control,
) -> Result<bool, PlateError> {
    Ok(storage::read_constructions(&snapshot(request.clone(), c)?)?
        .iter()
        .any(|o| o.id == *id))
}
fn replay(
    store: &RevisionStore,
    context: &ConstructionContext,
    fingerprint: beamcraft_model::digest::ContentDigest,
) -> Result<Option<ConstructionChange>, PlateError> {
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let op = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op, || Ok(fingerprint))?;
    if let PriorOp::Accepted { .. } = prior {
        let Some((_, sequence, _)) = &recorded else {
            return Err(integrity("accepted construction operation has no revision"));
        };
        let revision = store.open_at(*sequence)?;
        let evidence = crate::command_evidence::read(&revision)?
            .ok_or_else(|| integrity("construction operation has no request evidence"))?;
        if evidence.fingerprint != fingerprint {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        return Ok(Some(ConstructionChange {
            replayed: true,
            revision: replay_summary(&head, recorded)?,
            footprint: None,
            changed_layers: Vec::new(),
            constructions: storage::read_constructions(&snapshot(open_base(&base)?, &control())?)?,
        }));
    }
    Ok(None)
}
/// Create a construction object atomically, or replay the exact prior creation under the same
/// operation id.
pub fn create_construction(
    store: &RevisionStore,
    context: &ConstructionContext,
    command: &CreateConstructionObject,
    hook: &mut dyn CommitHook,
) -> Result<ConstructionChange, PlateError> {
    command.validate_arguments()?;
    let fingerprint = command.content_fingerprint()?;
    if let Some(replayed) = replay(store, context, fingerprint)? {
        return Ok(replayed);
    }
    let c = control();
    let (_, base) = load_base(store, &context.project, &context.workspace)?;
    let before: std::collections::BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let request = open_base(&base)?;
    let prims = snapshot(request.clone(), &c)?;
    let model = storage::read_constructions(&prims)?;
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
    let footprint = beamcraft_engine::construction::create(command, &model, &elements)?;
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &c,
    )?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::Create(Element {
                id: command.object.id.to_string(),
                name: command.object.name.clone(),
                shape: Shape::Construction(command.object.clone()),
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
        return Err(integrity(
            "unseen construction creation reported as replayed",
        ));
    }
    Ok(ConstructionChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        constructions: constructions(store)?,
    })
}
