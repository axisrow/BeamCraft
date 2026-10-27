// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Personal camera CRUD commands; all storage uses the typed W01 adapter.
use beamcraft_cli::{plate::PlateError, saved_views};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_json::Json;
use beamcraft_model::{
    error::ErrorCode,
    id::ElementId,
    saved_view::{Camera, Projection, SavedView},
};
use std::{collections::BTreeMap, ffi::OsString, path::Path};
fn invalid(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
        rule_id: None,
    }
}
pub fn handles(c: &OsString) -> bool {
    [
        "save-view",
        "list-views",
        "inspect-view",
        "update-view",
        "delete-view",
        "restore-view",
    ]
    .iter()
    .any(|s| c == s)
}
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}
pub(crate) fn view_json(v: &SavedView) -> Result<Json, PlateError> {
    let triple = |a: [f64; 3]| -> Result<Json, PlateError> {
        Ok(Json::Array(
            a.into_iter()
                .map(crate::plate_cmd::num)
                .collect::<Result<Vec<_>, _>>()?,
        ))
    };
    Ok(Json::object([
        ("id", Json::text(v.id.to_string())),
        ("name", Json::text(&v.name)),
        ("author", Json::text(v.author.to_string())),
        ("saved_ms", Json::uint(v.saved_ms)),
        ("comment", Json::text(&v.comment)),
        ("source_revision", Json::text(v.source_revision.to_string())),
        ("source_manifest", Json::text(v.source_manifest.to_string())),
        (
            "camera",
            Json::object([
                ("position_mm", triple(v.camera.placement.translation_mm)?),
                (
                    "orientation_xyzw",
                    Json::Array(
                        v.camera
                            .placement
                            .orientation_xyzw
                            .into_iter()
                            .map(crate::plate_cmd::num)
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                ),
                ("projection", Json::text(v.camera.projection.token())),
                ("width_mm", crate::plate_cmd::num(v.camera.width_mm)?),
                ("aspect", crate::plate_cmd::num(v.camera.aspect)?),
                ("fov_rad", crate::plate_cmd::num(v.camera.fov_rad)?),
            ]),
        ),
        (
            "work_plane",
            Json::Array(
                v.work_plane
                    .into_iter()
                    .map(triple)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        ),
    ]))
}
fn run(args: &[OsString]) -> Result<Json, PlateError> {
    let args = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument must be bounded UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options must be bounded KEY VALUE pairs"));
    }
    let editable = *command == "save-view" || *command == "update-view";
    let mut options = BTreeMap::new();
    for [key, value] in tail.as_chunks::<2>().0 {
        if !(matches!(*key, "--workspace-dir" | "--id" | "--expected-views")
            || (editable
                && matches!(
                    *key,
                    "--name"
                        | "--position"
                        | "--orientation-xyzw"
                        | "--projection"
                        | "--width"
                        | "--aspect"
                        | "--fov"
                        | "--plane"
                        | "--comment"
                )))
            || options.insert(*key, *value).is_some()
        {
            return Err(invalid("unknown or repeated view option"));
        }
    }
    let given = |k: &str| options.get(k).copied();
    let need = |k: &str| given(k).ok_or_else(|| invalid("required view option is missing"));
    let dir = Path::new(need("--workspace-dir")?);
    let mut views = saved_views::read(dir)?;
    if let Some(expected) = given("--expected-views") {
        let expected = if expected == "none" {
            None
        } else {
            Some(
                expected
                    .parse()
                    .map_err(|_| invalid("expected views version must be a full digest or none"))?,
            )
        };
        if expected != views.version {
            return Err(PlateError {
                code: ErrorCode::StaleWorkspace,
                message: "personal views version differs",
                rule_id: None,
            });
        }
    }
    let id = given("--id")
        .map(ElementId::parse)
        .transpose()
        .map_err(crate::plate_cmd::id_error)?;
    if *command == "list-views" {
        return Ok(Json::object([
            ("outcome", Json::text("views")),
            (
                "version",
                views
                    .version
                    .map_or(Json::Null, |d| Json::text(d.to_string())),
            ),
            (
                "views",
                Json::Array(
                    views
                        .document
                        .views
                        .iter()
                        .map(view_json)
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            ),
        ]));
    }
    if *command == "save-view" || *command == "update-view" {
        let id = match id {
            Some(id) => id,
            None if *command == "save-view" => ElementId::from_uuid(
                beamcraft_host::ids::IdSource::system()
                    .next_id()
                    .map_err(|_| invalid("view identity unavailable"))?,
            ),
            _ => return Err(invalid("--id is required")),
        };
        let old = views.document.views.iter().find(|v| v.id == id).cloned();
        if (*command == "save-view") == old.is_some() {
            return Err(PlateError {
                code: if old.is_some() {
                    ErrorCode::DuplicateId
                } else {
                    ErrorCode::NotFound
                },
                message: "view identity cannot be created or updated",
                rule_id: None,
            });
        }
        let mut camera = old
            .as_ref()
            .map_or_else(Camera::default, |v| v.camera.clone());
        if let Some(p) = given("--position") {
            camera.placement.translation_mm = crate::plate_cmd::translation(p)?;
        }
        if let Some(q) = given("--orientation-xyzw") {
            camera.placement.orientation_xyzw = crate::plate_cmd::quaternion(q)?;
        }
        if let Some(p) = given("--projection") {
            camera.projection =
                Projection::parse(p).ok_or_else(|| invalid("unknown camera projection"))?;
        }
        for (key, target) in [
            ("--width", &mut camera.width_mm),
            ("--aspect", &mut camera.aspect),
            ("--fov", &mut camera.fov_rad),
        ] {
            if let Some(v) = given(key) {
                *target = v
                    .parse()
                    .map_err(|_| invalid("camera parameter must be numeric"))?;
            }
        }
        let plane = match given("--plane") {
            Some(p) => {
                let p = WorkPlane::parse(p)?;
                [p.origin(), p.x(), p.y()]
            }
            None => old
                .as_ref()
                .map_or([[0.; 3], [1., 0., 0.], [0., 1., 0.]], |v| v.work_plane),
        };
        let name = given("--name")
            .map(str::to_owned)
            .or_else(|| old.as_ref().map(|v| v.name.clone()))
            .ok_or_else(|| invalid("--name is required"))?;
        let comment = given("--comment")
            .map(str::to_owned)
            .unwrap_or_else(|| old.as_ref().map_or_else(String::new, |v| v.comment.clone()));
        let view = saved_views::capture(dir, id, name, camera, plane, comment)?;
        views.document.views.retain(|v| v.id != id);
        views.document.views.push(view.clone());
        views.document.views.sort_by_key(|v| v.id);
        let version = saved_views::write(dir, &views)?;
        return Ok(Json::object([
            ("outcome", Json::text("view_saved")),
            ("version", Json::text(version.to_string())),
            ("view", view_json(&view)?),
        ]));
    }
    let id = id.ok_or_else(|| invalid("--id is required"))?;
    let view = views
        .document
        .views
        .iter()
        .find(|v| v.id == id)
        .ok_or(PlateError {
            code: ErrorCode::NotFound,
            message: "view not found",
            rule_id: None,
        })?
        .clone();
    if *command == "delete-view" {
        views.document.views.retain(|v| v.id != id);
        views
            .document
            .records
            .remove(&format!("view_{}", id.to_string().replace('-', "")));
        let version = saved_views::write(dir, &views)?;
        return Ok(Json::object([
            ("outcome", Json::text("view_deleted")),
            ("version", Json::text(version.to_string())),
        ]));
    }
    let changed = if *command == "restore-view" {
        saved_views::restore(dir, id)?.1
    } else {
        false
    };
    Ok(Json::object([
        (
            "outcome",
            Json::text(if *command == "restore-view" {
                "view_restored"
            } else {
                "view"
            }),
        ),
        (
            "version",
            views
                .version
                .map_or(Json::Null, |d| Json::text(d.to_string())),
        ),
        ("model_changed", Json::Bool(changed)),
        ("view", view_json(&view)?),
    ]))
}
