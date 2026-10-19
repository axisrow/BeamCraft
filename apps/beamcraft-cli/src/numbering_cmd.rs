// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A08 headless command `numbering-preview`: groups equivalent parts, proposes number series and
//! shows overlaps. Read-only: it approves nothing and writes nothing to the store.

use beamcraft_cli::numbering::{NumberingView, numbering_preview};
use beamcraft_cli::plate::PlateError;
use beamcraft_engine::numbering::{GroupKind, Series, Settings, unnumbered};
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_protocol::json::Json;
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
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Whether `command` is `numbering-preview`.
pub fn handles(command: &OsString) -> bool {
    command == "numbering-preview"
}

/// Runs the command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::plate_cmd::report(run(args))
}

const OPTIONS: [&str; 10] = [
    "--store",
    "--plate-series",
    "--beam-series",
    "--column-series",
    "--assembly-series",
    "--tolerance",
    "--compare-name",
    "--previous",
    "--offset",
    "--limit",
];

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

fn run(args: &[OsString]) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (_, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !OPTIONS.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let defaults = Settings::default();
    let settings = Settings {
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
    };
    let previous = options
        .get("--previous")
        .map(|t| {
            t.parse::<ContentDigest>()
                .map_err(|_| invalid("--previous is a full SHA-256"))
        })
        .transpose()?;
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
    let blobs = BlobStore::new(store_root).map_err(|e| PlateError {
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let view = numbering_preview(&store, &settings, previous)?;
    Ok(view_json(&view, offset, limit))
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
