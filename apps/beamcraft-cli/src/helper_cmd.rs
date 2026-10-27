// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 headless commands: `create-helper` and `helpers`. They call the engine API through
//! `beamcraft_cli::helper` and print one JSON object with the same error shape and exit codes as
//! the plate commands. Helpers are separate construction objects.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{check_project_option, footprint_json, id_error, num, revision_json};
use beamcraft_cli::helper::{HelperChange, HelperContext, create_helper, read_helpers, views};
use beamcraft_cli::plate::PlateError;
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::helper::{Helper, HelperGeometry};
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

/// Whether `command` is one of the helper commands.
pub fn handles(command: &OsString) -> bool {
    command == "create-helper" || command == "helpers" || command == "snap-helper"
}

/// Runs one helper command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
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
        "create-helper" => &["--store", "--name", "--kind", "--parameters", "--new-id"],
        "helpers" => &["--store"],
        "snap-helper" => &["--store", "--point", "--tolerance"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed =
            own.contains(key) || (*command == "create-helper" && CONTEXT_OPTIONS.contains(key));
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
    if *command == "helpers" || *command == "snap-helper" {
        let (revision, model) = read_helpers(&store)?;
        let pin = revision.as_ref().map(revision_json).unwrap_or(Json::Null);
        if *command == "helpers" {
            return Ok(Json::object([
                ("outcome", Json::text("helpers")),
                ("helpers", views_json(&views(&model))?),
                ("revision", pin),
            ]));
        }
        let point = options
            .get("--point")
            .ok_or_else(|| invalid("--point is required"))?;
        let point = beamcraft_engine::helper::parse_point(point)?;
        let tolerance = options
            .get("--tolerance")
            .ok_or_else(|| invalid("--tolerance is required"))?
            .parse::<f64>()
            .map_err(|_| invalid("malformed tolerance"))?;
        let snap = model.snap(point, tolerance)?;
        let snap = match snap {
            Some(s) => Json::object([
                ("id", Json::text(s.id.to_string())),
                ("role", Json::text(s.role)),
                (
                    "point_mm",
                    Json::Array(s.point_mm.into_iter().map(num).collect::<Run<Vec<_>>>()?),
                ),
                ("distance_mm", num(s.distance_mm)?),
            ]),
            None => Json::Null,
        };
        return Ok(Json::object([
            ("outcome", Json::text("helper_snap")),
            ("snap", snap),
            ("revision", pin),
        ]));
    }
    let _claim = check_project_option(&store, &options)?;
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let common = command_context::parse(&options, &mut fresh)?;
    let context = HelperContext {
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
    let command = Helper {
        id: new_id,
        name: given("--name")
            .ok_or_else(|| invalid("--name is required"))?
            .to_owned(),
        geometry: geometry(&options)?,
    };
    let change = create_helper(&store, &context, &command, &mut NoFault)?;
    change_json(&command, context.op_id, &change)
}

fn geometry(options: &BTreeMap<&str, &str>) -> Run<HelperGeometry> {
    let kind = options
        .get("--kind")
        .ok_or_else(|| invalid("--kind is required"))?;
    let parameters = options
        .get("--parameters")
        .ok_or_else(|| invalid("--parameters is required"))?;
    if parameters.len() > 256 || parameters.split(',').count() > 7 {
        return Err(invalid("helper parameters exceed the input bound"));
    }
    let values = parameters
        .split(',')
        .map(|v| {
            v.trim()
                .parse::<f64>()
                .map_err(|_| invalid("malformed helper parameter"))
        })
        .collect::<Run<Vec<_>>>()?;
    HelperGeometry::from_parameters(kind, &values).map_err(Into::into)
}

/// Explicit helper records in stable identity order.
pub(crate) fn views_json(views: &[Helper]) -> Run<Json> {
    Ok(Json::Array(
        views
            .iter()
            .map(|v| {
                Ok(Json::object([
                    ("id", Json::text(v.id.to_string())),
                    ("name", Json::text(v.name.clone())),
                    ("kind", Json::text(v.geometry.kind())),
                    (
                        "parameters",
                        Json::Array(
                            v.geometry
                                .parameters()
                                .into_iter()
                                .map(num)
                                .collect::<Run<Vec<_>>>()?,
                        ),
                    ),
                ]))
            })
            .collect::<Run<Vec<_>>>()?,
    ))
}

fn change_json(subject: &Helper, op_id: OpId, change: &HelperChange) -> Run<Json> {
    let outcome = if change.replayed {
        "helper_replayed"
    } else {
        "helper_created"
    };
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.id.to_string())),
        (
            "helper",
            Json::object([
                ("element_id", Json::text(subject.id.to_string())),
                ("name", Json::text(subject.name.clone())),
            ]),
        ),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("helpers", views_json(&change.helpers)?),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ]))
}
