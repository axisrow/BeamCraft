// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A09 headless commands: `number-assign`, `number-approve` and `marks`. They call the engine
//! API through `beamcraft_cli::marks` and print one JSON object with the same error shape
//! and exit codes as the plate commands.

use crate::plate_cmd::{footprint_json, id_error, revision_json};
use beamcraft_cli::marks::{MarkChange, MarkView, NumberingContext, approve, assign, marks};
use beamcraft_cli::plate::PlateError;
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_model::command::{ApproveMark, AssignMark, MarkStatus};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use beamcraft_protocol::json::Json;
use std::collections::BTreeMap;
use std::ffi::OsString;

type Run<T> = Result<T, PlateError>;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
    }
}

const CONTEXT_OPTIONS: [&str; 5] = [
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

/// Whether `command` is one of the numbering commands.
pub fn handles(command: &OsString) -> bool {
    command == "number-assign" || command == "number-approve" || command == "marks"
}

/// Runs one numbering command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok(json) => (json, 0),
        Err(e) => (
            Json::object([
                (
                    "outcome",
                    Json::text(if e.code == ErrorCode::OutcomeUnknown {
                        "outcome_unknown"
                    } else {
                        "rejected"
                    }),
                ),
                ("code", Json::text(e.code.as_str())),
                ("message", Json::text(e.message)),
                ("transport_code", Json::Null),
                ("observed_context", Json::Null),
                ("diagnostics", Json::Array(vec![])),
            ]),
            match e.code {
                ErrorCode::OutcomeUnknown => 4,
                ErrorCode::BackendUnavailable => 3,
                _ => 2,
            },
        ),
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
        "number-assign" => &["--store", "--id", "--mark", "--revoke"],
        "number-approve" => &["--store", "--id"],
        "marks" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed = own.contains(key) || (*command != "marks" && CONTEXT_OPTIONS.contains(key));
        if !allowed || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(store_root).map_err(|e| PlateError {
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let _claim = crate::plate_cmd::check_project_option(&store, &options)?;
    if *command == "marks" {
        return Ok(Json::object([
            ("outcome", Json::text("marks")),
            ("marks", views_json(&marks(&store)?)),
        ]));
    }
    let given = |key: &str| options.get(key).copied();
    let required = |key: &str, missing: &'static str| given(key).ok_or_else(|| invalid(missing));
    let target = ElementId::parse(required("--id", "--id is required")?).map_err(id_error)?;
    // The value/revoke pair belongs to `number-assign` alone; an approval carries neither.
    let mark = if *command == "number-assign" {
        let revoke = match given("--revoke") {
            Some("true") => true,
            Some(_) => return Err(invalid("--revoke is true or omitted")),
            None => false,
        };
        match (given("--mark"), revoke) {
            (Some(value), false) => Some(value.to_owned()),
            (None, true) => None,
            (Some(_), true) | (None, false) => {
                return Err(invalid(
                    "number-assign takes exactly one of --mark TEXT or --revoke true",
                ));
            }
        }
    } else {
        None
    };
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let context = NumberingContext {
        op_id: match given("--op-id") {
            Some(text) => OpId::parse(text).map_err(id_error)?,
            None => OpId::from_uuid(fresh()?),
        },
        revision_id: match given("--revision-id") {
            Some(text) => RevisionId::parse(text).map_err(id_error)?,
            None => RevisionId::from_uuid(fresh()?),
        },
        actor: match given("--actor-id") {
            Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
            None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
        },
        project: match given("--project-id") {
            Some(text) => ProjectId::parse(text).map_err(id_error)?,
            None => ProjectId::from_uuid(fresh()?),
        },
        workspace: match given("--workspace-id") {
            Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
            None => WorkspaceId::from_uuid(fresh()?),
        },
    };
    let change = if *command == "number-assign" {
        let command = AssignMark {
            target_id: target,
            mark,
        };
        assign(&store, &context, &command, &mut NoFault)?
    } else {
        approve(
            &store,
            &context,
            &ApproveMark { target_id: target },
            &mut NoFault,
        )?
    };
    Ok(change_json(target, context.op_id, &change))
}

fn views_json(views: &[MarkView]) -> Json {
    Json::Array(
        views
            .iter()
            .map(|v| {
                Json::object([
                    ("id", Json::text(v.id.to_string())),
                    ("kind", Json::text(v.kind.clone())),
                    ("name", Json::text(v.name.clone())),
                    ("mark", Json::text(v.mark.clone())),
                    (
                        "status",
                        Json::text(match v.status {
                            MarkStatus::Assigned => "assigned",
                            MarkStatus::Approved => "approved",
                        }),
                    ),
                ])
            })
            .collect(),
    )
}

fn change_json(subject: ElementId, op_id: OpId, change: &MarkChange) -> Json {
    let outcome = if change.replayed {
        "mark_changed_replayed"
    } else {
        "mark_changed"
    };
    Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("marks", views_json(&change.marks)),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ])
}
