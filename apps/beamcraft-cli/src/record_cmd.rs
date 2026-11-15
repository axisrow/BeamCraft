// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 headless commands: `record-create` and `record-list`. They call the engine API
//! through `beamcraft_commands::record` and print one JSON object with the same error
//! shape and exit codes as the annotation commands.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{check_project_option, footprint_json, id_error, revision_json};
use beamcraft_commands::record::{RecordChange, RecordContext, create_record, records};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_model::record::{
    ConnectionKind, DeclaredConnection, GapRequirement, JointIntent, Record, RecordPayload,
};
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

/// Runs one record command and returns the JSON result and the process exit code.
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
        "record-create" => &[
            "--store",
            "--name",
            "--kind",
            "--new-id",
            "--scope",
            "--connection",
            "--gap-a",
            "--gap-b",
            "--gap-min",
            "--gap-max",
            "--gap-tolerance",
        ],
        "record-list" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 60 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed =
            own.contains(key) || (*command != "record-list" && CONTEXT_OPTIONS.contains(key));
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
    if *command == "record-list" {
        let found = records(&store)?;
        return Ok(Json::object([
            ("outcome", Json::text("records")),
            ("records", list_json(&found)?),
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
    let context = RecordContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    let new_id = match given("--new-id") {
        Some(text) => ElementId::parse(text).map_err(id_error)?,
        None => ElementId::from_uuid(crate::command_context::default_id(
            options.get("--op-id").copied(),
            "new_id/0",
            &mut fresh,
        )?),
    };
    let name = given("--name")
        .ok_or_else(|| invalid("--name is required"))?
        .to_owned();
    let payload = match given("--kind").ok_or_else(|| invalid("--kind is required"))? {
        "joint_intent" => {
            let scope = given("--scope")
                .ok_or_else(|| invalid("a joint intent needs --scope UUID,UUID,..."))?
                .split(',')
                .map(|text| ElementId::parse(text.trim()).map_err(id_error))
                .collect::<Run<Vec<_>>>()?;
            let mut connections = Vec::new();
            if let Some(texts) = given("--connection") {
                // The declared kind travels inside one option value: `--connection
                // UUID:weld:UUID`, repeated per declared connection. The pairs are
                // comma-free so one option value carries one connection.
                let mut text_list = texts.split(',').peekable();
                if text_list.peek().is_some() {
                    for text in text_list {
                        let parts: Vec<&str> = text.split(':').collect();
                        let [a, kind, b] = parts.as_slice() else {
                            return Err(invalid("--connection takes UUID:weld|contact:UUID"));
                        };
                        let kind = match *kind {
                            "weld" => ConnectionKind::Weld,
                            "contact" => ConnectionKind::Contact,
                            _ => return Err(invalid("--connection takes UUID:weld|contact:UUID")),
                        };
                        connections.push(DeclaredConnection {
                            a: ElementId::parse(a).map_err(id_error)?,
                            b: ElementId::parse(b).map_err(id_error)?,
                            kind,
                        });
                    }
                }
            }
            RecordPayload::JointIntent(JointIntent { scope, connections })
        }
        "gap_requirement" => {
            let number = |key: &'static str| -> Run<f64> {
                given(key)
                    .map(|text| text.parse::<f64>())
                    .transpose()
                    .map_err(|_| invalid("a gap value is a number"))?
                    .ok_or_else(|| {
                        invalid("a gap requirement needs --gap-min, --gap-max and --gap-tolerance")
                    })
            };
            let a = given("--gap-a")
                .ok_or_else(|| invalid("a gap requirement needs --gap-a and --gap-b"))?;
            let b = given("--gap-b")
                .ok_or_else(|| invalid("a gap requirement needs --gap-a and --gap-b"))?;
            RecordPayload::GapRequirement(GapRequirement {
                a: ElementId::parse(a).map_err(id_error)?,
                b: ElementId::parse(b).map_err(id_error)?,
                permitted_min_mm: number("--gap-min")?,
                permitted_max_mm: number("--gap-max")?,
                tolerance_mm: number("--gap-tolerance")?,
            })
        }
        _ => return Err(invalid("--kind is one of joint_intent or gap_requirement")),
    };
    let command = beamcraft_model::record::CreateRecord {
        record: Record {
            id: new_id,
            name,
            payload,
        },
    };
    let change = create_record(&store, &context, &command, &mut NoFault)?;
    change_json(new_id, context.op_id, &change)
}

/// The records as JSON.
fn list_json(stored: &[Record]) -> Run<Json> {
    Ok(Json::Array(
        stored
            .iter()
            .map(|record| {
                let mut fields = vec![
                    ("id", Json::text(record.id.to_string())),
                    ("name", Json::text(record.name.clone())),
                ];
                match &record.payload {
                    RecordPayload::JointIntent(intent) => {
                        fields.push(("kind", Json::text("joint_intent")));
                        fields.push((
                            "scope",
                            Json::Array(
                                intent
                                    .scope
                                    .iter()
                                    .map(|id| Json::text(id.to_string()))
                                    .collect(),
                            ),
                        ));
                        fields.push((
                            "connections",
                            Json::Array(
                                intent
                                    .connections
                                    .iter()
                                    .map(|connection| {
                                        Json::object([
                                            ("a", Json::text(connection.a.to_string())),
                                            ("b", Json::text(connection.b.to_string())),
                                            ("kind", Json::text(connection.kind.token())),
                                        ])
                                    })
                                    .collect(),
                            ),
                        ));
                    }
                    RecordPayload::GapRequirement(gap) => {
                        fields.push(("kind", Json::text("gap_requirement")));
                        fields.push(("a", Json::text(gap.a.to_string())));
                        fields.push(("b", Json::text(gap.b.to_string())));
                        fields.push(("permitted_min_mm", number(gap.permitted_min_mm)?));
                        fields.push(("permitted_max_mm", number(gap.permitted_max_mm)?));
                        fields.push(("tolerance_mm", number(gap.tolerance_mm)?));
                    }
                }
                Ok(Json::Object(
                    fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
                ))
            })
            .collect::<Run<Vec<_>>>()?,
    ))
}

/// A finite measurement; an unfittable value fails the command rather than a guessed null.
fn number(value: f64) -> Run<Json> {
    Json::number(value).map_err(|_| invalid("a gap value is not representable"))
}

fn change_json(subject: ElementId, op_id: OpId, change: &RecordChange) -> Run<Json> {
    let outcome = if change.replayed {
        "record_created_replayed"
    } else {
        "record_created"
    };
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("records", list_json(&change.records)?),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ]))
}
