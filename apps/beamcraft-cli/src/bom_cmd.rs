// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! E03 headless command `bom-export`: the verified head revision's bill of materials as CSV
//! files. It calls the engine's pure tables through `beamcraft_cli::bom` and prints one JSON
//! object with the same error shape and exit codes as the other commands.

use beamcraft_cli::bom::rows;
use beamcraft_cli::plate::PlateError;
use beamcraft_cli::plate::inspect;
use beamcraft_engine::bom::{elements_csv, summary_csv, tables};
use beamcraft_engine::limits::MAX_ELEMENTS;
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;

type Run<T> = Result<T, PlateError>;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Whether `command` is `bom-export`.
pub fn handles(command: &OsString) -> bool {
    command == "bom-export"
}

/// Runs `bom-export` and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

/// Creates a new file (never overwrites) and returns its handle; nothing is written yet.
fn reserve(path: &str) -> Run<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| invalid("an output file exists or cannot be created"))
}

/// Writes and syncs a reserved file.
fn write_sync(file: &mut std::fs::File, bytes: &str) -> Run<()> {
    file.write_all(bytes.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| PlateError {
            code: ErrorCode::BackendUnavailable,
            message: "the report could not be written",
        })
}

/// Reserves every output file before any bytes are written, so a refusal on any path — an
/// existing or uncreatable summary included, and the same path given twice — lands while
/// nothing exists; the files reserved so far are removed again on the way out.
fn reserve_all(outputs: Vec<(&str, String)>) -> Run<Vec<(String, std::fs::File, String)>> {
    let mut reserved = Vec::with_capacity(outputs.len());
    for (path, bytes) in outputs {
        match reserve(path) {
            Ok(file) => reserved.push((path.to_owned(), file, bytes)),
            Err(e) => {
                for (created, ..) in &reserved {
                    let _ = std::fs::remove_file(created);
                }
                return Err(e);
            }
        }
    }
    Ok(reserved)
}

fn run(args: &[OsString]) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .skip(1)
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    if !args.len().is_multiple_of(2) || args.len() > 6 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in args.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !matches!(*key, "--store" | "--out" | "--summary")
            || options.insert(*key, *value).is_some()
        {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let out = options
        .get("--out")
        .ok_or_else(|| invalid("--out is required"))?;
    let blobs = BlobStore::new(store).map_err(|e| PlateError {
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let inspection = inspect(&store)?;
    // The approved numbering marks (A09) of the same head; a mark the store does not carry
    // falls back to the element id in the report.
    let (mark_revision, marks) = beamcraft_cli::numbering_approval::head_marks(&store)?;
    if mark_revision != inspection.revision.revision_id {
        return Err(invalid(
            "the store head moved while the report was gathered",
        ));
    }
    let bom = tables(&rows(&inspection, &marks)?, MAX_ELEMENTS)?;
    // Both files are rendered before the first one is created, so a rendering failure writes
    // nothing; both are then reserved before any bytes are written, so a refusal on either
    // path leaves nothing.
    let elements = elements_csv(&bom);
    let summary = options
        .get("--summary")
        .map(|path| (*path, summary_csv(&bom)));
    let mut outputs = vec![(*out, elements)];
    outputs.extend(summary);
    for (path, mut file, bytes) in reserve_all(outputs)? {
        if let Err(e) = write_sync(&mut file, &bytes) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
    }
    Ok(Json::object([
        ("outcome", Json::text("bom_exported")),
        (
            "revision_id",
            Json::text(inspection.revision.revision_id.clone()),
        ),
        ("elements", Json::uint(bom.elements.len() as u64)),
        ("groups", Json::uint(bom.groups.len() as u64)),
        ("elements_file", Json::text((*out).to_owned())),
        (
            "summary_file",
            options
                .get("--summary")
                .map_or(Json::Null, |p| Json::text((*p).to_owned())),
        ),
    ]))
}
