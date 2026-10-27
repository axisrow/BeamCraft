// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Headless CLI using the same native dispatcher as application callers.

mod help;
mod report;
mod revision;

use beamcraft_host::{
    control::{Cancellation, Control, HostError},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{ErrorCode, digest::ContentDigest};
use std::collections::BTreeMap;

mod assembly_cmd;
mod bolt_cmd;
mod bom_cmd;
mod changeset_cmd;
mod command_context;
mod coordinator_cmd;
mod drawing_cmd;
mod export_cmd;
mod grid_cmd;
mod history_cmd;
mod level_cmd;
mod library_cmd;
mod numbering_cmd;
mod pick_cmd;
mod plate_cmd;
mod project_cmd;
mod rebase_cmd;
mod reference_cmd;
mod resolve_cmd;
mod view;
mod view_export;
mod weld_cmd;
mod work_plane_cmd;

fn invalid() -> HostError {
    HostError::logical(ErrorCode::InvalidArgument)
}
fn main() -> std::process::ExitCode {
    let args = std::env::args_os().skip(1).take(50).collect::<Vec<_>>();
    if args.len() == 1 && (args.first().is_some_and(|s| s == "--version" || s == "-V")) {
        println!("beamcraft {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
    if args.is_empty()
        || (args.len() == 1 && args.first().is_some_and(|s| s == "--help" || s == "-h"))
    {
        help::print();
        return std::process::ExitCode::SUCCESS;
    }
    if args.first().is_some_and(|c| c == "export-view") {
        match view_export::execute(&args) {
            view_export::Output::Raw(bytes) => {
                use std::io::Write;
                let mut out = std::io::stdout().lock();
                if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
                    return std::process::ExitCode::from(3);
                }
                return std::process::ExitCode::SUCCESS;
            }
            view_export::Output::Framed { head, bundle } => {
                use std::io::Write;
                let mut out = std::io::stdout().lock();
                let written = out
                    .write_all(&head)
                    .and_then(|()| out.write_all(&bundle))
                    .and_then(|()| out.flush());
                if written.is_err() {
                    return std::process::ExitCode::from(3);
                }
                return std::process::ExitCode::SUCCESS;
            }
            view_export::Output::Json(json, exit) => return print_json(&json, exit),
        }
    }
    if args.first().is_some_and(export_cmd::handles) {
        // The report lists every element, so it can exceed the control-message bounds that
        // `print_json` enforces; it is rendered by the command itself.
        return export_cmd::run_and_print(&args);
    }
    // A serving coordinator holds its store for as long as it runs, so a compaction cannot start
    // under it (U14).
    let _serving = match history_cmd::coordinator_lease(&args) {
        Ok(lease) => lease,
        Err((json, exit)) => return print_json(&json, exit),
    };
    let (json, exit) = if args
        .first()
        .is_some_and(|c| c == "snapshot-view" || c == "inspect-view-changes")
    {
        view::execute(&args)
    } else if args.first().is_some_and(history_cmd::handles) {
        history_cmd::execute(&args)
    } else if args.first().is_some_and(|c| reference_cmd::handles(c)) {
        reference_cmd::execute(&args)
    } else if args.first().is_some_and(|c| {
        c == "create-assembly"
            || c == "attach-to-assembly"
            || c == "detach-from-assembly"
            || c == "set-main-part"
            || c == "explode-assembly"
            || c == "assemblies"
    }) {
        assembly_cmd::execute(&args)
    } else if args.first().is_some_and(|c| c == "rebase-workspace") {
        rebase_cmd::execute(&args)
    } else if args.first().is_some_and(|c| c == "resolve-workspace") {
        resolve_cmd::execute(&args)
    } else if args.first().is_some_and(|c| {
        c == "create-changeset" || c == "inspect-changeset" || c == "apply-changeset"
    }) {
        changeset_cmd::execute(&args)
    } else if args.first().is_some_and(coordinator_cmd::handles) {
        coordinator_cmd::execute(&args)
    } else if args.first().is_some_and(drawing_cmd::handles) {
        drawing_cmd::execute(&args)
    } else if args.first().is_some_and(|c| c == "work-plane") {
        work_plane_cmd::execute(&args)
    } else if args.first().is_some_and(pick_cmd::handles) {
        pick_cmd::execute(&args)
    } else if args.first().is_some_and(weld_cmd::handles) {
        weld_cmd::execute(&args)
    } else if args.first().is_some_and(bolt_cmd::handles) {
        bolt_cmd::execute(&args)
    } else if args.first().is_some_and(bom_cmd::handles) {
        bom_cmd::execute(&args)
    } else if args.first().is_some_and(library_cmd::handles) {
        library_cmd::execute(&args)
    } else if args.first().is_some_and(project_cmd::handles) {
        project_cmd::execute(&args)
    } else if args.first().is_some_and(grid_cmd::handles) {
        grid_cmd::execute(&args)
    } else if args.first().is_some_and(level_cmd::handles) {
        level_cmd::execute(&args)
    } else if args.first().is_some_and(numbering_cmd::handles) {
        numbering_cmd::execute(&args)
    } else if args.first().is_some_and(|c| {
        c == "create-plate"
            || c == "inspect-elements"
            || c == "define-material"
            || c == "define-profile"
            || c == "definitions"
            || c == "create-member"
            || c == "add-hole"
            || c == "update-element"
            || c == "delete-element"
            || c == "copy-element"
            || c == "cut-by-plane"
            || c == "create-workspace"
            || c == "inspect-workspace"
            || c == "publish-workspace"
            || c == "undo-workspace"
            || c == "redo-workspace"
            || c == "inspect-workspace-history"
    }) {
        plate_cmd::execute(&args)
    } else {
        match run(args) {
            Ok(j) => (j, 0),
            Err((e, observed)) => (
                Json::object([
                    (
                        "outcome",
                        Json::text(if e.code == ErrorCode::OutcomeUnknown {
                            "outcome_unknown"
                        } else if e.code == ErrorCode::Cancelled {
                            "cancelled"
                        } else {
                            "rejected"
                        }),
                    ),
                    ("code", Json::text(e.code.as_str())),
                    ("message", Json::text(e.message())),
                    ("observed_context", observed),
                    ("diagnostics", Json::Array(vec![])),
                ]),
                e.exit_code(),
            ),
        }
    };
    print_json(&json, exit)
}
fn print_json(json: &Json, exit: u8) -> std::process::ExitCode {
    match json.encode(&Limits::DEFAULT) {
        Ok(bytes) => {
            use std::io::Write;
            if std::io::stdout().lock().write_all(&bytes).is_err() {
                return std::process::ExitCode::from(3);
            }
        }
        Err(_) => {
            println!("{{\"outcome\":\"rejected\",\"code\":\"LIMIT_EXCEEDED\"}}");
            return std::process::ExitCode::from(2);
        }
    }
    std::process::ExitCode::from(exit)
}
fn run(args: Vec<std::ffi::OsString>) -> Result<Json, (HostError, Json)> {
    let result = (|| -> Result<Json, HostError> {
        if args.len() > 49 {
            return Err(invalid());
        }
        let args = args
            .into_iter()
            .map(|s| s.into_string().map_err(|_| invalid()))
            .collect::<Result<Vec<_>, _>>()?;
        if args.iter().any(|s| s.len() > 4096) {
            return Err(invalid());
        }
        let command = args.first().ok_or_else(invalid)?;
        let tail = args.get(1..).ok_or_else(invalid)?;
        if !tail.len().is_multiple_of(2) {
            return Err(invalid());
        }
        let mut options = BTreeMap::new();
        for pair in tail.as_chunks::<2>().0 {
            let [key, value] = pair;
            if !matches!(
                key.as_str(),
                "--store"
                    | "--project-id"
                    | "--workspace-id"
                    | "--generation"
                    | "--file"
                    | "--digest"
                    | "--timeout-ms"
                    | "--cancel-after-ms"
                    | "--op-id"
                    | "--revision-id"
                    | "--base"
                    | "--schema"
                    | "--root-layer"
                    | "--layers"
                    | "--min-age-secs"
            ) || options.insert(key.as_str(), value.as_str()).is_some()
            {
                return Err(invalid());
            }
        }
        let get = |k: &str| options.get(k).copied().ok_or_else(invalid);
        let timeout = options
            .get("--timeout-ms")
            .map(|s| s.parse::<u64>().map_err(|_| invalid()))
            .transpose()?;
        let cancellation = Cancellation::default();
        if let Some(ms) = options.get("--cancel-after-ms") {
            let ms = ms.parse::<u64>().map_err(|_| invalid())?;
            if ms > 120000 {
                return Err(invalid());
            }
            if ms == 0 {
                cancellation.cancel();
            } else {
                let token = cancellation.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    token.cancel();
                });
            }
        }
        let control = Control::new(timeout, cancellation)?;
        control.remaining()?;
        let store = BlobStore::new(get("--store")?).map_err(|e| HostError::logical(e.code()))?;
        let parse_digest = |s: &str| s.parse::<ContentDigest>().map_err(|_| invalid());
        if command == "put-blob" {
            let bytes = read_file(get("--file")?, 32 * 1024 * 1024)?;
            let (digest, _) = store
                .publish(&bytes)
                .map_err(|e| HostError::logical(e.code()))?;
            control.remaining()?;
            return Ok(Json::object([
                ("outcome", Json::text("blob_stored")),
                ("digest", Json::text(digest.to_string())),
            ]));
        }
        if command == "verify-blob" {
            let digest = parse_digest(get("--digest")?)?;
            store
                .verify(&digest)
                .map_err(|e| HostError::logical(e.code()))?;
            control.remaining()?;
            return Ok(Json::object([
                ("outcome", Json::text("blob_verified")),
                ("digest", Json::text(digest.to_string())),
            ]));
        }
        if command == "commit-revision" {
            // The deadline and cancellation are checked before and during the commit (not
            // after the root switch: a timeout would hide a committed revision). A retry with
            // the same op id returns the revision.
            return revision::commit(&store, &options, &control);
        }
        if command == "open-revision" {
            let result = revision::open(&store)?;
            control.remaining()?;
            return Ok(result);
        }
        // Recovery and outcome queries only restate what the root proves; a deadline after the
        // work would hide a result that is already durable, so none is checked afterwards.
        if command == "recover-revision" {
            return revision::recover(&store, &options);
        }
        if command == "inspect-revision-outcome" {
            return revision::outcome(&store, &options);
        }
        Err(HostError::logical(ErrorCode::UnsupportedCommand))
    })();
    // These commands carry no workspace context to report.
    result.map_err(|e| (e, Json::Null))
}
fn read_file(path: &str, cap: usize) -> Result<Vec<u8>, HostError> {
    beamcraft_cli::source_file::read_file(path, cap).map_err(HostError::logical)
}
