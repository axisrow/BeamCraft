// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Running a `beamcraft` CLI command in a separate process for the create tools. The UI commits
//! through the same command a user types, so the accepted revision is the engine's, never a copy
//! made here. Output is read with a cap and the process is killed after a deadline; a read-only
//! lookup is also killed as soon as its answer is no longer wanted.

use crate::edit::{
    Edit, EditKind, Edited, ElementInfo, Ray, inspect_element_args, parse_edited,
    parse_element_info, pick_in,
};
use crate::props::{Details, PropertyEdit, parse_details};
use crate::publish::{Published, Status, parse_published, parse_status, status_args};
use crate::stats::Stats;
use crate::tools::{
    Created, Definitions, Prepared, ToolError, definitions_args, parse_created, parse_definitions,
};
use beamcraft_engine::ViewSnapshot;
use beamcraft_model::id::ElementId;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The longest a command may run before it is killed.
pub const DEADLINE: Duration = Duration::from_secs(120);

/// The most standard output read from one command.
pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;

/// The cancel flag of a query nobody gives up.
static NEVER: AtomicBool = AtomicBool::new(false);

/// A finished process: its exit status (`None` when it was ended by a signal) and its output.
#[derive(Debug)]
pub struct Output {
    /// The exit status.
    pub code: Option<i32>,
    /// Standard output, capped at [`MAX_OUTPUT`].
    pub stdout: Vec<u8>,
}

/// Counts a running CLI process for the perf counters until it is dropped.
struct Active<'a>(&'a Stats);

impl<'a> Active<'a> {
    fn new(stats: &'a Stats) -> Self {
        stats.cli_runs.add(1);
        stats.cli_active.add(1);
        Self(stats)
    }
}

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.cli_active.decrement();
    }
}

fn drain(
    stream: Option<impl Read + Send + 'static>,
    cap: usize,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stream) = stream {
            let cap = u64::try_from(cap).unwrap_or(u64::MAX);
            let mut limited = stream.take(cap.saturating_add(1));
            if limited.read_to_end(&mut bytes).is_err() {
                bytes.clear();
            }
            // Keep reading so the child never blocks on a full pipe.
            let _ = std::io::copy(&mut limited.into_inner(), &mut std::io::sink());
        }
        bytes
    })
}

/// Runs `cli args...` and waits for it, at most `deadline`; output is capped at [`MAX_OUTPUT`].
pub fn run(
    cli: &Path,
    args: &[String],
    deadline: Duration,
    stats: &Stats,
) -> Result<Output, ToolError> {
    run_capped(cli, args, deadline, MAX_OUTPUT, stats)
}

/// Like [`run`] with an explicit output cap (more than `cap` bytes of output is an error).
pub fn run_capped(
    cli: &Path,
    args: &[String],
    deadline: Duration,
    cap: usize,
    stats: &Stats,
) -> Result<Output, ToolError> {
    run_cancellable(cli, args, deadline, cap, &NEVER, stats)
}

/// Like [`run_capped`], and given up as soon as `cancel` is set: the process is killed (or never
/// started) and the answer is `CANCELLED`. Only for read-only queries, whose answer can simply be
/// dropped; a change command is never cancelled.
#[allow(clippy::too_many_arguments)]
pub fn run_cancellable(
    cli: &Path,
    args: &[String],
    deadline: Duration,
    cap: usize,
    cancel: &AtomicBool,
    stats: &Stats,
) -> Result<Output, ToolError> {
    let cancelled = || {
        ToolError::local(
            "CANCELLED",
            "the lookup was no longer wanted and was stopped",
        )
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    let mut command = Command::new(cli);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes for the console-subsystem CLI.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| {
        ToolError::local(
            "CLI_UNAVAILABLE",
            format!("the beamcraft CLI could not be started ({})", e.kind()),
        )
    })?;
    let _active = Active::new(stats);
    let reader = drain(child.stdout.take(), cap);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                stats.cli_ns.add_duration(started.elapsed());
                break status;
            }
            Ok(None) if cancel.load(Ordering::Relaxed) => {
                let _ = child.kill();
                let _ = child.wait();
                stats.cli_ns.add_duration(started.elapsed());
                stats.cli_killed.add(1);
                let _ = reader.join();
                return Err(cancelled());
            }
            Ok(None) if started.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                stats.cli_ns.add_duration(started.elapsed());
                let _ = reader.join();
                return Err(ToolError::local(
                    "OUTCOME_UNKNOWN",
                    "the CLI did not finish in time and was stopped (reload to see whether the change was accepted)",
                ));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(ToolError::local(
                    "CLI_FAILED",
                    format!("waiting for the CLI failed ({})", e.kind()),
                ));
            }
        }
    };
    let stdout = reader.join().unwrap_or_default();
    if stdout.len() > cap {
        return Err(ToolError::local(
            "CLI_FAILED",
            "the CLI reply exceeds the size limit",
        ));
    }
    Ok(Output {
        code: status.code(),
        stdout,
    })
}

/// Runs a previewed create command and reads its reply.
pub fn commit(cli: &Path, prepared: &Prepared, stats: &Stats) -> Result<Created, ToolError> {
    let output = run(cli, &prepared.args, DEADLINE, stats)?;
    parse_created(prepared.kind, output.code, &output.stdout)
}

/// Lists the profile and material definitions of the store's accepted revision.
pub fn definitions(cli: &Path, store: &Path, stats: &Stats) -> Result<Definitions, ToolError> {
    let output = run(cli, &definitions_args(store), DEADLINE, stats)?;
    parse_definitions(output.code, &output.stdout)
}

/// Runs a previewed edit (or an undo or redo) and reads its reply.
pub fn commit_edit(cli: &Path, edit: &Edit, stats: &Stats) -> Result<Edited, ToolError> {
    let output = run(cli, &edit.args, DEADLINE, stats)?;
    parse_edited(edit.kind, output.code, &output.stdout)
}

/// What a click found: the element id the engine's `pick` returned and, apart from it, whether
/// the element's details could be read (a failure there never loses the selection).
#[derive(Debug)]
pub struct Selected {
    /// The picked element.
    pub id: String,
    /// Its kind, name and translation as `inspect-elements` reports them.
    pub info: Result<ElementInfo, ToolError>,
    /// All its reported properties and the revision they were read from.
    pub details: Result<Details, ToolError>,
}

impl Selected {
    /// The selection of `id` from one reading of its details.
    pub fn from_details(id: String, details: Result<Details, ToolError>) -> Self {
        Self {
            id,
            info: details.clone().map(|d| d.info()),
            details,
        }
    }
}

/// Looks up one element of the accepted revision.
pub fn element_info(
    cli: &Path,
    store: &Path,
    id: &str,
    stats: &Stats,
) -> Result<ElementInfo, ToolError> {
    lookup(cli, store, id, &NEVER, stats).0
}

/// Reads the properties of one element of the accepted revision, with the revision they come from.
pub fn element_details(
    cli: &Path,
    store: &Path,
    id: &str,
    stats: &Stats,
) -> Result<Details, ToolError> {
    lookup(cli, store, id, &NEVER, stats).1
}

/// [`element_info`] and [`element_details`] of one element from one `inspect-elements --element ID`:
/// one process and one inspection, with a reply of one element whatever the size of the model.
/// `cancel` gives it up ([`run_cancellable`]).
pub fn lookup(
    cli: &Path,
    store: &Path,
    id: &str,
    cancel: &AtomicBool,
    stats: &Stats,
) -> (Result<ElementInfo, ToolError>, Result<Details, ToolError>) {
    // An id no revision can hold is not in this one (the CLI would refuse it as malformed).
    if ElementId::parse(id).is_err() {
        let gone = ToolError::local("NOT_FOUND", "the element is not in the accepted revision");
        return (Err(gone.clone()), Err(gone));
    }
    match run_cancellable(
        cli,
        &inspect_element_args(store, id),
        DEADLINE,
        MAX_OUTPUT,
        cancel,
        stats,
    ) {
        Ok(output) => (
            parse_element_info(id, output.code, &output.stdout),
            parse_details(id, output.code, &output.stdout),
        ),
        Err(e) => (Err(e.clone()), Err(e)),
    }
}

/// Selects along `ray`: the engine's pick on `snapshot`, the snapshot the window shows, in this
/// process ([`pick_in`]), then [`lookup`] of the element it hit. A click costs one CLI process
/// and one inspection; `cancel` gives the lookup up.
pub fn select(
    cli: &Path,
    store: &Path,
    snapshot: &ViewSnapshot,
    ray: &Ray,
    cancel: &AtomicBool,
    stats: &Stats,
) -> Result<Option<Selected>, ToolError> {
    let Some(id) = pick_in(snapshot, ray)? else {
        return Ok(None);
    };
    let (info, details) = lookup(cli, store, &id, cancel, stats);
    Ok(Some(Selected { id, info, details }))
}

/// Runs a previewed property edit (`update-element`) and reads its reply.
pub fn commit_property(
    cli: &Path,
    edit: &PropertyEdit,
    stats: &Stats,
) -> Result<Edited, ToolError> {
    let output = run(cli, &edit.args, DEADLINE, stats)?;
    // The reply of `update-element` is the one a move reads: outcome `element_updated`.
    parse_edited(EditKind::Move, output.code, &output.stdout)
}

/// Runs a confirmed `publish-workspace` and reads its reply.
pub fn publish(cli: &Path, args: &[String], stats: &Stats) -> Result<Published, ToolError> {
    let output = run(cli, args, DEADLINE, stats)?;
    parse_published(output.code, &output.stdout)
}

/// Reads `inspect-workspace` for a published store and a workspace directory.
pub fn workspace_status(
    cli: &Path,
    published: &Path,
    workspace: &Path,
    stats: &Stats,
) -> Result<Status, ToolError> {
    let output = run(cli, &status_args(published, workspace), DEADLINE, stats)?;
    parse_status(output.code, &output.stdout)
}
