// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A07.2 and A07.3 headless commands: `library-list`, `library-show` and `library-add`. They read
//! the files named on the command line (one `--library`, or an `--environment` with its
//! libraries, and an optional `--user-library` that comes first; never a hidden global), never a
//! store, and print one JSON object with the error shape and exit codes of the plate commands.
//! Only `library-add` writes, and only the one library file it names.

use crate::plate_cmd::{Run, definition_json, fit_page, invalid, page_of, profile_record};
use crate::report::report;
use beamcraft_cli::library::{
    EnvironmentInfo, LibraryEntry, LibraryInfo, Lookup, NewLibrary, RejectedEntry, Shadowed,
    Source, add_entry, open_lookup,
};
use beamcraft_json::Json;
use std::collections::BTreeMap;
use std::ffi::OsString;

/// Whether `command` is one of the library commands.
pub fn handles(command: &OsString) -> bool {
    command == "library-list" || command == "library-show" || command == "library-add"
}

/// Runs one library command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    report(run(args))
}

fn optional(value: &Option<String>) -> Json {
    value.as_ref().map_or(Json::Null, Json::text)
}

fn info_json(info: &LibraryInfo) -> Json {
    Json::object([
        ("id", Json::text(info.id.clone())),
        ("title", optional(&info.title)),
        ("version", optional(&info.version)),
        ("source", optional(&info.source)),
        ("licence", optional(&info.licence)),
        ("digest", Json::text(info.digest.to_string())),
        ("bytes", Json::uint(info.bytes)),
    ])
}

fn environment_json(info: &EnvironmentInfo) -> Json {
    Json::object([
        ("id", Json::text(info.id.clone())),
        ("title", optional(&info.title)),
        ("digest", Json::text(info.digest.to_string())),
        ("bytes", Json::uint(info.bytes)),
        (
            "libraries",
            Json::Array(info.libraries.iter().map(Json::text).collect()),
        ),
    ])
}

fn source_json(source: &Source) -> Json {
    Json::object([
        ("path", Json::text(source.path.clone())),
        ("library", info_json(&source.library.info)),
        (
            "entry_count",
            Json::uint(source.library.entries.len() as u64),
        ),
        (
            "rejected_count",
            Json::uint(source.library.rejected.len() as u64),
        ),
    ])
}

fn entry_json(entry: &LibraryEntry, library_id: &str) -> Run<Json> {
    let digest = entry.record.resource()?.digest;
    let Json::Object(mut object) = definition_json(&digest, &entry.record)? else {
        return Err(invalid("entry result is not an object"));
    };
    // The digest is what `define-profile --library` would pin; the library prim is not a
    // definition path.
    object.remove("prim_path");
    object.insert("library_prim".into(), Json::text(entry.prim_path.clone()));
    object.insert("library_id".into(), Json::text(library_id));
    object.insert("designation".into(), Json::text(entry.designation.clone()));
    Ok(Json::Object(object))
}

fn rejected_json<'a>(rejected: impl Iterator<Item = (&'a str, &'a RejectedEntry)>) -> Json {
    Json::Array(
        rejected
            .map(|(library_id, r)| {
                Json::object([
                    ("library_id", Json::text(library_id)),
                    ("prim_path", Json::text(r.prim_path.clone())),
                    (
                        "designation",
                        r.designation.as_ref().map_or(Json::Null, Json::text),
                    ),
                    ("code", Json::text(r.code.as_str())),
                    ("message", Json::text(r.message)),
                ])
            })
            .collect(),
    )
}

fn shadow_json(records: impl IntoIterator<Item = Shadowed>) -> Json {
    Json::Array(
        records
            .into_iter()
            .map(|s| {
                Json::object([
                    ("designation", Json::text(s.designation)),
                    ("used_library", Json::text(s.used_library)),
                    ("shadowed_library", Json::text(s.shadowed_library)),
                ])
            })
            .collect(),
    )
}

const PROFILE_OPTIONS: &[&str] = &[
    "--width",
    "--height",
    "--family",
    "--h",
    "--b",
    "--tw",
    "--tf",
    "--t",
    "--r",
    "--c",
    "--d",
    "--outer",
    "--cavity",
    "--designation",
    "--standard",
    "--area-mm2",
    "--mass-per-meter-kg",
    "--surface-per-meter-m2",
];

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
    let lookup_options = ["--library", "--environment", "--user-library"];
    let allowed: Vec<&str> = match *command {
        "library-list" => [
            &lookup_options[..],
            &[
                "--family",
                "--filter",
                "--offset",
                "--limit",
                "--shadow-offset",
            ],
        ]
        .concat(),
        "library-show" => [&lookup_options[..], &["--designation"]].concat(),
        "library-add" => [
            &["--library", "--library-id", "--title"][..],
            PROFILE_OPTIONS,
        ]
        .concat(),
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 48 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !allowed.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    if *command == "library-add" {
        return add(&options);
    }
    let (lookup, environment) = open_lookup(
        options.get("--library").copied(),
        options.get("--environment").copied(),
        options.get("--user-library").copied(),
    )?;
    if *command == "library-show" {
        let designation = options
            .get("--designation")
            .ok_or_else(|| invalid("--designation is required"))?;
        let (source, entry) = lookup.find(designation)?;
        return Ok(Json::object([
            ("outcome", Json::text("library_entry")),
            ("library", info_json(&source.library.info)),
            ("library_path", Json::text(source.path.clone())),
            ("entry", entry_json(entry, &source.library.info.id)?),
            ("shadowed", shadow_json(lookup.shadowed_for(designation))),
        ]));
    }
    list(&options, &lookup, environment.as_ref())
}

fn list(
    options: &BTreeMap<&str, &str>,
    lookup: &Lookup,
    environment: Option<&EnvironmentInfo>,
) -> Run<Json> {
    let family = options.get("--family");
    let filter = options.get("--filter").map(|f| f.to_ascii_lowercase());
    let (offset, limit) = page_of(options)?;
    let matching: Vec<(&Source, &LibraryEntry)> = lookup
        .effective()
        .into_iter()
        .filter(|(_, e)| family.is_none_or(|f| e.record.kind() == *f))
        .filter(|(_, e)| {
            filter
                .as_ref()
                .is_none_or(|f| e.designation.to_ascii_lowercase().contains(f.as_str()))
        })
        .collect();
    let refused: Vec<(&str, &RejectedEntry)> = lookup
        .sources()
        .iter()
        .flat_map(|s| {
            s.library
                .rejected
                .iter()
                .map(|r| (s.library.info.id.as_str(), r))
        })
        .collect();
    // The shadow records are paged on their own (`--shadow-offset`, `--limit` of them): a few
    // overlapping libraries of thousands of entries make tens of thousands of records, which
    // must not decide whether the entries fit the output budget.
    let shadows = lookup.shadowed();
    let shadow_offset = match options.get("--shadow-offset") {
        Some(text) => text
            .parse::<usize>()
            .map_err(|_| invalid("--shadow-offset is not a number"))?,
        None => 0,
    };
    let shadow_end = shadow_offset.saturating_add(limit).min(shadows.len());
    let shadow_page: Vec<Shadowed> = shadows
        .get(shadow_offset..shadow_end)
        .unwrap_or_default()
        .to_vec();
    let shadow_next = if shadow_end < shadows.len() {
        Json::uint(shadow_end as u64)
    } else {
        Json::Null
    };
    let shadow_total = shadows.len() as u64;
    // One position space: the matching entries, then the refused ones.
    let (entry_total, rejected_total) = (matching.len(), refused.len());
    let total = entry_total.saturating_add(rejected_total);
    fit_page(total.saturating_sub(offset), limit, |count| {
        let entries_here = entry_total.saturating_sub(offset).min(count);
        let entries = matching
            .iter()
            .skip(offset)
            .take(entries_here)
            .map(|(source, entry)| entry_json(entry, &source.library.info.id))
            .collect::<Run<Vec<_>>>()?;
        let rejected = rejected_json(
            refused
                .iter()
                .skip(offset.saturating_sub(entry_total))
                .take(count - entries_here)
                .copied(),
        );
        let next = offset.saturating_add(count);
        let mut object = BTreeMap::from([
            ("outcome".to_owned(), Json::text("library")),
            ("entry_count".to_owned(), Json::uint(entry_total as u64)),
            (
                "rejected_count".to_owned(),
                Json::uint(rejected_total as u64),
            ),
            ("offset".to_owned(), Json::uint(offset as u64)),
            ("page_size".to_owned(), Json::uint(count as u64)),
            (
                "next_offset".to_owned(),
                if next < total {
                    Json::uint(next as u64)
                } else {
                    Json::Null
                },
            ),
            ("entries".to_owned(), Json::Array(entries)),
            ("rejected".to_owned(), rejected),
            (
                "libraries".to_owned(),
                Json::Array(lookup.sources().iter().map(source_json).collect()),
            ),
            ("shadowed".to_owned(), shadow_json(shadow_page.clone())),
            ("shadowed_count".to_owned(), Json::uint(shadow_total)),
            ("shadowed_next_offset".to_owned(), shadow_next.clone()),
        ]);
        if let [only] = lookup.sources()
            && environment.is_none()
        {
            object.insert("library".to_owned(), info_json(&only.library.info));
        }
        if let Some(environment) = environment {
            object.insert("environment".to_owned(), environment_json(environment));
        }
        Ok(Json::Object(object))
    })
}

/// `library-add`: one profile entry into the library file named by `--library`, creating the file
/// when `--library-id` is given.
fn add(options: &BTreeMap<&str, &str>) -> Run<Json> {
    let path = options
        .get("--library")
        .ok_or_else(|| invalid("--library is required"))?;
    let designation = options
        .get("--designation")
        .ok_or_else(|| invalid("--designation is required"))?;
    let new = match (options.get("--library-id"), options.get("--title")) {
        (Some(id), title) => Some(NewLibrary {
            id: (*id).to_owned(),
            title: title.map(|t| (*t).to_owned()),
        }),
        (None, Some(_)) => return Err(invalid("--title needs --library-id")),
        (None, None) => None,
    };
    let record = profile_record(options)?;
    let added = add_entry(path, record, designation, new)?;
    Ok(Json::object([
        ("outcome", Json::text("library_entry_added")),
        ("created", Json::Bool(added.created)),
        ("library", info_json(&added.info)),
        ("entry_count", Json::uint(added.entry_count as u64)),
        ("entry", entry_json(&added.entry, &added.info.id)?),
    ]))
}
