// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V15 clipping CRUD for one personally saved view.
use beamcraft_cli::{plane_views, plate::PlateError, saved_views, view_clipping};
use beamcraft_engine::view_clipping::{ClipPlane, Clipping, MAX_PLANES};
use beamcraft_json::Json;
use beamcraft_model::{error::ErrorCode, id::ElementId};
use std::{collections::BTreeMap, ffi::OsString, path::Path};
fn invalid(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
        rule_id: None,
    }
}
pub fn handles(c: &OsString) -> bool {
    c == "set-view-clipping" || c == "clear-view-clipping"
}
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}
fn run(args: &[OsString]) -> Result<Json, PlateError> {
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 20 {
        return Err(invalid("options must be bounded pairs"));
    }
    let mut options = BTreeMap::new();
    for [k, v] in tail.as_chunks::<2>().0 {
        let k = k.to_str().ok_or_else(|| invalid("invalid option"))?;
        let v = v
            .to_str()
            .filter(|s| s.len() <= 4096)
            .ok_or_else(|| invalid("argument must be bounded UTF-8"))?;
        if !(matches!(k, "--workspace-dir" | "--id" | "--expected-views")
            || (command == "set-view-clipping"
                && matches!(k, "--planes" | "--work-area" | "--depth-enabled")))
            || options.insert(k, v).is_some()
        {
            return Err(invalid("unknown or repeated clipping option"));
        }
    }
    let given = |k: &str| options.get(k).copied();
    let dir = Path::new(given("--workspace-dir").ok_or_else(|| invalid("workspace is required"))?);
    let id = ElementId::parse(given("--id").ok_or_else(|| invalid("view id is required"))?)
        .map_err(crate::plate_cmd::id_error)?;
    let mut views = saved_views::read(dir)?;
    if let Some(expected) = given("--expected-views")
        && expected
            != views
                .version
                .map_or_else(|| "none".into(), |d| d.to_string())
    {
        return Err(PlateError {
            code: ErrorCode::StaleWorkspace,
            message: "personal views version differs",
            rule_id: None,
        });
    }
    let mut clipping = if command == "clear-view-clipping" {
        Clipping {
            planes: Vec::new(),
            depth_enabled: false,
        }
    } else {
        view_clipping::read(&views, id)?
    };
    if command == "clear-view-clipping" {
        plane_views::set_work_area(&mut views, id, None)?;
    } else {
        if given("--planes").is_none()
            && given("--work-area").is_none()
            && given("--depth-enabled").is_none()
        {
            return Err(invalid("at least one clipping property is required"));
        }
        if let Some(s) = given("--planes") {
            clipping.planes = if s == "none" {
                Vec::new()
            } else {
                let p = s
                    .split(';')
                    .take(MAX_PLANES + 1)
                    .map(|s| {
                        let values = s
                            .split(',')
                            .take(7)
                            .map(|v| {
                                v.parse::<f64>()
                                    .map_err(|_| invalid("plane needs six numbers"))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let [a, b, c, x, y, z] = values.as_slice() else {
                            return Err(invalid("plane needs six numbers"));
                        };
                        Ok(ClipPlane::new([*a, *b, *c], [*x, *y, *z])?)
                    })
                    .collect::<Result<Vec<_>, PlateError>>()?;
                if p.len() > MAX_PLANES {
                    return Err(invalid("too many clip planes"));
                }
                p
            };
        }
        if let Some(s) = given("--depth-enabled") {
            clipping.depth_enabled = match s {
                "true" => true,
                "false" => false,
                _ => return Err(invalid("depth-enabled must be true or false")),
            };
            if clipping.depth_enabled && !views.document.records.contains_key(&plane_views::key(id))
            {
                return Err(invalid("depth requires a constructed plane view"));
            }
        }
        if let Some(s) = given("--work-area") {
            plane_views::set_work_area(&mut views, id, crate::plane_view_cmd::parse_area(s)?)?;
        }
    }
    view_clipping::set(&mut views, id, clipping.clone())?;
    let version = saved_views::write(dir, &views)?;
    Ok(Json::object([
        ("outcome", Json::text("view_clipping_saved")),
        ("version", Json::text(version.to_string())),
        ("planes", view_clipping::planes_json(&clipping.planes)?),
        ("depth_enabled", Json::Bool(clipping.depth_enabled)),
    ]))
}
