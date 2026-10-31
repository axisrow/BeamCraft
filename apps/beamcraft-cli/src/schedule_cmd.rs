// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! I05 headless command `schedule-export`: the verified head rendered through a schedule
//! template into CSV and HTML files. It reuses the E03 read (the BOM rows and their pinned
//! attribute context), the V14 filter evaluator and the E03 atomic write discipline, and
//! prints one JSON object with the same error shape and exit codes as the other commands.

use crate::bom_cmd::{reserve_all, write_sync};
use beamcraft_cli::{
    plate::{PlateError, inspect},
    schedules,
};
use beamcraft_engine::limits::MAX_ELEMENTS;
use beamcraft_engine::schedule::{build, elements_csv, groups_csv, html};
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use std::ffi::OsString;
use std::io::Read;

type Run<T> = Result<T, PlateError>;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// The JSON field of one optional KEY VALUE path: the path, or null when the option is away.
fn optional(options: &std::collections::BTreeMap<&str, &str>, key: &str) -> Json {
    options
        .get(key)
        .map_or(Json::Null, |p| Json::text((*p).to_owned()))
}

/// Runs `schedule-export` and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

fn run(args: &[OsString]) -> Run<Json> {
    let options: std::collections::BTreeMap<&str, &str> = crate::bom_cmd::pairs(
        args,
        &["--store", "--template", "--out", "--summary", "--html"],
        10,
    )?
    .into_iter()
    .collect();
    let store = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let template_path = options
        .get("--template")
        .ok_or_else(|| invalid("--template is required"))?;
    let out = options
        .get("--out")
        .ok_or_else(|| invalid("--out is required"))?;
    // The template is read bounded before the store, so a broken or oversized template
    // refuses without opening it.
    let mut template_bytes = Vec::new();
    std::fs::File::open(template_path)
        .and_then(|f| {
            f.take(schedules::TEMPLATE_BOUND as u64 + 1)
                .read_to_end(&mut template_bytes)
        })
        .map_err(|_| invalid("the schedule template cannot be read"))?;
    let template_json = schedules::decode(&template_bytes)?;
    let blobs = BlobStore::new(store).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    // The attribute state is pinned first; the inspection below must name the same revision,
    // or the head moved while the schedule was gathered.
    let (model, attr_revision) = schedules::pinned(&store)?;
    let inspection = inspect(&store)?;
    if inspection.revision.revision_id != attr_revision {
        return Err(invalid(
            "the store head moved while the schedule was gathered",
        ));
    }
    // The approved numbering marks (A09) of the same head, exactly as export-bom reads them.
    let (mark_revision, marks) = beamcraft_cli::numbering_approval::head_marks(&store)?;
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
        ("summary_file", optional(&options, "--summary")),
        ("html_file", optional(&options, "--html")),
    ]))
}
