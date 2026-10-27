// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V09 construction intent persisted alongside W01 cameras in the same personal document.
use crate::{
    plate::PlateError,
    saved_views::{self, Views},
};
use beamcraft_engine::{
    plane_view::{PlaneSource, PlaneView},
    work_plane::WorkPlane,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::{id::ElementId, saved_view::SavedView};
use std::path::Path;
fn invalid() -> PlateError {
    PlateError::new(
        ErrorCode::InvalidArgument,
        "invalid saved plane-view construction",
    )
}
/// Stable personal record key for one saved view's construction.
pub fn key(id: ElementId) -> String {
    format!("view_{}", id.to_string().replace('-', ""))
}
fn num(v: f64) -> Result<Json, PlateError> {
    Json::number(v).map_err(|_| invalid())
}
fn point(p: [f64; 3]) -> Result<Json, PlateError> {
    Ok(Json::Array(
        p.into_iter().map(num).collect::<Result<Vec<_>, _>>()?,
    ))
}
/// Serialize admitted construction parameters, never model opinions.
pub fn encode(p: PlaneView) -> Result<String, PlateError> {
    p.validate()?;
    let source = match p.source {
        PlaneSource::Plane(plane) => Json::object([
            ("kind", Json::text("plane")),
            ("plane", Json::text(plane.expression())),
        ]),
        PlaneSource::Level(id) => Json::object([
            ("kind", Json::text("level")),
            ("level_id", Json::text(id.to_string())),
        ]),
        PlaneSource::GridLine { grid, axis } => Json::object([
            ("kind", Json::text("grid-line")),
            ("grid_id", Json::text(grid.to_string())),
            ("axis_id", Json::text(axis.to_string())),
        ]),
        PlaneSource::TwoPoints(a, b) => Json::object([
            ("kind", Json::text("two-points")),
            ("start", point(a)?),
            ("end", point(b)?),
        ]),
    };
    let json = Json::object([
        ("version", Json::uint(1)),
        ("source", source),
        ("negative_side", Json::Bool(p.negative_side)),
        ("depth_mm", num(p.depth_mm)?),
        ("width_mm", num(p.width_mm)?),
        ("aspect", num(p.aspect)?),
    ]);
    String::from_utf8(json.encode(&Limits::DEFAULT).map_err(|_| invalid())?).map_err(|_| invalid())
}
fn vector(j: &Json) -> Result<[f64; 3], PlateError> {
    j.array()
        .map_err(|_| invalid())?
        .iter()
        .map(|v| v.float().map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| invalid())
}
/// Read a closed typed construction record; unsupported future data is refused explicitly.
pub fn decode(s: &str) -> Result<PlaneView, PlateError> {
    let j = beamcraft_json::decode(s.as_bytes(), &Limits::DEFAULT).map_err(|_| invalid())?;
    j.closed(&[
        "version",
        "source",
        "negative_side",
        "depth_mm",
        "width_mm",
        "aspect",
    ])
    .map_err(|_| invalid())?;
    if j.get("version")
        .and_then(Json::unsigned)
        .map_err(|_| invalid())?
        != 1
    {
        return Err(invalid());
    }
    let source = j.get("source").map_err(|_| invalid())?;
    let kind = source
        .get("kind")
        .and_then(Json::string)
        .map_err(|_| invalid())?;
    let source = match kind {
        "plane" => {
            source.closed(&["kind", "plane"]).map_err(|_| invalid())?;
            PlaneSource::Plane(WorkPlane::parse(
                source
                    .get("plane")
                    .and_then(Json::string)
                    .map_err(|_| invalid())?,
            )?)
        }
        "two-points" => {
            source
                .closed(&["kind", "start", "end"])
                .map_err(|_| invalid())?;
            PlaneSource::TwoPoints(
                vector(source.get("start").map_err(|_| invalid())?)?,
                vector(source.get("end").map_err(|_| invalid())?)?,
            )
        }
        "level" => {
            source
                .closed(&["kind", "level_id"])
                .map_err(|_| invalid())?;
            PlaneSource::Level(
                ElementId::parse(
                    source
                        .get("level_id")
                        .and_then(Json::string)
                        .map_err(|_| invalid())?,
                )
                .map_err(|_| invalid())?,
            )
        }
        "grid-line" => {
            source
                .closed(&["kind", "grid_id", "axis_id"])
                .map_err(|_| invalid())?;
            PlaneSource::GridLine {
                grid: ElementId::parse(
                    source
                        .get("grid_id")
                        .and_then(Json::string)
                        .map_err(|_| invalid())?,
                )
                .map_err(|_| invalid())?,
                axis: ElementId::parse(
                    source
                        .get("axis_id")
                        .and_then(Json::string)
                        .map_err(|_| invalid())?,
                )
                .map_err(|_| invalid())?,
            }
        }
        _ => return Err(invalid()),
    };
    let p = PlaneView {
        source,
        negative_side: j
            .get("negative_side")
            .and_then(Json::boolean)
            .map_err(|_| invalid())?,
        depth_mm: j
            .get("depth_mm")
            .and_then(Json::float)
            .map_err(|_| invalid())?,
        width_mm: j
            .get("width_mm")
            .and_then(Json::float)
            .map_err(|_| invalid())?,
        aspect: j
            .get("aspect")
            .and_then(Json::float)
            .map_err(|_| invalid())?,
    };
    p.validate()?;
    Ok(p)
}
/// Add one constructed view to a previously read personal document; publication remains CAS.
pub fn create(
    dir: &Path,
    views: &mut Views,
    id: ElementId,
    name: String,
    comment: String,
    p: PlaneView,
) -> Result<SavedView, PlateError> {
    if views.document.views.iter().any(|v| v.id == id) {
        return Err(PlateError::new(
            ErrorCode::DuplicateId,
            "view identity already exists",
        ));
    }
    let model = current(dir)?;
    let (camera, plane) = p.resolve_with(&model.grids, &model.levels)?;
    let view = saved_views::capture(
        dir,
        id,
        name,
        camera,
        [plane.origin(), plane.x(), plane.y()],
        comment,
    )?;
    if view.source_manifest != model.revision.manifest_digest {
        return Err(PlateError::new(
            ErrorCode::StaleWorkspace,
            "model changed during view construction",
        ));
    }
    views.document.records.insert(key(id), encode(p)?);
    views.document.views.push(view.clone());
    views.document.views.sort_by_key(|v| v.id);
    Ok(view)
}
/// Resolve saved construction against one already verified model revision.
pub fn resolve_with(
    views: &Views,
    view: &SavedView,
    model: &crate::plate::Inspection,
) -> Result<SavedView, PlateError> {
    let Some(record) = views.document.records.get(&key(view.id)) else {
        return Ok(view.clone());
    };
    let p = decode(record)?;
    let (camera, plane) = p.resolve_with(&model.grids, &model.levels)?;
    let mut view = view.clone();
    view.camera = camera;
    view.work_plane = [plane.origin(), plane.x(), plane.y()];
    Ok(view)
}
/// A resolved camera, or an explicitly unresolved saved reference with no fallback claim.
#[derive(Debug, Clone)]
pub struct Restored {
    /// Resolved state, or stored provenance when unresolved is true.
    pub view: SavedView,
    /// Whether the source model changed.
    pub model_changed: bool,
    /// A required grid, axis or level no longer resolves.
    pub unresolved: bool,
}
/// Restore all model-dependent inputs from the same verified model context.
pub fn restore(dir: &Path, views: &Views, view: &SavedView) -> Result<Restored, PlateError> {
    let model = current(dir)?;
    let model_changed = view.source_manifest != model.revision.manifest_digest;
    let (view, unresolved) = match resolve_with(views, view, &model) {
        Ok(view) => (view, false),
        Err(e) if e.code == ErrorCode::NotFound => (view.clone(), true),
        Err(e) => return Err(e),
    };
    work_area(views, view.id)?;
    Ok(Restored {
        view,
        model_changed,
        unresolved,
    })
}
/// Read one verified model pin; inactive anchors are unavailable to view construction.
pub fn current(dir: &Path) -> Result<crate::plate::Inspection, PlateError> {
    let store = beamcraft_host::revision::RevisionStore::new(
        beamcraft_host::storage::BlobStore::new(dir)
            .map_err(|e| PlateError::new(e.code(), "workspace root is not usable"))?,
    );
    let head = store.open()?;
    let mut model = crate::plate::inspect_revision(&head)?;
    let mut reader = beamcraft_layer_engine::Reader::default();
    let control = crate::plate::control();
    let opened = reader.open(
        crate::plate::open_base(&crate::plate::base_of(&head)?)?,
        &control,
    )?;
    let prims = crate::assembly::read_prims(&reader, &opened.context, &control)?;
    let active = prims
        .iter()
        .filter(|p| {
            p.active
                && crate::plate::property(p, "bim:isDeactivated")
                    != Some(&beamcraft_layer_engine::Value::Bool(true))
        })
        .filter_map(|p| {
            p.element_id
                .as_deref()
                .and_then(|id| ElementId::parse(id).ok())
        })
        .collect::<std::collections::BTreeSet<_>>();
    model.grids.retain(|g| active.contains(&g.id));
    let mut levels = beamcraft_engine::level::Levels::default();
    for (id, record) in model.levels.levels() {
        if active.contains(id) {
            levels.insert_level(*id, record.clone());
        }
    }
    model.levels = levels;
    Ok(model)
}
fn area_key(id: ElementId) -> String {
    format!("area_{}", id.to_string().replace('-', ""))
}
/// Set or clear a finite work area in the existing personal document.
pub fn set_work_area(
    views: &mut Views,
    id: ElementId,
    area: Option<beamcraft_engine::plane_view::WorkArea>,
) -> Result<(), PlateError> {
    if !views.document.views.iter().any(|v| v.id == id) {
        return Err(PlateError::new(ErrorCode::NotFound, "view not found"));
    }
    if let Some(area) = area {
        area.validate()?;
        let json = Json::object([("min", point(area.min)?), ("max", point(area.max)?)]);
        let s = String::from_utf8(json.encode(&Limits::DEFAULT).map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
        views.document.records.insert(area_key(id), s);
    } else {
        views.document.records.remove(&area_key(id));
    }
    Ok(())
}
/// Admit a saved world work area before any projection consumes it.
pub fn work_area(
    views: &Views,
    id: ElementId,
) -> Result<Option<beamcraft_engine::plane_view::WorkArea>, PlateError> {
    let Some(s) = views.document.records.get(&area_key(id)) else {
        return Ok(None);
    };
    let json = beamcraft_json::decode(s.as_bytes(), &Limits::DEFAULT).map_err(|_| invalid())?;
    json.closed(&["min", "max"]).map_err(|_| invalid())?;
    let area = beamcraft_engine::plane_view::WorkArea {
        min: vector(json.get("min").map_err(|_| invalid())?)?,
        max: vector(json.get("max").map_err(|_| invalid())?)?,
    };
    area.validate()?;
    Ok(Some(area))
}
