// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V03 headless command `export-view`: the pinned view snapshot of the verified head revision as
//! one engine bundle (`beamcraft_engine::query::bundle`), the input of the native viewport.
//!
//! `--out -` writes the raw bundle to standard output and nothing else (the viewport reads it
//! through a pipe); `--out FILE` creates a new file (never overwrites) and prints a JSON summary.
//!
//! `--with-definitions true` and `--with-element ID` (only with `--out -`) add replies taken from
//! the same verified inspection, so one process and one inspection serve a whole viewport
//! refresh and everything it shows belongs to the exported revision (issue 26, finding 2).
//! Standard output is then a frame: [`FRAME_MAGIC`]; the length of the definitions reply as a
//! little-endian `u32` and that reply byte for byte as `beamcraft definitions --store ROOT`
//! prints it; the length and the reply of `beamcraft inspect-elements --store ROOT --element ID`
//! the same way; then the bundle. A length of zero is a reply that was not asked for.
//! `--with-grids true` selects [`GRID_FRAME_MAGIC`] and adds a third length-prefixed reply,
//! containing `list-grids` display data from that same inspection, before the bundle.

use crate::plate_cmd::{definitions_json, id_error, inspection_json};
use crate::report::report;
use beamcraft_commands::plate::inspect;
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use session::Refusal;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;

/// The first bytes of a framed reply (`--with-definitions true` or `--with-element ID`).
pub const FRAME_MAGIC: &[u8; 8] = b"BCVXFRM1";
/// Version 2 adds pinned grid display data as a third length-prefixed reply.
pub const GRID_FRAME_MAGIC: &[u8; 8] = b"BCVXFRM2";

/// What `execute` produced.
pub enum Output {
    /// A JSON object for standard output, and the exit code.
    Json(Json, u8),
    /// Raw bundle bytes for standard output.
    Raw(Vec<u8>),
    /// A frame: the magic and bounded length-prefixed replies, then the bundle.
    Framed {
        /// Magic, lengths and replies.
        head: Vec<u8>,
        /// The bundle.
        bundle: Vec<u8>,
    },
}

/// A reply exactly as the command that owns it prints it (the CLI's `print_json`).
fn printed(result: Result<Json, Refusal>) -> Vec<u8> {
    let (json, _) = report(result);
    json.encode(&beamcraft_json::Limits::DEFAULT)
        .unwrap_or_else(|_| {
            // What `print_json` prints when a reply exceeds the encoder budget.
            b"{\"outcome\":\"rejected\",\"code\":\"LIMIT_EXCEEDED\"}\n".to_vec()
        })
}

/// Appends `reply` with its length; `None` is a reply that was not asked for.
fn section(head: &mut Vec<u8>, reply: Option<&[u8]>) -> Result<(), Refusal> {
    let reply = reply.unwrap_or_default();
    let length = u32::try_from(reply.len()).map_err(|_| Refusal {
        rule_id: None,
        code: ErrorCode::LimitExceeded,
        message: "a framed reply is too large",
    })?;
    head.extend_from_slice(&length.to_le_bytes());
    head.extend_from_slice(reply);
    Ok(())
}

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs `export-view`.
pub fn execute(args: &[OsString]) -> Output {
    match run(args) {
        Ok(output) => output,
        Err(e) => Output::Json(
            Json::object([
                ("outcome", Json::text("rejected")),
                ("code", Json::text(e.code.as_str())),
                ("message", Json::text(e.message)),
                ("observed_context", Json::Null),
                (
                    "diagnostics",
                    Json::Array(crate::plate_cmd::domain_diagnostics(&e)),
                ),
            ]),
            match e.code {
                ErrorCode::BackendUnavailable => 3,
                _ => 2,
            },
        ),
    }
}

fn run(args: &[OsString]) -> Result<Output, Refusal> {
    let args: Vec<&str> = args
        .iter()
        .skip(1)
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Result<_, _>>()?;
    if !args.len().is_multiple_of(2) || args.len() > 10 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in args.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !matches!(
            *key,
            "--store" | "--out" | "--with-definitions" | "--with-element" | "--with-grids"
        ) || options.insert(*key, *value).is_some()
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
    let with_definitions = match options.get("--with-definitions") {
        None | Some(&"false") => false,
        Some(&"true") => true,
        Some(_) => return Err(invalid("--with-definitions is true or false")),
    };
    let with_element = match options.get("--with-element") {
        Some(text) => Some(ElementId::parse(text).map_err(id_error)?),
        None => None,
    };
    let with_grids = match options.get("--with-grids") {
        None | Some(&"false") => false,
        Some(&"true") => true,
        Some(_) => return Err(invalid("--with-grids is true or false")),
    };
    let framed = with_definitions || with_element.is_some() || with_grids;
    // Checked before the store is opened, so a refused request never creates the file.
    if framed && *out != "-" {
        return Err(invalid("a framed reply needs --out -"));
    }
    let blobs = BlobStore::new(store).map_err(|e| Refusal {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let inspection = inspect(&RevisionStore::new(blobs))?;
    inspection.require_evaluated_geometry()?;
    if let Some(error) = inspection.reference_view_error {
        return Err(error);
    }
    let snapshot = &inspection.snapshot;
    let bundle = snapshot.to_bundle().map_err(|e| Refusal {
        rule_id: None,
        code: e.f04_code(),
        message: "the view snapshot could not be bundled",
    })?;
    if framed {
        let definitions =
            with_definitions.then(|| printed(definitions_json(&inspection, &BTreeMap::new())));
        let element = with_element.map(|id| printed(inspection_json(&inspection, Some(id))));
        let mut head = if with_grids {
            GRID_FRAME_MAGIC
        } else {
            FRAME_MAGIC
        }
        .to_vec();
        section(&mut head, definitions.as_deref())?;
        section(&mut head, element.as_deref())?;
        if with_grids {
            let grids = printed(
                crate::grid_cmd::views_json(&inspection.grids).and_then(|grids| {
                    let axes = steel::engine::snap::member_snap_axes(&inspection.members)
                        .map_err(|_| invalid("member snap reference limit exceeded"))?;
                    let axes = axes
                        .iter()
                        .map(|axis| {
                            let points = axis
                                .points_mm
                                .iter()
                                .map(|point| {
                                    point
                                        .iter()
                                        .map(|v| crate::plate_cmd::num(*v))
                                        .collect::<Result<Vec<_>, _>>()
                                        .map(Json::Array)
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            Ok(Json::object([
                                ("element_id", Json::text(axis.element_id.to_string())),
                                ("points_mm", Json::Array(points)),
                            ]))
                        })
                        .collect::<Result<Vec<_>, Refusal>>()?;
                    Ok(Json::object([
                        ("outcome", Json::text("grids")),
                        ("grids", grids),
                        ("member_axes", Json::Array(axes)),
                    ]))
                }),
            );
            section(&mut head, Some(&grids))?;
        }
        return Ok(Output::Framed { head, bundle });
    }
    if *out == "-" {
        return Ok(Output::Raw(bundle));
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .map_err(|_| Refusal {
            rule_id: None,
            code: ErrorCode::InvalidArgument,
            message: "the output file exists or cannot be created",
        })?;
    file.write_all(&bundle)
        .and_then(|()| file.sync_all())
        .map_err(|_| Refusal {
            rule_id: None,
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
