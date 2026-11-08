// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! I05 headless commands: `schedule-export`, `schedule-define`, `schedule-undefine` and
//! `schedules`. The export renders the verified head through a schedule template into CSV
//! and HTML files, from a template file or from a stored template; the definition commands
//! save and remove templates in the model, so they carry revisions, footprints, commit and
//! undo like any authored entity. Every command prints one JSON object with the same error
//! shape and exit codes as the other commands.

use crate::bom_cmd::{reserve_all, write_sync};
use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{check_project_option, footprint_json, revision_json};
use beamcraft_commands::attribute::AttrContext;
use beamcraft_commands::{plate::inspect, schedules};
use beamcraft_engine::limits::MAX_ELEMENTS;
use beamcraft_engine::schedule::{build, elements_csv, groups_csv, html};
use beamcraft_host::ids::IdSource;
use beamcraft_host::revision::NoFault;
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::schedule::{DefineSchedule, UndefineSchedule};
use session::Refusal;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read;

type Run<T> = Result<T, Refusal>;

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// The JSON field of one optional KEY VALUE path: the path, or null when the option is away.
fn optional(options: &BTreeMap<&str, &str>, key: &str) -> Json {
    options
        .get(key)
        .map_or(Json::Null, |p| Json::text((*p).to_owned()))
}

/// Runs one schedules command and returns the JSON result and the process exit code.
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
    let reads = matches!(*command, "schedules");
    let mut allowed: Vec<&str> = match *command {
        "schedule-export" => vec![
            "--store",
            "--out",
            "--summary",
            "--html",
            "--template",
            "--schedule",
        ],
        "schedule-define" => vec!["--store", "--name", "--template"],
        "schedule-undefine" => vec!["--store", "--name"],
        "schedules" => vec!["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !reads {
        allowed.extend_from_slice(&CONTEXT_OPTIONS);
    }
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options: BTreeMap<&str, &str> = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !allowed.contains(key) || options.insert(key, value).is_some() {
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
    if *command == "schedules" {
        let stored = schedules::stored(&store)?;
        let entries = stored
            .iter()
            .map(|s| {
                Json::object([
                    ("name", Json::text(s.name.clone())),
                    ("template", Json::text(s.template.clone())),
                ])
            })
            .collect::<Vec<_>>();
        return Ok(Json::object([
            ("outcome", Json::text("schedules")),
            ("schedules", Json::Array(entries)),
        ]));
    }
    if *command == "schedule-export" {
        return export(&store, &options);
    }
    // The two definition commands: the same context and lease discipline as the attributes.
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
    let context = AttrContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    let (subject, change) = match *command {
        "schedule-define" => {
            let name = given("--name")
                .ok_or_else(|| invalid("--name is required"))?
                .to_owned();
            let template = template_document(given("--template"))?;
            let command = DefineSchedule {
                name: name.clone(),
                template: template.clone(),
            };
            let subject = Json::object([("name", Json::text(name))]);
            let change = beamcraft_commands::schedules::define_schedule(
                &store,
                &context,
                &command,
                &mut NoFault,
            )?;
            (subject, change)
        }
        "schedule-undefine" => {
            let name = given("--name")
                .ok_or_else(|| invalid("--name is required"))?
                .to_owned();
            let command = UndefineSchedule { name: name.clone() };
            let subject = Json::object([("name", Json::text(name))]);
            let change = beamcraft_commands::schedules::undefine_schedule(
                &store,
                &context,
                &command,
                &mut NoFault,
            )?;
            (subject, change)
        }
        _ => return Err(invalid("unknown command")),
    };
    let base = format!(
        "schedule_{}",
        command.strip_prefix("schedule-").unwrap_or(command)
    );
    change_json(&base, context.op_id, subject, &change)
}

/// The bounded read of one template file: the same cap and refusal the export applies, so a
/// definition stores exactly what an export would accept.
fn template_document(path: Option<&str>) -> Run<String> {
    let path = path.ok_or_else(|| invalid("--template is required"))?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| {
            f.take(schedules::TEMPLATE_BOUND as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| invalid("the schedule template cannot be read"))?;
    schedules::decode(&bytes)?;
    String::from_utf8(bytes).map_err(|_| invalid("the schedule template is not UTF-8"))
}

/// The export: the verified head rendered through a template file or a stored template.
fn export(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let out = options
        .get("--out")
        .ok_or_else(|| invalid("--out is required"))?;
    let template_json = match (options.get("--template"), options.get("--schedule")) {
        (Some(_), Some(_)) => {
            return Err(invalid("name one template: --template or --schedule"));
        }
        (Some(path), None) => {
            // The template is read bounded before the store, so a broken or oversized
            // template refuses without opening it.
            let mut template_bytes = Vec::new();
            std::fs::File::open(path)
                .and_then(|f| {
                    f.take(schedules::TEMPLATE_BOUND as u64 + 1)
                        .read_to_end(&mut template_bytes)
                })
                .map_err(|_| invalid("the schedule template cannot be read"))?;
            schedules::decode(&template_bytes)?
        }
        (None, Some(name)) => {
            schedules::decode(schedules::stored_named(store, name)?.template.as_bytes())?
        }
        (None, None) => {
            return Err(invalid("one of --template or --schedule is required"));
        }
    };
    // The attribute state is pinned first; the inspection below must name the same revision,
    // or the head moved while the schedule was gathered.
    let (model, attr_revision) = schedules::pinned(store)?;
    let inspection = inspect(store)?;
    if inspection.revision.revision_id != attr_revision {
        return Err(invalid(
            "the store head moved while the schedule was gathered",
        ));
    }
    // The approved numbering marks (A09) of the same head, exactly as export-bom reads them.
    let (mark_revision, marks) = beamcraft_commands::numbering_approval::head_marks(store)?;
    if mark_revision != inspection.revision.revision_id {
        return Err(invalid(
            "the store head moved while the schedule was gathered",
        ));
    }
    let (template, input_rows, unknown_columns) =
        schedules::prepare(&inspection, &marks, &model, &template_json)?;
    let schedule = build(&template, &input_rows, MAX_ELEMENTS)?;
    // Every file is rendered before the first one is created, then reserved, exactly as
    // export-bom writes its two files.
    let mut outputs = vec![(*out, elements_csv(&template, &schedule))];
    if let Some(path) = options.get("--summary") {
        outputs.push((*path, groups_csv(&template, &schedule)));
    }
    if let Some(path) = options.get("--html") {
        outputs.push((*path, html(&template, &schedule)));
    }
    for (path, mut file, bytes) in reserve_all(outputs)? {
        if let Err(e) = write_sync(&mut file, &bytes) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
    }
    Ok(Json::object([
        ("outcome", Json::text("schedule_exported")),
        (
            "revision_id",
            Json::text(inspection.revision.revision_id.clone()),
        ),
        ("elements", Json::uint(schedule.rows.len() as u64)),
        ("groups", Json::uint(schedule.groups.len() as u64)),
        (
            "unknown_columns",
            Json::Array(
                unknown_columns
                    .iter()
                    .map(|s| Json::text(s.as_str()))
                    .collect(),
            ),
        ),
        ("elements_file", Json::text((*out).to_owned())),
        ("summary_file", optional(options, "--summary")),
        ("html_file", optional(options, "--html")),
    ]))
}

/// The one JSON shape of a definition result, exactly the attribute commands' form.
fn change_json(
    base: &str,
    op_id: beamcraft_model::id::OpId,
    subject: Json,
    change: &beamcraft_commands::attribute::AttrChange,
) -> Run<Json> {
    let outcome = if change.replayed {
        format!("{base}_replayed")
    } else {
        base.to_owned()
    };
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("subject", subject),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ]))
}
