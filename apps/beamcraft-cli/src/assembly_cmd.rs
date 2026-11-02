// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A01 headless commands: `create-assembly`, `attach-to-assembly`, `detach-from-assembly` and
//! `assemblies`. They call the engine API through `beamcraft_commands::assembly` and print one JSON
//! object with the same error shape and exit codes as the plate commands.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{footprint_json, id_error, revision_json};
use beamcraft_commands::assembly::{
    AssemblyChange, AssemblyContext, AssemblyView, assemblies, attach, create_assembly, detach,
    explode, set_main_part,
};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::command::{
    AttachToAssembly, CreateAssembly, DetachFromAssembly, ExplodeAssembly, SetAssemblyMainPart,
};
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

/// Runs one assembly command and returns the JSON result and the process exit code.
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
        "create-assembly" => &["--store", "--name", "--new-id"],
        "attach-to-assembly" => &["--store", "--id", "--assembly-id"],
        "detach-from-assembly" => &["--store", "--id"],
        "set-main-part" => &["--store", "--id", "--assembly-id"],
        "explode-assembly" => &["--store", "--id"],
        "assemblies" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed =
            own.contains(key) || (*command != "assemblies" && CONTEXT_OPTIONS.contains(key));
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
    let _claim = crate::plate_cmd::check_project_option(&store, &options)?;
    if *command == "assemblies" {
        return Ok(Json::object([
            ("outcome", Json::text("assemblies")),
            ("assemblies", views_json(&assemblies(&store)?)),
        ]));
    }
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| Refusal {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let element = |key: &str, missing: &'static str| -> Run<ElementId> {
        ElementId::parse(given(key).ok_or_else(|| invalid(missing))?).map_err(id_error)
    };
    let common = command_context::parse(&options, &mut fresh)?;
    let context = AssemblyContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    let (outcome, subject, change) = match *command {
        "create-assembly" => {
            let new_id = match given("--new-id") {
                Some(text) => ElementId::parse(text).map_err(id_error)?,
                None => ElementId::from_uuid(fresh()?),
            };
            let command = CreateAssembly {
                new_id,
                name: given("--name")
                    .ok_or_else(|| invalid("--name is required"))?
                    .to_owned(),
            };
            let change = create_assembly(&store, &context, &command, &mut NoFault)?;
            ("assembly_created", new_id, change)
        }
        "attach-to-assembly" => {
            let command = AttachToAssembly {
                part_id: element("--id", "--id is required")?,
                assembly_id: element("--assembly-id", "--assembly-id is required")?,
            };
            let change = attach(&store, &context, &command, &mut NoFault)?;
            ("part_attached", command.part_id, change)
        }
        "set-main-part" => {
            let command = SetAssemblyMainPart {
                assembly_id: element("--assembly-id", "--assembly-id is required")?,
                part_id: element("--id", "--id is required")?,
            };
            let change = set_main_part(&store, &context, &command, &mut NoFault)?;
            ("main_part_set", command.assembly_id, change)
        }
        "explode-assembly" => {
            let command = ExplodeAssembly {
                assembly_id: element("--id", "--id is required")?,
            };
            let change = explode(&store, &context, &command, &mut NoFault)?;
            ("assembly_exploded", command.assembly_id, change)
        }
        _ => {
            let command = DetachFromAssembly {
                part_id: element("--id", "--id is required")?,
            };
            let change = detach(&store, &context, &command, &mut NoFault)?;
            ("part_detached", command.part_id, change)
        }
    };
    Ok(change_json(outcome, subject, context.op_id, &change))
}

fn views_json(views: &[AssemblyView]) -> Json {
    Json::Array(
        views
            .iter()
            .map(|v| {
                Json::object([
                    ("id", Json::text(v.id.to_string())),
                    ("name", Json::text(v.name.clone())),
                    (
                        "part_of",
                        v.parent.map_or(Json::Null, |p| Json::text(p.to_string())),
                    ),
                    (
                        "parts",
                        Json::Array(v.parts.iter().map(|p| Json::text(p.to_string())).collect()),
                    ),
                    (
                        "main",
                        v.main.map_or(Json::Null, |p| Json::text(p.to_string())),
                    ),
                ])
            })
            .collect(),
    )
}

fn change_json(outcome: &str, subject: ElementId, op_id: OpId, change: &AssemblyChange) -> Json {
    let outcome = if change.replayed {
        format!("{outcome}_replayed")
    } else {
        outcome.to_owned()
    };
    Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("assemblies", views_json(&change.assemblies)),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ])
}
