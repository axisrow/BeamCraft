// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V15 typed clip state in the W01 personal document.
use crate::{plate::PlateError, saved_views::Views};
use beamcraft_engine::view_clipping::{ClipPlane, Clipping};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{error::ErrorCode, id::ElementId, saved_view::SavedView};
fn invalid() -> PlateError {
    PlateError::new(
        ErrorCode::InvalidArgument,
        "invalid personal clipping state",
    )
}
fn key(id: ElementId) -> String {
    format!("clip_{}", id.to_string().replace('-', ""))
}
fn point(j: &Json) -> Result<[f64; 3], PlateError> {
    j.array()
        .map_err(|_| invalid())?
        .iter()
        .map(|v| v.float().map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| invalid())
}
/// JSON plane parameters with the positive half-space convention.
pub fn planes_json(planes: &[ClipPlane]) -> Result<Json, PlateError> {
    let point = |p: [f64; 3]| -> Result<Json, PlateError> {
        Ok(Json::Array(
            p.into_iter()
                .map(|v| Json::number(v).map_err(|_| invalid()))
                .collect::<Result<Vec<_>, _>>()?,
        ))
    };
    Ok(Json::Array(
        planes
            .iter()
            .map(|p| {
                Ok(Json::object([
                    ("point_mm", point(p.point_mm)?),
                    ("normal", point(p.normal)?),
                ]))
            })
            .collect::<Result<Vec<_>, PlateError>>()?,
    ))
}
/// Decode a capped stored clip setting; absence retains constructed-view depth by default.
pub fn read(views: &Views, id: ElementId) -> Result<Clipping, PlateError> {
    let Some(s) = views.document.records.get(&key(id)) else {
        return Ok(Clipping::default());
    };
    let j = beamcraft_json::decode(s.as_bytes(), &Limits::DEFAULT).map_err(|_| invalid())?;
    j.closed(&["planes", "depth_enabled"])
        .map_err(|_| invalid())?;
    let planes = j
        .get("planes")
        .and_then(Json::array)
        .map_err(|_| invalid())?;
    if planes.len() > beamcraft_engine::view_clipping::MAX_PLANES {
        return Err(invalid());
    }
    let planes = planes
        .iter()
        .map(|p| -> Result<ClipPlane, PlateError> {
            p.closed(&["point_mm", "normal"]).map_err(|_| invalid())?;
            let origin = point(p.get("point_mm").map_err(|_| invalid())?)?;
            let normal = point(p.get("normal").map_err(|_| invalid())?)?;
            Ok(ClipPlane::new(origin, normal)?)
        })
        .collect::<Result<Vec<_>, PlateError>>()?;
    let c = Clipping {
        planes,
        depth_enabled: j
            .get("depth_enabled")
            .and_then(Json::boolean)
            .map_err(|_| invalid())?,
    };
    c.validate()?;
    Ok(c)
}
/// Replace one admitted setting in memory; the caller publishes the personal file with CAS.
pub fn set(views: &mut Views, id: ElementId, c: Clipping) -> Result<(), PlateError> {
    if !views.document.views.iter().any(|v| v.id == id) {
        return Err(PlateError::new(ErrorCode::NotFound, "view not found"));
    }
    c.validate()?;
    let j = Json::object([
        ("planes", planes_json(&c.planes)?),
        ("depth_enabled", Json::Bool(c.depth_enabled)),
    ]);
    let s = String::from_utf8(j.encode(&Limits::DEFAULT).map_err(|_| invalid())?)
        .map_err(|_| invalid())?;
    views.document.records.insert(key(id), s);
    Ok(())
}
/// Expand stored clipping using the resolved camera's current work plane and saved depth.
pub fn effective(views: &Views, view: &SavedView) -> Result<Vec<ClipPlane>, PlateError> {
    let c = read(views, view.id)?;
    let depth = views
        .document
        .records
        .get(&crate::plane_views::key(view.id))
        .map(|s| crate::plane_views::decode(s).map(|p| (view.work_plane, p.depth_mm)))
        .transpose()?;
    Ok(c.effective(crate::plane_views::work_area(views, view.id)?, depth)?)
}
