// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V09 CLI camera construction through engine builders and the W01 personal store.
use beamcraft_cli::{plane_views, plate::PlateError, saved_views};
use beamcraft_engine::{
    plane_view::{PlaneSource, PlaneView},
    work_plane::WorkPlane,
};
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
    c == "create-view"
}
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}
fn run(args: &[OsString]) -> Result<Json, PlateError> {
    let tail = args.get(1..).ok_or_else(|| invalid("missing arguments"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 36 {
        return Err(invalid("options must be bounded KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [k, v] = pair;
        let k = k.to_str().ok_or_else(|| invalid("invalid option"))?;
        let v = v
            .to_str()
            .filter(|s| s.len() <= 4096)
            .ok_or_else(|| invalid("argument must be bounded UTF-8"))?;
        if !matches!(
            k,
            "--workspace-dir"
                | "--kind"
                | "--name"
                | "--id"
                | "--comment"
                | "--plane"
                | "--start"
                | "--end"
                | "--side"
                | "--depth"
                | "--width"
                | "--aspect"
                | "--expected-views"
        ) || options.insert(k, v).is_some()
        {
            return Err(invalid("unknown or repeated view option"));
        }
    }
    let given = |k: &str| options.get(k).copied();
    let need = |k: &str| given(k).ok_or_else(|| invalid("required construction option is missing"));
    let dir = Path::new(need("--workspace-dir")?);
    let mut views = saved_views::read(dir)?;
    if let Some(expected) = given("--expected-views")
        && expected
            != views
                .version
                .map_or_else(|| "none".to_owned(), |d| d.to_string())
    {
        return Err(PlateError {
            code: ErrorCode::StaleWorkspace,
            message: "personal views version differs",
            rule_id: None,
        });
    }
    let coordinate_plane = given("--plane")
        .map(WorkPlane::parse)
        .transpose()?
        .unwrap_or_default();
    let source = match need("--kind")? {
        "plane" => {
            if given("--start").is_some() || given("--end").is_some() {
                return Err(invalid("plane view does not take two-point options"));
            }
            PlaneSource::Plane(coordinate_plane)
        }
        "two-points" => {
            let a = coordinate_plane.resolve_coordinate(
                need("--start")?,
                [0.; 3],
                beamcraft_engine::coordinate::AxisLock::None,
            )?;
            let b = coordinate_plane.resolve_coordinate(
                need("--end")?,
                a,
                beamcraft_engine::coordinate::AxisLock::None,
            )?;
            PlaneSource::TwoPoints(coordinate_plane.to_world(a)?, coordinate_plane.to_world(b)?)
        }
        _ => return Err(invalid("view kind must be plane or two-points")),
    };
    let number = |k: &str, default: f64| {
        given(k).map_or(Ok(default), |s| {
            s.parse::<f64>()
                .map_err(|_| invalid("view parameter must be numeric"))
        })
    };
    let p = PlaneView {
        source,
        negative_side: match given("--side") {
            None | Some("positive") => false,
            Some("negative") => true,
            _ => return Err(invalid("side must be positive or negative")),
        },
        depth_mm: number("--depth", 10000.)?,
        width_mm: number("--width", 10000.)?,
        aspect: number("--aspect", 1.)?,
    };
    let id = match given("--id") {
        Some(s) => ElementId::parse(s).map_err(crate::plate_cmd::id_error)?,
        None => ElementId::from_uuid(
            beamcraft_host::ids::IdSource::system()
                .next_id()
                .map_err(|_| invalid("view identity unavailable"))?,
        ),
    };
    let view = plane_views::create(
        dir,
        &mut views,
        id,
        need("--name")?.to_owned(),
        given("--comment").unwrap_or("").to_owned(),
        p,
    )?;
    let version = saved_views::write(dir, &views)?;
    Ok(Json::object([
        ("outcome", Json::text("view_created")),
        ("version", Json::text(version.to_string())),
        ("view", crate::saved_views_cmd::view_json(&view)?),
        (
            "construction",
            beamcraft_json::decode(
                plane_views::encode(p)?.as_bytes(),
                &beamcraft_json::Limits::DEFAULT,
            )
            .map_err(|_| invalid("construction encoding is invalid"))?,
        ),
    ]))
}
