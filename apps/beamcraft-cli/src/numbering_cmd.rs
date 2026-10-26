// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A08 headless command `preview-numbering` (groups equivalent parts, proposes number series and
//! shows overlaps; read-only) and the A09 commands `approve-numbering` (stores the approved marks
//! as authored data in the next revision) and `inspect-numbering` (reads the approval back and
//! says whether the model still matches it).

use crate::command_context::{self, CONTEXT_OPTIONS};
use beamcraft_cli::numbering::{NumberingView, numbering_preview};
use beamcraft_cli::numbering_approval::{
    ApprovalContext, Approved, StatusView, approve_numbering, numbering_status,
};
use beamcraft_cli::plate::PlateError;
use beamcraft_engine::numbering::{GroupKind, Series, Settings, unnumbered};
use beamcraft_engine::numbering_approval::{ApproveNumbering, Report, compare_group};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeMap;
use std::ffi::OsString;

type Run<T> = Result<T, PlateError>;

/// Groups listed per page.
const DEFAULT_LIMIT: usize = 64;
const MAX_LIMIT: usize = 256;
/// Members listed per group.
const MAX_MEMBERS: usize = 32;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Whether `command` is one of the numbering commands.
pub fn handles(command: &OsString) -> bool {
    command == "preview-numbering"
        || command == "approve-numbering"
        || command == "inspect-numbering"
}

/// Runs the command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

const SETTINGS_OPTIONS: [&str; 6] = [
    "--plate-series",
    "--beam-series",
    "--column-series",
    "--assembly-series",
    "--tolerance",
    "--compare-name",
];
const PAGING_OPTIONS: [&str; 2] = ["--offset", "--limit"];

fn series(text: Option<&&str>, default: Series) -> Run<Series> {
    let Some(text) = text else {
        return Ok(default);
    };
    let (prefix, start) = text
        .rsplit_once(':')
        .ok_or_else(|| invalid("a series is PREFIX:START"))?;
    Ok(Series {
        prefix: prefix.to_owned(),
        start: start
            .parse::<u64>()
            .map_err(|_| invalid("a series start is not a number"))?,
    })
}

fn settings_of(options: &BTreeMap<&str, &str>) -> Run<Settings> {
    let defaults = Settings::default();
    Ok(Settings {
        plate: series(options.get("--plate-series"), defaults.plate)?,
        beam: series(options.get("--beam-series"), defaults.beam)?,
        column: series(options.get("--column-series"), defaults.column)?,
        assembly: series(options.get("--assembly-series"), defaults.assembly)?,
        tolerance_mm: options
            .get("--tolerance")
            .map(|t| {
                t.parse::<f64>()
                    .map_err(|_| invalid("a number is malformed"))
            })
            .transpose()?
            .unwrap_or(defaults.tolerance_mm),
        compare_name: match options.get("--compare-name") {
            None | Some(&"false") => false,
            Some(&"true") => true,
            Some(_) => return Err(invalid("--compare-name is true or false")),
        },
    })
}

fn token_option(
    options: &BTreeMap<&str, &str>,
    key: &str,
    message: &'static str,
) -> Run<Option<ContentDigest>> {
    options
        .get(key)
        .map(|t| t.parse::<ContentDigest>().map_err(|_| invalid(message)))
        .transpose()
}

fn paging(options: &BTreeMap<&str, &str>) -> Run<(usize, usize)> {
    let offset = options
        .get("--offset")
        .map(|t| {
            t.parse::<usize>()
                .map_err(|_| invalid("--offset is a number"))
        })
        .transpose()?
        .unwrap_or(0);
    let limit = options
        .get("--limit")
        .map(|t| {
            t.parse::<usize>()
                .map_err(|_| invalid("--limit is a number"))
        })
        .transpose()?
        .unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return Err(invalid("--limit is 1 to 256"));
    }
    Ok((offset, limit))
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
    let own: Vec<&str> = match *command {
        "preview-numbering" => [
            &SETTINGS_OPTIONS[..],
            &PAGING_OPTIONS,
            &["--store", "--previous"],
        ]
        .concat(),
        "approve-numbering" => [
            &SETTINGS_OPTIONS[..],
            &CONTEXT_OPTIONS,
            &["--store", "--preview-token"],
        ]
        .concat(),
        "inspect-numbering" => [&PAGING_OPTIONS[..], &["--store"]].concat(),
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !own.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    match *command {
        "approve-numbering" => {
            let settings = settings_of(&options)?;
            let preview_token = token_option(
                &options,
                "--preview-token",
                "--preview-token is a full SHA-256",
            )?
            .ok_or_else(|| invalid("--preview-token is required"))?;
            let store = open_store(store_root)?;
            let _claim = crate::plate_cmd::check_project_option(&store, &options)?;
            approve(
                &store,
                &context_of(&options)?,
                &ApproveNumbering {
                    settings,
                    preview_token,
                },
            )
        }
        "inspect-numbering" => {
            let (offset, limit) = paging(&options)?;
            let store = open_store(store_root)?;
            Ok(status_json(&numbering_status(&store)?, offset, limit))
        }
        _ => {
            let settings = settings_of(&options)?;
            let previous = token_option(&options, "--previous", "--previous is a full SHA-256")?;
            let (offset, limit) = paging(&options)?;
            let store = open_store(store_root)?;
            let view = numbering_preview(&store, &settings, previous)?;
            Ok(view_json(&view, offset, limit))
        }
    }
}

fn open_store(root: &str) -> Run<RevisionStore> {
    let blobs = BlobStore::new(root).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    Ok(RevisionStore::new(blobs))
}

fn context_of(options: &BTreeMap<&str, &str>) -> Run<ApprovalContext> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let common = command_context::parse(options, &mut fresh)?;
    Ok(ApprovalContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    })
}

fn approve(
    store: &RevisionStore,
    context: &ApprovalContext,
    command: &ApproveNumbering,
) -> Run<Json> {
    let approved = approve_numbering(store, context, command, &mut NoFault)?;
    let status = numbering_status(store)?;
    Ok(approved_json(context, &approved, &status))
}

fn approved_json(context: &ApprovalContext, approved: &Approved, status: &StatusView) -> Json {
    let outcome = if approved.replayed {
        "numbering_approved_replayed"
    } else if approved.unchanged {
        "numbering_approved_unchanged"
    } else {
        "numbering_approved"
    };
    let count = |n: Option<usize>| n.map_or(Json::Null, |n| Json::uint(n as u64));
    Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(context.op_id.to_string())),
        ("revision_id", Json::text(status.revision_id.clone())),
        (
            "revision",
            crate::plate_cmd::revision_json(&approved.revision),
        ),
        (
            "changed_layers",
            Json::Array(approved.changed_layers.iter().map(Json::text).collect()),
        ),
        ("groups", count(approved.groups)),
        ("marks", count(approved.marks)),
        (
            "footprint",
            approved.footprint.as_ref().map_or(Json::Null, |f| {
                Json::object([
                    ("reads", Json::uint(f.reads().len() as u64)),
                    ("writes", Json::uint(f.writes().len() as u64)),
                ])
            }),
        ),
        ("approval", approval_json(&status.report)),
    ])
}

fn series_text(s: &Series) -> String {
    format!("{}:{}", s.prefix, s.start)
}

fn approval_json(report: &Report) -> Json {
    Json::object([
        ("state", Json::text(report.currency.as_str())),
        ("reason", report.reason.map_or(Json::Null, Json::text)),
        ("marked", Json::uint(report.marked as u64)),
        ("unmarked", Json::uint(report.unmarked as u64)),
        ("differing", Json::uint(report.differing as u64)),
        (
            "differing_ids",
            Json::Array(
                report
                    .differing_ids
                    .iter()
                    .map(|id| Json::text(id.to_string()))
                    .collect(),
            ),
        ),
        (
            "approved_token",
            report
                .basis
                .as_ref()
                .map_or(Json::Null, |b| Json::text(b.token.to_string())),
        ),
        (
            "approved_settings",
            report.basis.as_ref().map_or(Json::Null, |b| {
                let s = &b.settings;
                Json::object([
                    ("plate_series", Json::text(series_text(&s.plate))),
                    ("beam_series", Json::text(series_text(&s.beam))),
                    ("column_series", Json::text(series_text(&s.column))),
                    ("assembly_series", Json::text(series_text(&s.assembly))),
                    (
                        "tolerance_mm",
                        Json::number(s.tolerance_mm).unwrap_or(Json::Null),
                    ),
                    ("compare_name", Json::Bool(s.compare_name)),
                ])
            }),
        ),
    ])
}

fn status_json(status: &StatusView, offset: usize, limit: usize) -> Json {
    let page: Vec<Json> = status
        .marks
        .iter()
        .skip(offset)
        .take(limit)
        .map(|m| {
            Json::object([
                ("id", Json::text(m.id.to_string())),
                (
                    "kind",
                    m.kind.map_or(Json::Null, |k| Json::text(kind_token(k))),
                ),
                ("series", Json::text(m.mark.series.clone())),
                ("number", Json::uint(m.mark.number)),
                (
                    "mark",
                    m.kind.map_or(Json::Null, |k| Json::text(m.mark.text(k))),
                ),
            ])
        })
        .collect();
    Json::object([
        ("outcome", Json::text("numbering_status")),
        ("revision_id", Json::text(status.revision_id.clone())),
        ("writes_model", Json::Bool(false)),
        ("approval", approval_json(&status.report)),
        ("count", Json::uint(status.marks.len() as u64)),
        ("offset", Json::uint(offset as u64)),
        (
            "next_offset",
            if offset.saturating_add(limit) < status.marks.len() {
                Json::uint((offset + limit) as u64)
            } else {
                Json::Null
            },
        ),
        ("marks", Json::Array(page)),
    ])
}

fn kind_token(kind: GroupKind) -> &'static str {
    match kind {
        GroupKind::Part(k) => k.as_str(),
        GroupKind::Assembly => "assembly",
    }
}

fn view_json(view: &NumberingView, offset: usize, limit: usize) -> Json {
    let groups = &view.preview.groups;
    let page: Vec<Json> = groups
        .iter()
        .skip(offset)
        .take(limit)
        .map(|g| {
            Json::object([
                ("kind", Json::text(kind_token(g.kind))),
                ("position", Json::text(g.position.clone())),
                ("number", Json::uint(g.number)),
                ("status", Json::text(g.status.as_str())),
                (
                    "overlaps_with",
                    g.overlaps_with.clone().map_or(Json::Null, Json::text),
                ),
                ("reference", Json::text(g.reference.to_string())),
                ("count", Json::uint(g.members.len() as u64)),
                (
                    "members",
                    Json::Array(
                        g.members
                            .iter()
                            .take(MAX_MEMBERS)
                            .map(|m| Json::text(m.to_string()))
                            .collect(),
                    ),
                ),
                (
                    "members_truncated",
                    Json::Bool(g.members.len() > MAX_MEMBERS),
                ),
                ("description", Json::text(g.description.clone())),
                (
                    "stored",
                    Json::text(compare_group(g, &view.stored).as_str()),
                ),
            ])
        })
        .collect();
    let series = |name: &str, s: &Series, kind: GroupKind| {
        let of_kind: Vec<_> = groups.iter().filter(|g| g.kind == kind).collect();
        Json::object([
            ("kind", Json::text(name)),
            ("prefix", Json::text(s.prefix.clone())),
            ("start", Json::uint(s.start)),
            ("groups", Json::uint(of_kind.len() as u64)),
            (
                "first",
                of_kind
                    .first()
                    .map_or(Json::Null, |g| Json::text(g.position.clone())),
            ),
            (
                "last",
                of_kind
                    .last()
                    .map_or(Json::Null, |g| Json::text(g.position.clone())),
            ),
        ])
    };
    use beamcraft_engine::numbering::PartKind;
    let s = &view.settings;
    let overlapping = groups
        .iter()
        .filter(|g| g.status == beamcraft_engine::numbering::Status::Overlapping)
        .count();
    Json::object([
        ("outcome", Json::text("numbering_preview")),
        ("revision_id", Json::text(view.revision_id.clone())),
        ("preview_token", Json::text(view.preview.token.to_string())),
        ("stale", view.stale.map_or(Json::Null, Json::Bool)),
        ("writes_model", Json::Bool(false)),
        ("approval", approval_json(&view.approval)),
        (
            "settings",
            Json::object([
                ("compare_name", Json::Bool(s.compare_name)),
                (
                    "tolerance_mm",
                    Json::number(s.tolerance_mm).unwrap_or(Json::Null),
                ),
            ]),
        ),
        (
            "series",
            Json::Array(vec![
                series("plate", &s.plate, GroupKind::Part(PartKind::Plate)),
                series("beam", &s.beam, GroupKind::Part(PartKind::Beam)),
                series("column", &s.column, GroupKind::Part(PartKind::Column)),
                series("assembly", &s.assembly, GroupKind::Assembly),
            ]),
        ),
        (
            "counts",
            Json::object([
                ("parts", Json::uint(view.parts as u64)),
                ("assemblies", Json::uint(view.assemblies as u64)),
                ("groups", Json::uint(groups.len() as u64)),
                ("overlapping_groups", Json::uint(overlapping as u64)),
                (
                    "unnumbered_members",
                    Json::uint(unnumbered(&view.preview).len() as u64),
                ),
            ]),
        ),
        ("offset", Json::uint(offset as u64)),
        (
            "next_offset",
            if offset.saturating_add(limit) < groups.len() {
                Json::uint((offset + limit) as u64)
            } else {
                Json::Null
            },
        ),
        ("groups", Json::Array(page)),
    ])
}
