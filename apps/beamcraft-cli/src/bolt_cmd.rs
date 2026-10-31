// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A05 headless commands: `create-bolt` and `bolts`. They call the engine API through
//! `beamcraft_cli::bolt` and print one JSON object with the same error shape and exit codes as
//! the weld commands.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{footprint_json, id_error, revision_json};
use beamcraft_cli::bolt::{BoltChange, BoltContext, BoltView, bolts, create_bolt};
use beamcraft_cli::plate::PlateError;
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::command::{BoltSpec, CreateBolt};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use std::collections::BTreeMap;
use std::ffi::OsString;

type Run<T> = Result<T, PlateError>;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs one bolt command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

/// Parses `X,Y,Z` as three finite numbers of millimetres.
fn vec3(text: &str) -> Run<[f64; 3]> {
    let parts: Option<Vec<f64>> = text
        .split(',')
        .map(|p| p.trim().parse::<f64>().ok())
        .collect();
    let Some(values) = parts else {
        return Err(invalid("the vector is three comma-separated numbers"));
    };
    let v: [f64; 3] = <[f64; 3]>::try_from(values.as_slice())
        .map_err(|_| invalid("the vector is three comma-separated numbers"))?;
    if v.iter().any(|n| !n.is_finite()) {
        return Err(PlateError {
            rule_id: None,
            code: ErrorCode::NonFiniteNumber,
            message: "the vector is three comma-separated numbers",
        });
    }
    Ok(v)
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
        "create-bolt" => &[
            "--store",
            "--name",
            "--new-id",
            "--main",
            "--secondary",
            "--diameter",
            "--count",
            "--pitch",
            "--origin",
            "--direction",
        ],
        "bolts" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed = own.contains(key) || (*command != "bolts" && CONTEXT_OPTIONS.contains(key));
        if !allowed || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(store_root).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let _claim = crate::plate_cmd::check_project_option(&store, &options)?;
    if *command == "bolts" {
        return Ok(Json::object([
            ("outcome", Json::text("bolts")),
            ("bolts", views_json(&bolts(&store)?)),
        ]));
    }
    let given = |key: &str| options.get(key).copied();
    let required = |key: &str, missing: &'static str| given(key).ok_or_else(|| invalid(missing));
    let number = |key: &'static str, missing: &'static str| -> Run<f64> {
        required(key, missing)?
            .parse::<f64>()
            .map_err(|_| invalid("the value is a number"))
    };
    // Every bolt property is authored explicitly: no option has a default.
    let diameter = number("--diameter", "--diameter is required")?;
    let pitch = number("--pitch", "--pitch is required")?;
    let count: usize = required("--count", "--count is required")?
        .parse::<usize>()
        .map_err(|_| invalid("--count is a whole number of bolts"))?;
    let origin = vec3(required("--origin", "--origin is required")?)?;
    let direction = vec3(required("--direction", "--direction is required")?)?;
    // The model stores a unit direction: normalize here, refuse zero as degenerate.
    let squared = direction.iter().map(|v| v * v).sum::<f64>();
    if squared <= 0.0 {
        return Err(invalid("--direction is a non-zero vector"));
    }
    let norm = squared.sqrt();
    let bolt = BoltSpec {
        diameter_mm: diameter,
        count,
        pitch_mm: pitch,
        origin_mm: origin,
        direction_mm: [
            direction[0] / norm,
            direction[1] / norm,
            direction[2] / norm,
        ],
    };
    let element = |key: &str, missing: &'static str| -> Run<ElementId> {
        ElementId::parse(required(key, missing)?).map_err(id_error)
    };
    let (main_id, secondary_id) = (
        element("--main", "--main is required")?,
        element("--secondary", "--secondary is required")?,
    );
    let name = required("--name", "--name is required")?.to_owned();
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let common = command_context::parse(&options, &mut fresh)?;
    let context = BoltContext {
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
    let command = CreateBolt {
        new_id,
        name,
        main_id,
        secondary_id,
        bolt,
    };
    let change = create_bolt(&store, &context, &command, &mut NoFault)?;
    Ok(change_json(new_id, context.op_id, &change))
}

fn views_json(views: &[BoltView]) -> Json {
    Json::Array(
        views
            .iter()
            .map(|v| {
                Json::object([
                    ("id", Json::text(v.id.to_string())),
                    ("name", Json::text(v.name.clone())),
                    ("main", Json::text(v.main.to_string())),
                    ("secondary", Json::text(v.secondary.to_string())),
                    ("diameter_mm", Json::text(v.spec.diameter_mm.to_string())),
                    ("count", Json::text(v.spec.count.to_string())),
                    ("pitch_mm", Json::text(v.spec.pitch_mm.to_string())),
                    (
                        "origin_mm",
                        Json::Array(
                            v.spec
                                .origin_mm
                                .iter()
                                .map(|n| Json::text(n.to_string()))
                                .collect(),
                        ),
                    ),
                    (
                        "direction_mm",
                        Json::Array(
                            v.spec
                                .direction_mm
                                .iter()
                                .map(|n| Json::text(n.to_string()))
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    )
}

fn change_json(subject: ElementId, op_id: OpId, change: &BoltChange) -> Json {
    let outcome = if change.replayed {
        "bolt_created_replayed"
    } else {
        "bolt_created"
    };
    Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("bolts", views_json(&change.bolts)),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ])
}
