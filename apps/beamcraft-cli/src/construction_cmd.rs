// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 headless commands: `create-construction` and `list-construction`. They call the engine
//! API through `beamcraft_cli::construction` and print one JSON object with the same error
//! shape and exit codes as the grid and level commands.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{
    check_project_option, footprint_json, id_error, num, revision_json, triple,
};
use beamcraft_cli::construction::{
    ConstructionChange, ConstructionContext, constructions, create_construction,
};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::command::CreateConstructionObject;
use beamcraft_model::construction::ConstructionGeometry;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use session::Refusal;
use std::collections::BTreeMap;
use std::ffi::OsString;

type Run<T> = Result<T, Refusal>;

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs one construction object command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

/// One world coordinate triple from `X,Y,Z`.
fn coordinates(text: &str) -> Run<[f64; 3]> {
    let numbers = text
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.parse::<f64>()
                .map_err(|_| invalid("a number is malformed"))
        })
        .collect::<Run<Vec<_>>>()?;
    match numbers.as_slice() {
        [x, y, z] => Ok([*x, *y, *z]),
        _ => Err(invalid("--a and --b take three coordinates X,Y,Z")),
    }
}

fn run(args: &[OsString]) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    let own: &[&str] = match *command {
        "create-construction" => &[
            "--store", "--name", "--kind", "--a", "--b", "--radius", "--new-id",
        ],
        "list-construction" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed =
            own.contains(key) || (*command != "list-construction" && CONTEXT_OPTIONS.contains(key));
        if !allowed || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(store_root).map_err(|e| Refusal {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    if *command == "list-construction" {
        let found = constructions(&store)?;
        return Ok(Json::object([
            ("outcome", Json::text("construction_objects")),
            ("objects", objects_json(&found)?),
        ]));
    }
    let _claim = check_project_option(&store, &options)?;
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| Refusal {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let common = command_context::parse(&options, &mut fresh)?;
    let context = ConstructionContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    let new_id = match given("--new-id") {
        Some(text) => ElementId::parse(text).map_err(id_error)?,
        None => ElementId::from_uuid(fresh()?),
    };
    let a = coordinates(given("--a").ok_or_else(|| invalid("--a is required"))?)?;
    let b = match given("--b") {
        Some(text) => Some(coordinates(text)?),
        None => None,
    };
    let radius = match given("--radius") {
        Some(text) => Some(
            text.parse::<f64>()
                .map_err(|_| invalid("a number is malformed"))?,
        ),
        None => None,
    };
    let geometry = match given("--kind").ok_or_else(|| invalid("--kind is required"))? {
        "point" => {
            if b.is_some() || radius.is_some() {
                return Err(invalid("a point takes no --b or --radius"));
            }
            ConstructionGeometry::Point { point_mm: a }
        }
        "line" => ConstructionGeometry::Line {
            start_mm: a,
            end_mm: b.ok_or_else(|| invalid("a line needs --b"))?,
        },
        "circle" => ConstructionGeometry::Circle {
            centre_mm: a,
            axis: b.ok_or_else(|| invalid("a circle needs --b"))?,
            radius_mm: radius.ok_or_else(|| invalid("a circle needs --radius"))?,
        },
        "plane" => {
            if radius.is_some() {
                return Err(invalid("a plane takes no --radius"));
            }
            ConstructionGeometry::Plane {
                point_mm: a,
                normal: b.ok_or_else(|| invalid("a plane needs --b"))?,
            }
        }
        _ => return Err(invalid("--kind is one of point, line, circle or plane")),
    };
    let command = CreateConstructionObject {
        object: beamcraft_model::construction::ConstructionObject {
            id: new_id,
            name: given("--name")
                .ok_or_else(|| invalid("--name is required"))?
                .to_owned(),
            geometry,
        },
    };
    let change = create_construction(&store, &context, &command, &mut NoFault)?;
    change_json(new_id, context.op_id, &change)
}

/// The construction objects as JSON, in the order given (ascending identity).
pub(crate) fn objects_json(
    objects: &[beamcraft_model::construction::ConstructionObject],
) -> Run<Json> {
    Ok(Json::Array(
        objects
            .iter()
            .map(|o| {
                let (a, b, radius) = match o.geometry {
                    ConstructionGeometry::Point { point_mm } => (point_mm, None, None),
                    ConstructionGeometry::Line { start_mm, end_mm } => {
                        (start_mm, Some(end_mm), None)
                    }
                    ConstructionGeometry::Circle {
                        centre_mm,
                        axis,
                        radius_mm,
                    } => (centre_mm, Some(axis), Some(radius_mm)),
                    ConstructionGeometry::Plane { point_mm, normal } => {
                        (point_mm, Some(normal), None)
                    }
                };
                let b_json = match b {
                    Some(b) => triple(b)?,
                    None => Json::Null,
                };
                let radius_json = match radius.map(num) {
                    Some(r) => r?,
                    None => Json::Null,
                };
                Ok(Json::object([
                    ("id", Json::text(o.id.to_string())),
                    ("name", Json::text(o.name.clone())),
                    ("kind", Json::text(o.geometry.as_str())),
                    ("a_mm", triple(a)?),
                    ("b_mm", b_json),
                    ("radius_mm", radius_json),
                ]))
            })
            .collect::<Run<Vec<_>>>()?,
    ))
}

fn change_json(subject: ElementId, op_id: OpId, change: &ConstructionChange) -> Run<Json> {
    let outcome = if change.replayed {
        "construction_created_replayed"
    } else {
        "construction_created"
    };
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("objects", objects_json(&change.constructions)?),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ]))
}
