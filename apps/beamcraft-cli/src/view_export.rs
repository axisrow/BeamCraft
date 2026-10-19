// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V03 headless command `view-export`: the pinned view snapshot of the verified head revision as
//! one engine bundle (`beamcraft_engine::query::bundle`), the input of the native viewport.
//!
//! `--out -` writes the raw bundle to standard output and nothing else (the viewport reads it
//! through a pipe); `--out FILE` creates a new file (never overwrites) and prints a JSON summary.

use beamcraft_cli::plate::{PlateError, inspect};
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_model::error::ErrorCode;
use beamcraft_protocol::json::Json;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;

/// What `execute` produced.
pub enum Output {
    /// A JSON object for standard output, and the exit code.
    Json(Json, u8),
    /// Raw bundle bytes for standard output.
    Raw(Vec<u8>),
}

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs `view-export`.
pub fn execute(args: &[OsString]) -> Output {
    match run(args) {
        Ok(output) => output,
        Err(e) => Output::Json(
            Json::object([
                ("outcome", Json::text("rejected")),
                ("code", Json::text(e.code.as_str())),
                ("message", Json::text(e.message)),
                ("transport_code", Json::Null),
                ("observed_context", Json::Null),
                ("diagnostics", Json::Array(vec![])),
            ]),
            match e.code {
                ErrorCode::BackendUnavailable => 3,
                _ => 2,
            },
        ),
    }
}

fn run(args: &[OsString]) -> Result<Output, PlateError> {
    let args: Vec<&str> = args
        .iter()
        .skip(1)
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Result<_, _>>()?;
    if !args.len().is_multiple_of(2) || args.len() > 6 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in args.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !matches!(*key, "--store" | "--out" | "--include-definitions")
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
    let include_definitions = match options.get("--include-definitions") {
        None => false,
        Some(&"true") => true,
        _ => return Err(invalid("--include-definitions must be true")),
    };
    let blobs = BlobStore::new(store).map_err(|e| PlateError {
        code: e.code(),
        message: "store root is not usable",
    })?;
    let inspection = inspect(&RevisionStore::new(blobs))?;
    let snapshot = &inspection.snapshot;
    let bundle = snapshot.to_bundle().map_err(|e| PlateError {
        code: e.f04_code(),
        message: "the view snapshot could not be bundled",
    })?;
    let bundle = if include_definitions {
        // Definitions and geometry come from the same verified inspection, not a second read
        // of a potentially newer head. A metadata failure does not discard valid geometry.
        let definitions = crate::plate_cmd::definitions_json(&inspection, &BTreeMap::new())
            .unwrap_or_else(|e| {
                Json::object([
                    ("outcome", Json::text("rejected")),
                    ("code", Json::text(e.code.as_str())),
                    ("message", Json::text(e.message)),
                ])
            });
        let metadata = definitions
            .encode(&beamcraft_protocol::message::read_limits())
            .map_err(|_| invalid("definitions could not be encoded"))?;
        beamcraft_protocol::refresh::encode(
            &bundle,
            &metadata,
            beamcraft_engine::query::bundle::max_bundle_bytes(&beamcraft_engine::Limits::default()),
        )
        .map_err(invalid)?
    } else {
        bundle
    };
    if *out == "-" {
        return Ok(Output::Raw(bundle));
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .map_err(|_| PlateError {
            code: ErrorCode::InvalidArgument,
            message: "the output file exists or cannot be created",
        })?;
    file.write_all(&bundle)
        .and_then(|()| file.sync_all())
        .map_err(|_| PlateError {
            code: ErrorCode::BackendUnavailable,
            message: "the bundle could not be written",
        })?;
    Ok(Output::Json(
        Json::object([
            ("outcome", Json::text("view_exported")),
            (
                "revision_id",
                Json::text(inspection.revision.revision_id.clone()),
            ),
            ("token", Json::text(snapshot.token().as_str())),
            ("elements", Json::text(snapshot.len().to_string())),
            ("meshes", Json::text(snapshot.meshes().len().to_string())),
            ("bytes", Json::text(bundle.len().to_string())),
        ]),
        0,
    ))
}
