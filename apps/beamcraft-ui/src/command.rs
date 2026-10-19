// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Running a `beamcraft` CLI command in a separate process for the tools. The UI commits
//! through the same command a user types, so the accepted revision is the engine's, never a copy
//! made here. Output is read with a cap and the process is killed after a deadline.

use crate::tools::{
    Definitions, EditResult, Prepared, ToolError, ToolKind, definitions_args, parse_created,
    parse_definitions, parse_deleted, parse_updated,
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

/// Runs a previewed tool command and reads its reply by the tool's kind.
pub fn run_tool(cli: &Path, prepared: &Prepared) -> Result<EditResult, ToolError> {
    let output = run(cli, &prepared.args, DEADLINE)?;
    match prepared.kind {
        ToolKind::Plate | ToolKind::Member => {
            parse_created(prepared.kind, output.code, &output.stdout).map(EditResult::Created)
        }
        ToolKind::Move => parse_updated(output.code, &output.stdout).map(EditResult::Updated),
        ToolKind::Delete => parse_deleted(output.code, &output.stdout).map(EditResult::Deleted),
    }
}

/// Lists the profile and material definitions of the store's accepted revision.
pub fn definitions(cli: &Path, store: &Path) -> Result<Definitions, ToolError> {
    let output = run(cli, &definitions_args(store), DEADLINE)?;
    parse_definitions(output.code, &output.stdout)
}
