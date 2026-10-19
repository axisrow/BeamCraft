// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Running a `beamcraft` CLI command in a separate process: the create tools (V05.1), the edit
//! tools (V05.2) and the properties and publication queries of V06. The UI commits through the
//! same command a user types, so the accepted revision is the engine's, never a copy made here.
//! Output is read with a cap and the process is killed after a deadline.

use crate::edit::{
    Edit, Edited, ElementInfo, Ray, inspect_args, parse_edited, parse_element_info, parse_pick,
    pick_args,
};
use crate::props::{
    Change, Properties, WorkspaceState, history_args, parse_change, parse_element, parse_history,
    parse_workspace_state, workspace_args,
};
use crate::tools::{
    Created, Definitions, Prepared, ToolError, definitions_args, parse_created, parse_definitions,
};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The longest a command may run before it is killed.
pub const DEADLINE: Duration = Duration::from_secs(120);

/// The most standard output read from one command.
pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;

/// A finished process: its exit status (`None` when it was ended by a signal) and its output.
#[derive(Debug)]
pub struct Output {
    /// The exit status.
    pub code: Option<i32>,
    /// Standard output, capped at [`MAX_OUTPUT`].
    pub stdout: Vec<u8>,
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
pub fn run(cli: &Path, args: &[String], deadline: Duration) -> Result<Output, ToolError> {
    run_capped(cli, args, deadline, MAX_OUTPUT)
}

/// Like [`run`] with an explicit output cap (more than `cap` bytes of output is an error).
pub fn run_capped(
    cli: &Path,
    args: &[String],
    deadline: Duration,
    cap: usize,
) -> Result<Output, ToolError> {
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
    let reader = drain(child.stdout.take(), cap);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
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
pub fn commit(cli: &Path, prepared: &Prepared) -> Result<Created, ToolError> {
    let output = run(cli, &prepared.args, DEADLINE)?;
    parse_created(prepared.kind, output.code, &output.stdout)
}

/// Lists the profile and material definitions of the store's accepted revision.
pub fn definitions(cli: &Path, store: &Path) -> Result<Definitions, ToolError> {
    let output = run(cli, &definitions_args(store), DEADLINE)?;
    parse_definitions(output.code, &output.stdout)
}

/// Runs a previewed edit (or an undo or redo) and reads its reply.
pub fn commit_edit(cli: &Path, edit: &Edit) -> Result<Edited, ToolError> {
    let output = run(cli, &edit.args, DEADLINE)?;
    parse_edited(edit.kind, output.code, &output.stdout)
}

/// The most standard output read from `plate-inspect` when one element is looked up.
const MAX_INSPECT_OUTPUT: usize = 32 * 1024 * 1024;

/// What a click found: the element id the engine's `pick` returned and, apart from it, whether
/// the element's details could be read (a failure there never loses the selection).
#[derive(Debug)]
pub struct Selected {
    /// The picked element.
    pub id: String,
    /// Its kind, name and translation as `plate-inspect` reports them.
    pub info: Result<ElementInfo, ToolError>,
}

/// Looks up one element of the accepted revision.
pub fn element_info(cli: &Path, store: &Path, id: &str) -> Result<ElementInfo, ToolError> {
    let output = run_capped(cli, &inspect_args(store), DEADLINE, MAX_INSPECT_OUTPUT)?;
    parse_element_info(id, output.code, &output.stdout)
}

/// Picks along `ray` with the engine's `pick` and reads the details of what it hit. `None` is
/// empty space.
pub fn select_at(cli: &Path, store: &Path, ray: &Ray) -> Result<Option<Selected>, ToolError> {
    let output = run(cli, &pick_args(store, ray), DEADLINE)?;
    let Some(id) = parse_pick(output.code, &output.stdout)? else {
        return Ok(None);
    };
    let info = element_info(cli, store, &id);
    Ok(Some(Selected { id, info }))
}

/// Reads the full properties of one element from the store's inspection; `None` when the store
/// has no such element. The properties panel's report; the editor's lighter lookup is
/// [`element_info`].
pub fn properties(cli: &Path, store: &Path, id: &str) -> Result<Option<Properties>, ToolError> {
    let output = run_capped(cli, &inspect_args(store), DEADLINE, MAX_INSPECT_OUTPUT)?;
    parse_element(output.code, &output.stdout, id)
}

/// Runs one argument list that changes something the edit panel does not stage (`update-element`
/// with a property patch, `workspace-publish`) and reads its reply.
pub fn run_change(cli: &Path, args: &[String]) -> Result<Change, ToolError> {
    let output = run(cli, args, DEADLINE)?;
    parse_change(output.code, &output.stdout)
}

/// Reads where the draft stands against the published head.
pub fn workspace_state(
    cli: &Path,
    published: &Path,
    dir: &Path,
) -> Result<WorkspaceState, ToolError> {
    let output = run(
        cli,
        &workspace_args("workspace-status", published, dir),
        DEADLINE,
    )?;
    parse_workspace_state(output.code, &output.stdout)
}

/// Reads the draft's undo and redo units.
pub fn workspace_history(cli: &Path, dir: &Path) -> Result<(u64, u64), ToolError> {
    let output = run(cli, &history_args(dir), DEADLINE)?;
    parse_history(output.code, &output.stdout)
}
