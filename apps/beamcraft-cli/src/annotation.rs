// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D06 headless annotation creation, listing and binding resolution, through the same atomic
//! revision pipeline as the construction object commands.
use crate::assembly::read_prims;
use crate::attribute::attr_model;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    host_uuid, inspect, integrity, load_base, open_base, prior_op_with, replay_summary, summarize,
};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Prepare, Shape, annotation as storage, schema_identity,
    validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Prim, Reader};
use beamcraft_model::annotation::{Annotation, CreateAnnotation};
use beamcraft_model::attribute::AttrValue;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::BTreeSet;

/// Trusted context for an annotation creation, using the same identities as level creation.
pub type AnnotationContext = crate::level::LevelContext;

/// The committed or replayed result of an annotation creation.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationChange {
    /// Whether the operation was already committed.
    pub replayed: bool,
    /// Revision holding the result.
    pub revision: RevisionSummary,
    /// Declared writes, absent on replay.
    pub footprint: Option<Footprint>,
    /// Layers whose bytes changed.
    pub changed_layers: Vec<String>,
    /// The annotations at the resulting head, in identity order.
    pub annotations: Vec<Annotation>,
}

fn snapshot(request: OpenRequest, c: &Control) -> Result<Vec<Prim>, PlateError> {
    validate_layers(request.clone(), schema_identity()?, c)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, c)?;
    read_prims(&reader, &opened.context, c)
}

/// Annotations of the verified head, in identity order. Empty for a store with no revision.
pub fn annotations(store: &RevisionStore) -> Result<Vec<Annotation>, PlateError> {
    let revision = match store.open() {
        Ok(r) => r,
        Err(RevisionError::NoRevision) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(storage::read_annotations(&snapshot(
        open_base(&crate::plate::base_of(&revision)?)?,
        &control(),
    )?)?)
}

/// Whether a verified snapshot has this owned annotation identity (for `delete-element`).
pub(crate) fn is_annotation(
    request: &OpenRequest,
    id: &ElementId,
    c: &Control,
) -> Result<bool, PlateError> {
    Ok(storage::read_annotations(&snapshot(request.clone(), c)?)?
        .iter()
        .any(|a| a.id == *id))
}

fn replay(
    store: &RevisionStore,
    context: &AnnotationContext,
    fingerprint: beamcraft_model::digest::ContentDigest,
) -> Result<Option<AnnotationChange>, PlateError> {
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let op = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op, || Ok(fingerprint))?;
    if let PriorOp::Accepted { .. } = prior {
        let Some((_, sequence, _)) = &recorded else {
            return Err(integrity("accepted annotation operation has no revision"));
        };
        let revision = store.open_at(*sequence)?;
        let evidence = crate::command_evidence::read(&revision)?
            .ok_or_else(|| integrity("annotation operation has no request evidence"))?;
        if evidence.fingerprint != fingerprint {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        let listed = storage::read_annotations(&snapshot(open_base(&base)?, &control())?)?;
        return Ok(Some(AnnotationChange {
            replayed: true,
            revision: replay_summary(&head, recorded)?,
            footprint: None,
            changed_layers: Vec::new(),
            annotations: listed,
        }));
    }
    Ok(None)
}

/// Create an annotation atomically, or replay the exact prior creation under the same
/// operation id. The placement pin is the head workspace generation at this moment.
pub fn create_annotation(
    store: &RevisionStore,
    context: &AnnotationContext,
    command: &CreateAnnotation,
    hook: &mut dyn CommitHook,
) -> Result<AnnotationChange, PlateError> {
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
    let model = storage::read_annotations(&prims)?;
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
    let footprint = beamcraft_engine::annotation::create(command, &model, &elements)?;
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
    let mut placed = command.clone();
    // The pins are the addressed bodies' full-mesh digests in the verified head: placement
    // needs every anchor live, and any later change of an addressed body marks the annotation
    // stale, so a renumbered feature address is never silently presented as the placed one.
    let inspection = inspect(store)?;
    placed.annotation.pins = command
        .annotation
        .payload
        .anchors()
        .map(|anchor| {
            let owner = beamcraft_engine::query::feature::FeatureId::parse(anchor.as_str())
                .map_err(|_| integrity("an admitted anchor stopped parsing during placement"))?
                .element();
            inspection
                .snapshot
                .element(&owner)
                .map(|entry| entry.full.mesh_digest)
                .ok_or_else(|| {
                    PlateError::from(CommandError::new(
                        ErrorCode::NotFound,
                        "feature",
                        "the anchored element is not in the head revision",
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::Create(Element {
                id: placed.annotation.id.to_string(),
                name: placed.annotation.name.clone(),
                shape: Shape::Annotation(placed.annotation.clone()),
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
        return Err(integrity("unseen annotation creation reported as replayed"));
    }
    Ok(AnnotationChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        annotations: annotations(store)?,
    })
}

/// One resolved annotation of the head revision: the stored record and its binding state.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationReport {
    /// The stored annotation.
    pub annotation: Annotation,
    /// The engine's binding decision against the pinned view snapshot.
    pub status: beamcraft_engine::annotation::AnnotationStatus,
}

/// The text of an `AttrValue`, the form a mark shows.
fn value_text(value: &AttrValue) -> String {
    match value {
        AttrValue::Text(text) | AttrValue::Choice(text) => text.clone(),
        AttrValue::Number(v) => format!("{v}"),
        AttrValue::Integer(v) => format!("{v}"),
        AttrValue::Boolean(v) => format!("{v}"),
    }
}

/// Resolves every annotation of the verified head against its pinned view snapshot, with mark
/// text from the anchored elements' user attributes (I04).
pub fn status(store: &RevisionStore) -> Result<Vec<AnnotationReport>, PlateError> {
    let inspection = inspect(store)?;
    let model = attr_model(store)?;
    let text = |id: &ElementId, name: &str| model.values.value_of(id, name).map(value_text);
    let statuses =
        beamcraft_engine::annotation::resolve(&inspection.snapshot, &inspection.annotations, &text);
    Ok(inspection
        .annotations
        .iter()
        .cloned()
        .zip(statuses)
        .map(|(annotation, status)| AnnotationReport { annotation, status })
        .collect())
}

/// The payload kind token of a stored record.
pub fn payload_of(annotation: &Annotation) -> &'static str {
    annotation.payload.as_str()
}
