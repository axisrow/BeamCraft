// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01 grid commands through the atomic revision pipeline and verified USD records.
use crate::assembly::read_prims;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, base_of, check_footprint,
    control, host_uuid, integrity, load_base, open_base, prior_op_with, replay_summary, summarize,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Prepare, Shape, grid as storage, schema_identity,
    validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Prim, Reader};
use beamcraft_model::command::{CommandError, CreateGrid};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::grid::Grid;
use beamcraft_model::id::ElementId;
use std::collections::{BTreeMap, BTreeSet};

/// Trusted context for a grid creation, using the same identities as level creation.
pub type GridContext = crate::level::LevelContext;

/// The committed or replayed result of a grid creation.
#[derive(Debug, Clone, PartialEq)]
pub struct GridChange {
    /// Whether the operation was already committed.
    pub replayed: bool,
    /// Revision holding the result.
    pub revision: RevisionSummary,
    /// Declared writes, absent on replay.
    pub footprint: Option<Footprint>,
    /// Layers whose bytes changed.
    pub changed_layers: Vec<String>,
    /// The verified grids at the resulting head.
    pub grids: Vec<Grid>,
}
fn snapshot(request: OpenRequest, c: &Control) -> Result<Vec<Prim>, PlateError> {
    validate_layers(request.clone(), schema_identity()?, c)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, c)?;
    read_prims(&reader, &opened.context, c)
}
/// Grids of the verified head, in identity order. Empty for a store with no revision.
pub fn grids(store: &RevisionStore) -> Result<Vec<Grid>, PlateError> {
    let revision = match store.open() {
        Ok(r) => r,
        Err(RevisionError::NoRevision) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(storage::read_grids(&snapshot(
        open_base(&base_of(&revision)?)?,
        &control(),
    )?)?)
}
/// Whether a verified snapshot has this owned grid identity.
pub(crate) fn is_grid(
    request: &OpenRequest,
    id: &ElementId,
    c: &Control,
) -> Result<bool, PlateError> {
    Ok(storage::read_grids(&snapshot(request.clone(), c)?)?
        .iter()
        .any(|g| g.id == *id))
}
fn replay(
    store: &RevisionStore,
    context: &GridContext,
    fingerprint: beamcraft_model::digest::ContentDigest,
) -> Result<Option<GridChange>, PlateError> {
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let op = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op, || Ok(fingerprint))?;
    if let PriorOp::Accepted { .. } = prior {
        let Some((_, sequence, _)) = &recorded else {
            return Err(integrity("accepted grid operation has no revision"));
        };
        let revision = store.open_at(*sequence)?;
        let evidence = crate::command_evidence::read(&revision)?
            .ok_or_else(|| integrity("grid operation has no request evidence"))?;
        if evidence.fingerprint != fingerprint {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        return Ok(Some(GridChange {
            replayed: true,
            revision: replay_summary(&head, recorded)?,
            footprint: None,
            changed_layers: Vec::new(),
            grids: storage::read_grids(&snapshot(open_base(&base)?, &control())?)?,
        }));
    }
    Ok(None)
}
/// Create a grid atomically, or replay the exact prior creation under the same operation id.
pub fn create_grid(
    store: &RevisionStore,
    context: &GridContext,
    command: &CreateGrid,
    hook: &mut dyn CommitHook,
) -> Result<GridChange, PlateError> {
    command.validate_arguments()?;
    let fingerprint = command.content_fingerprint()?;
    commit_grid(store, context, command, fingerprint, None, hook)
}
fn commit_grid(
    store: &RevisionStore,
    context: &GridContext,
    command: &CreateGrid,
    fingerprint: beamcraft_model::digest::ContentDigest,
    source: Option<&Grid>,
    hook: &mut dyn CommitHook,
) -> Result<GridChange, PlateError> {
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
    let model = storage::read_grids(&prims)?;
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
    let mut footprint = beamcraft_engine::grid::create(command, &model, &elements)?;
    if let Some(source) = source {
        if model.iter().find(|g| g.id == source.id) != Some(source) {
            return Err(CommandError::new(
                ErrorCode::BaseRevisionMismatch,
                "source_id",
                "source grid changed during copy",
            )
            .into());
        }
        let mut reads = footprint.reads().iter().copied().collect::<Vec<_>>();
        reads.extend(
            [
                beamcraft_engine::footprint::Group::Lifecycle,
                beamcraft_engine::footprint::Group::ShapeInputs,
                beamcraft_engine::footprint::Group::Placement,
                beamcraft_engine::footprint::Group::Label,
            ]
            .into_iter()
            .map(|group| beamcraft_engine::footprint::Entry {
                target: beamcraft_engine::footprint::Target::Element(source.id),
                group,
            }),
        );
        footprint = Footprint::from_entries(
            &reads,
            &footprint.writes().iter().copied().collect::<Vec<_>>(),
        )?;
    }
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
                id: command.grid.id.to_string(),
                name: command.grid.name.clone(),
                shape: Shape::Grid(command.grid.clone()),
                placement: storage::placement(&command.grid),
                material: None,
            })],
        },
        &c,
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
            revision_id: &context.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("unseen grid creation reported as replayed"));
    }
    Ok(GridChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        grids: grids(store)?,
    })
}

/// Copy one grid independently, preserving expressions and labels and replacing every identity.
pub fn copy_grid(
    store: &RevisionStore,
    context: &GridContext,
    request: &beamcraft_model::grid::CopyGrid,
    hook: &mut dyn CommitHook,
) -> Result<GridChange, PlateError> {
    let fingerprint = request.content_fingerprint()?;
    if let Some(replayed) = replay(store, context, fingerprint)? {
        return Ok(replayed);
    }
    let source = grids(store)?
        .into_iter()
        .find(|g| g.id == request.source_id)
        .ok_or(CommandError::new(
            ErrorCode::NotFound,
            "source_id",
            "source grid not found",
        ))?;
    let mut copy = source.clone();
    copy.id = request.new_id;
    if let Some(name) = &request.name {
        copy.name = name.clone();
    }
    let [dx, dy] = request.offset_mm;
    let [x, y] = copy.translation_mm;
    copy.translation_mm = [x + dx, y + dy];
    if request.new_axis_ids.len() != source.axis_ids().count() {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "new_axis_ids",
            "copy axis id count disagrees with the source",
        )
        .into());
    }
    let mut ids = request.new_axis_ids.iter().copied();
    for family in [&mut copy.u, &mut copy.v] {
        match family {
            beamcraft_model::grid::GridFamily::Regular(f) => {
                for id in &mut f.axis_ids {
                    *id = ids
                        .next()
                        .ok_or_else(|| integrity("copy axis id missing"))?;
                }
            }
            beamcraft_model::grid::GridFamily::Drawn(lines) => {
                for line in lines {
                    line.axis_id = ids
                        .next()
                        .ok_or_else(|| integrity("copy axis id missing"))?;
                }
            }
        }
    }
    copy.validate()?;
    commit_grid(
        store,
        context,
        &CreateGrid { grid: copy },
        fingerprint,
        Some(&source),
        hook,
    )
}

/// Apply one atomic line edit, with index-backed ownership and incoming-reference guards.
pub fn edit_grid(
    store: &RevisionStore,
    context: &GridContext,
    request: &beamcraft_model::grid_edit::GridLineEditRequest,
    hook: &mut dyn CommitHook,
) -> Result<GridChange, PlateError> {
    let fingerprint = request.content_fingerprint()?;
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
    let base_request = open_base(&base)?;
    let prims = snapshot(base_request.clone(), &c)?;
    let model = storage::read_grids(&prims)?;
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        base_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &c,
    )?;
    let index = author.element_index(&opened.context, &c)?;
    let id = beamcraft_layer_engine::index::ElementId::parse(&request.target_id.to_string())?;
    author.authorize_write(&index, &id, &c)?;
    let source = model
        .iter()
        .find(|g| g.id == request.target_id)
        .ok_or(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "target_id",
            "target is not a supported grid",
        ))?;
    let mut used = prims
        .iter()
        .filter_map(|p| p.element_id.as_deref())
        .map(ElementId::parse)
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|_| integrity("invalid element identity"))?;
    used.extend(model.iter().flat_map(|g| g.axis_ids()));
    let mut referenced = BTreeSet::new();
    if let beamcraft_model::grid_edit::GridLineEdit::Remove { axis_id } = request.edit
        && !index.delete_blockers(&id)?.is_empty()
    {
        referenced.insert(axis_id);
    }
    let edited = beamcraft_engine::grid_edit::edit(request, source, &used, &referenced)?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::Grid {
                id: request.target_id.to_string(),
                grid: edited.grid,
            }],
        },
        &c,
    )?;
    check_footprint(&prepared, &edited.footprint)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &edited.footprint,
            fingerprint,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &context.revision_id,
            op: host_uuid(&context.op_id.to_string())?,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("unseen grid edit reported as replayed"));
    }
    Ok(GridChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(edited.footprint),
        changed_layers,
        grids: grids(store)?,
    })
}
