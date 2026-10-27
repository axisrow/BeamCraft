// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Read-only V08 work-plane construction, including a real face pick.
use crate::plate_cmd::{invalid, triple};
use beamcraft_cli::plate::{PlateError, inspect};
use beamcraft_engine::work_plane::{WorkPlane, pick_work_plane};
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use std::{collections::BTreeMap, ffi::OsString};

/// Construct a session plane without writing to the model.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}
fn run(args: &[OsString]) -> Result<Json, PlateError> {
    let args = args
        .iter()
        .map(|s| {
            s.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (_, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 14 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for [key, value] in tail.as_chunks::<2>().0 {
        if ![
            "--plane",
            "--parallel",
            "--offset",
            "--points",
            "--vertical",
            "--store",
            "--origin",
            "--direction",
        ]
        .contains(key)
            || options.insert(*key, *value).is_some()
        {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let modes = ["--plane", "--parallel", "--points", "--vertical", "--store"]
        .iter()
        .filter(|k| options.contains_key(**k))
        .count();
    if modes != 1 {
        return Err(invalid("choose one work-plane construction mode"));
    }
    let get = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| invalid("missing plane option"))
    };
    let numbers = |text: &str| -> Result<Vec<f64>, PlateError> {
        if text.len() > 384 {
            return Err(invalid("plane text exceeds limit"));
        }
        text.split(',')
            .take(10)
            .map(|v| {
                v.trim()
                    .parse::<f64>()
                    .map_err(|_| invalid("malformed plane number"))
            })
            .collect()
    };
    if options.contains_key("--offset") && !options.contains_key("--parallel") {
        return Err(invalid("offset requires parallel mode"));
    }
    if ["--origin", "--direction"]
        .iter()
        .any(|k| options.contains_key(k))
        && !options.contains_key("--store")
    {
        return Err(invalid("ray options require store mode"));
    }
    let plane = if let Some(text) = options.get("--plane") {
        WorkPlane::parse(text)?
    } else if let Some(name) = options.get("--parallel") {
        let offset = options.get("--offset").map_or(Ok(0.), |v| {
            v.parse::<f64>().map_err(|_| invalid("malformed offset"))
        })?;
        WorkPlane::parallel(name, offset)?
    } else if let Some(text) = options.get("--points") {
        let values = numbers(text)?;
        let [a, b, c, d, e, f, g, h, i] = values.as_slice() else {
            return Err(invalid("three points need nine numbers"));
        };
        WorkPlane::three_points([*a, *b, *c], [*d, *e, *f], [*g, *h, *i])?
    } else if let Some(text) = options.get("--vertical") {
        let values = numbers(text)?;
        let [a, b, c, d, e, f] = values.as_slice() else {
            return Err(invalid("two points need six numbers"));
        };
        WorkPlane::vertical([*a, *b, *c], [*d, *e, *f])?
    } else {
        let blobs = BlobStore::new(get("--store")?).map_err(|e| PlateError {
            code: e.code(),
            rule_id: None,
            message: "store root is not usable",
        })?;
        let inspection = inspect(&RevisionStore::new(blobs))?;
        let origin = crate::plate_cmd::translation(get("--origin")?)?;
        let direction = crate::plate_cmd::translation(get("--direction")?)?;
        let Some(plane) = pick_work_plane(&inspection.snapshot, origin, direction)? else {
            return Ok(Json::object([
                ("outcome", Json::text("no_face")),
                ("plane", Json::Null),
            ]));
        };
        plane
    };
    Ok(Json::object([
        ("outcome", Json::text("work_plane")),
        ("plane", Json::text(plane.expression())),
        ("origin_mm", triple(plane.origin())?),
        ("x", triple(plane.x())?),
        ("y", triple(plane.y())?),
        ("normal", triple(plane.normal())?),
    ]))
}
