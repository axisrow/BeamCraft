// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Headless CLI using the same native dispatcher as application callers.

mod help;
mod report;
mod revision;
mod session_cmd;

use beamcraft_commands::registry::{self, Route};
use beamcraft_host::{
    control::{Cancellation, Control, HostError},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{ErrorCode, digest::ContentDigest};
use std::collections::BTreeMap;

mod annotation_cmd;
mod assembly_cmd;
mod attribute_cmd;
mod bolt_cmd;
mod bom_cmd;
mod changeset_cmd;
mod command_context;
mod compensate_cmd;
mod construction_cmd;
mod coordination_cmd;
mod coordinator_cmd;
mod drawing_cmd;
mod export_cmd;
mod fastener_cmd;
mod filters_cmd;
mod grid_cmd;
mod history_cmd;
mod ifc_cmd;
mod level_cmd;
mod library_cmd;
mod numbering_cmd;
mod pick_cmd;
mod plate_cmd;
mod project_cmd;
mod rebase_cmd;
mod reference_cmd;
mod resolve_cmd;
mod schedule_cmd;
mod view;
mod view_export;
mod view_screenshot;
mod views_cmd;
mod weld_cmd;
mod work_plane_cmd;

fn invalid() -> HostError {
    HostError::logical(ErrorCode::InvalidArgument)
}
fn main() -> std::process::ExitCode {
    // The steel discipline joins the platform registry first, so every command below reads and
    // authors steel parts; a repeated install is kept, never duplicated.
    steel::layer::install();
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
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
    // `--screenshot FILE` and `--self-test FILE` (with the window's own options) are the window's: the CLI has no
    // window, so it runs `beamcraft-ui` beside it with the same arguments and passes its reply on.
    if args
        .first()
        .is_some_and(|a| a == "--screenshot" || a == "--self-test")
    {
        return screenshot(&args);
    }
    // `command --help` answers with that command's own help instead of a JSON refusal
    // (visual audit finding 11); a `-h` in a value position stays a value. Help needs no
    // store and holds no coordinator lease. It sits above the early-return routes, so the
    // words they own (commands, session, batch words, view export, export) get help too.
    if let Some(word) = args.first().and_then(|w| w.to_str())
        && let Some(command) = registry::COMMANDS.iter().find(|entry| entry.name == word)
        && args
            .get(1..)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .any(|(position, arg)| {
                position % 2 == 0 && arg.to_str().is_some_and(|s| s == "--help" || s == "-h")
            })
    {
        println!("{}", help::for_family(command.family));
        return std::process::ExitCode::SUCCESS;
    }
    let route = args.first().and_then(|c| registry::route(c));
    match route {
        Some(Route::Commands) => return list_commands(args.get(1..).unwrap_or_default()),
        Some(Route::Session) => return session_cmd::execute(&args),
        Some(Route::Batch) => {
            // Parse the options exactly as the session would, so an unlisted one is refused
            // as unknown; a batch word then only exists inside a running session.
            let word = args
                .first()
                .and_then(|w| w.to_str())
                .unwrap_or_default()
                .to_owned();
            let tail = args.get(1..).unwrap_or_default();
            if let Err(refusal) = session_cmd::check_batch_options(&word, tail) {
                let json = Json::object([
                    ("outcome", Json::text("rejected")),
                    ("code", Json::text(refusal.code.as_str())),
                    ("message", Json::text(refusal.message)),
                    ("observed_context", Json::Null),
                    ("diagnostics", Json::Array(vec![])),
                ]);
                return print_json(&json, session_cmd::refusal_exit(refusal.code));
            }
            let error = HostError::logical(ErrorCode::InvalidArgument);
            let json = Json::object([
                ("outcome", Json::text("rejected")),
                ("code", Json::text(error.code.as_str())),
                (
                    "message",
                    Json::text(
                        "this batch word belongs to a running beamcraft session and takes no process of its own",
                    ),
                ),
                ("observed_context", Json::Null),
                ("diagnostics", Json::Array(vec![])),
            ]);
            return print_json(&json, error.exit_code());
        }
        Some(Route::ExportView) => return export_view(&args),
        Some(Route::ScreenshotView) => {
            let (json, exit) = view_screenshot::execute(&args);
            return print_json(&json, exit);
        }
        Some(Route::Export) => {
            // The report lists every element, so it can exceed the control-message bounds that
            // `print_json` enforces; it is rendered by the command itself.
            return export_cmd::run_and_print(&args);
        }
        _ => {}
    }
    // A serving coordinator holds its store for as long as it runs, so a compaction cannot start
    // under it (U14).
    let _serving = match history_cmd::coordinator_lease(&args) {
        Ok(lease) => lease,
        Err((json, exit)) => return print_json(&json, exit),
    };
    // Dispatch is by registry route only; an unregistered word takes the store-level path, which
    // refuses it.
    let (json, exit) = dispatch(route, &args);
    print_json(&json, exit)
}

/// The one-process dispatch by registry route; the session reuses it per line, so a command
/// refuses and answers identically in either mode. `None` takes the store-level path, which
/// refuses an unregistered word.
fn dispatch(route: Option<Route>, args: &[std::ffi::OsString]) -> (Json, u8) {
    match route {
        Some(Route::View) => view::execute(args),
        Some(Route::Views) => views_cmd::execute(args),
        Some(Route::Annotation) => annotation_cmd::execute(args),
        Some(Route::History) => history_cmd::execute(args),
        Some(Route::Reference) => reference_cmd::execute(args),
        Some(Route::Ifc) => ifc_cmd::execute(args),
        Some(Route::Assembly) => assembly_cmd::execute(args),
        Some(Route::Rebase) => rebase_cmd::execute(args),
        Some(Route::Resolve) => resolve_cmd::execute(args),
        Some(Route::Compensate) => compensate_cmd::execute(args),
        Some(Route::Changeset) => changeset_cmd::execute(args),
        Some(Route::Coordinator) => coordinator_cmd::execute(args),
        Some(Route::Drawing) => drawing_cmd::execute(args),
        Some(Route::WorkPlane) => work_plane_cmd::execute(args),
        Some(Route::Pick) => pick_cmd::execute(args),
        Some(Route::Filters) => filters_cmd::execute(args),
        Some(Route::Construction) => construction_cmd::execute(args),
        Some(Route::Weld) => weld_cmd::execute(args),
        Some(Route::Bolt) => bolt_cmd::execute(args),
        Some(Route::Bom) => bom_cmd::execute(args),
        Some(Route::Schedule) => schedule_cmd::execute(args),
        Some(Route::Library) => library_cmd::execute(args),
        Some(Route::Fastener) => fastener_cmd::execute(args),
        Some(Route::Project) => project_cmd::execute(args),
        Some(Route::Grid) => grid_cmd::execute(args),
        Some(Route::Level) => level_cmd::execute(args),
        Some(Route::Coordination) => coordination_cmd::execute(args),
        Some(Route::Attribute) => attribute_cmd::execute(args),
        Some(Route::Numbering) => numbering_cmd::execute(args),
        Some(Route::Plate) => plate_cmd::execute(args),
        // The early returns above already took these four.
        Some(
            Route::Generic
            | Route::Commands
            | Route::ExportView
            | Route::ScreenshotView
            | Route::Export
            | Route::Session
            | Route::Batch,
        )
        | None => run_store_command(args.to_vec()),
    }
}
/// Runs a store-level blob or revision command, or refuses an unregistered word.
fn run_store_command(args: Vec<std::ffi::OsString>) -> (Json, u8) {
    match run(args) {
        Ok(j) => (j, 0),
        Err((e, observed)) => {
            // `open-revision`'s schema refusal: the digests are its diagnostic, not a context.
            let schema = matches!(&observed, Json::Object(map) if map.contains_key("schema"));
            let (message, observed, diagnostics) = if schema {
                (
                    "the store's layer schema digest does not match this build's; pre-alpha \
                     stores are not migrated, so recreate the model with this build",
                    Json::Null,
                    vec![observed],
                )
            } else {
                (e.message(), observed, vec![])
            };
            (
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
                    ("message", Json::text(message)),
                    ("observed_context", observed),
                    ("diagnostics", Json::Array(diagnostics)),
                ]),
                e.exit_code(),
            )
        }
    }
}
/// Runs `export-view`, which writes raw or framed bytes instead of a JSON document.
fn export_view(args: &[std::ffi::OsString]) -> std::process::ExitCode {
    use std::io::Write;
    match view_export::execute(args) {
        view_export::Output::Raw(bytes) => {
            let mut out = std::io::stdout().lock();
            if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
                return std::process::ExitCode::from(3);
            }
            std::process::ExitCode::SUCCESS
        }
        view_export::Output::Framed { head, bundle } => {
            let mut out = std::io::stdout().lock();
            let written = out
                .write_all(&head)
                .and_then(|()| out.write_all(&bundle))
                .and_then(|()| out.flush());
            if written.is_err() {
                return std::process::ExitCode::from(3);
            }
            std::process::ExitCode::SUCCESS
        }
        view_export::Output::Json(json, exit) => print_json(&json, exit),
    }
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
    // `open-revision`'s schema refusal carries the store and build digests as its diagnostic.
    let mut schema_refusal: Option<Json> = None;
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
            let result = match revision::open(&store) {
                Ok(result) => result,
                Err(Ok(error)) => return Err(error),
                Err(Err((error, diagnostic))) => {
                    schema_refusal = Some(diagnostic);
                    return Err(error);
                }
            };
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
    // These commands carry no workspace context to report; a schema refusal carries its
    // diagnostic in the same slot, which `run_store_command` moves to `diagnostics`.
    result.map_err(|e| (e, schema_refusal.unwrap_or(Json::Null)))
}
fn read_file(path: &str, cap: usize) -> Result<Vec<u8>, HostError> {
    beamcraft_commands::source_file::read_file(path, cap).map_err(HostError::logical)
}
/// `commands`: the registry, with no store involved. It takes no options.
fn list_commands(tail: &[std::ffi::OsString]) -> std::process::ExitCode {
    match tail {
        [] => print_json(&beamcraft_commands::registry::listing(), 0),
        _ => {
            let error = invalid();
            let json = Json::object([
                ("outcome", Json::text("rejected")),
                ("code", Json::text(error.code.as_str())),
                ("message", Json::text(error.message())),
                ("observed_context", Json::Null),
                ("diagnostics", Json::Array(vec![])),
            ]);
            print_json(&json, error.exit_code())
        }
    }
}

/// Runs the window program beside this one with `args` (`--screenshot FILE` and the window's
/// options) and returns its exit status; its reply is printed by the window program itself.
fn screenshot(args: &[std::ffi::OsString]) -> std::process::ExitCode {
    let window = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|dir| dir.join(format!("beamcraft-ui{}", std::env::consts::EXE_SUFFIX)))
    });
    let status = window.map(|ui| std::process::Command::new(ui).args(args).status());
    match status {
        Some(Ok(status)) => std::process::ExitCode::from(
            status
                .code()
                .and_then(|c| u8::try_from(c).ok())
                .unwrap_or(1),
        ),
        _ => {
            println!(
                "{{\"outcome\":\"rejected\",\"code\":\"BACKEND_UNAVAILABLE\",\"message\":\"the window program beamcraft-ui was not found beside the CLI\"}}"
            );
            std::process::ExitCode::from(3)
        }
    }
}
