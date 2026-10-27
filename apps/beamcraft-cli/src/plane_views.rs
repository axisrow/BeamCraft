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
    p.resolve()?;
    let source = match p.source {
        PlaneSource::Plane(plane) => Json::object([
            ("kind", Json::text("plane")),
            ("plane", Json::text(plane.expression())),
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
    p.resolve()?;
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
    let (camera, plane) = p.resolve()?;
    let view = saved_views::capture(
        dir,
        id,
        name,
        camera,
        [plane.origin(), plane.x(), plane.y()],
        comment,
    )?;
    views.document.records.insert(key(id), encode(p)?);
    views.document.views.push(view.clone());
    views.document.views.sort_by_key(|v| v.id);
    Ok(view)
}
/// Resolve saved construction into transient camera state without writing a personal or model file.
pub fn resolve(views: &Views, view: &SavedView) -> Result<SavedView, PlateError> {
    let Some(record) = views.document.records.get(&key(view.id)) else {
        return Ok(view.clone());
    };
    let p = decode(record)?;
    let (camera, plane) = p.resolve()?;
    let mut view = view.clone();
    view.camera = camera;
    view.work_plane = [plane.origin(), plane.x(), plane.y()];
    Ok(view)
}
