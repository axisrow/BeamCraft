// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D2 headless commands: `create-level`, `create-levels`, `update-level` and `levels`. They call the
//! engine API through `beamcraft_commands::level` and print one JSON object with the same error
//! shape and exit codes as the plate commands. Assigning an element to a level is
//! `update-element --level`.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{check_project_option, footprint_json, id_error, num, revision_json};
use beamcraft_commands::level::{
    LevelChange, LevelContext, LevelView, create_level, create_levels, levels, update_level,
};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::command::{CreateLevel, UpdateLevel};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_model::level::{CreateLevels, level_elevations};
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

/// Runs one level command and returns the JSON result and the process exit code.
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
        "create-level" => &["--store", "--name", "--elevation", "--new-id"],
        "create-levels" => &["--store", "--elevations", "--names"],
        "update-level" => &[
            "--store",
            "--id",
            "--name",
            "--elevation",
            "--move-elements",
        ],
        "levels" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed = own.contains(key) || (*command != "levels" && CONTEXT_OPTIONS.contains(key));
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
    if *command == "levels" {
        return Ok(Json::object([
            ("outcome", Json::text("levels")),
            ("levels", views_json(&levels(&store)?)?),
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
    let context = LevelContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    let elevation = |text: &str| {
        text.parse::<f64>()
            .map_err(|_| invalid("a number is malformed"))
    };
    if *command == "update-level" {
        let target_id = ElementId::parse(given("--id").ok_or_else(|| invalid("--id is required"))?)
            .map_err(id_error)?;
        let update = UpdateLevel {
            target_id,
            name: given("--name").map(str::to_owned),
            elevation_mm: given("--elevation").map(elevation).transpose()?,
            move_elements: match given("--move-elements") {
                None | Some("false") => false,
                Some("true") => true,
                Some(_) => return Err(invalid("--move-elements is true or false")),
            },
        };
        let change = update_level(&store, &context, &update, &mut NoFault)?;
        return change_json(target_id, context.op_id, &change, "level_updated");
    }
    if *command == "create-levels" {
        let elevations =
            given("--elevations").ok_or_else(|| invalid("--elevations is required"))?;
        let count = level_elevations(elevations)?.len();
        let batch = CreateLevels {
            new_ids: (0..count)
                .map(|_| Ok(ElementId::from_uuid(fresh()?)))
                .collect::<Run<_>>()?,
            elevations: elevations.to_owned(),
            names: given("--names").unwrap_or_default().to_owned(),
        };
        let change = create_levels(&store, &context, &batch, &mut NoFault)?;
        return batch_json(&batch, context.op_id, &change);
    }
    let new_id = match given("--new-id") {
        Some(text) => ElementId::parse(text).map_err(id_error)?,
        None => ElementId::from_uuid(crate::command_context::default_id(
            options.get("--op-id").copied(),
            "new_id/0",
            &mut fresh,
        )?),
    };
    let command = CreateLevel {
        new_id,
        name: given("--name")
            .ok_or_else(|| invalid("--name is required"))?
            .to_owned(),
        elevation_mm: elevation(
            given("--elevation").ok_or_else(|| invalid("--elevation is required"))?,
        )?,
    };
    let change = create_level(&store, &context, &command, &mut NoFault)?;
    change_json(new_id, context.op_id, &change, "level_created")
}

/// The levels as JSON, in the order given (ascending identity).
pub(crate) fn views_json(views: &[LevelView]) -> Run<Json> {
    Ok(Json::Array(
        views
            .iter()
            .map(|v| {
                Ok(Json::object([
                    ("id", Json::text(v.id.to_string())),
                    ("name", Json::text(v.name.clone())),
                    ("elevation_mm", num(v.elevation_mm)?),
                    (
                        "elements",
                        Json::Array(
                            v.elements
                                .iter()
                                .map(|e| Json::text(e.to_string()))
                                .collect(),
                        ),
                    ),
                ]))
            })
            .collect::<Run<Vec<_>>>()?,
    ))
}

/// The reply of `create-levels`: the created identities in the order of the elevations; a replay
/// created nothing in this call, so it lists none (the head's `levels` hold the first call's).
fn batch_json(batch: &CreateLevels, op_id: OpId, change: &LevelChange) -> Run<Json> {
    let (outcome, created) = if change.replayed {
        ("levels_created_replayed", Vec::new())
    } else {
        (
            "levels_created",
            batch
                .new_ids
                .iter()
                .map(|id| Json::text(id.to_string()))
                .collect(),
        )
    };
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_ids", Json::Array(created)),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("levels", views_json(&change.levels)?),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ]))
}

fn change_json(subject: ElementId, op_id: OpId, change: &LevelChange, accepted: &str) -> Run<Json> {
    let outcome = if change.replayed {
        format!("{accepted}_replayed")
    } else {
        accepted.to_owned()
    };
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("levels", views_json(&change.levels)?),
        (
            "moved_elements",
            Json::Array(
                change
                    .moved
                    .iter()
                    .map(|id| Json::text(id.to_string()))
                    .collect(),
            ),
        ),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ]))
}
