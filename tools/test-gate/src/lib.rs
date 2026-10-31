// SPDX-License-Identifier: MPL-2.0
//! Recorded test-gate policy, bounded reports and subprocess supervision.

pub mod github;
pub mod local;
pub mod process;
pub mod report;

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

/// One diagnostic returned by the gate instead of panicking.
pub type Error = Box<dyn std::error::Error + Send + Sync>;
/// Fallible gate operation.
pub type Result<T> = std::result::Result<T, Error>;
/// Maximum accepted JSON, inventory or command response (32 MiB).
pub const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// Creates a bounded diagnostic.
pub fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(std::io::Error::other(message.into()).into())
}

/// Reads a bounded UTF-8 file.
pub fn read(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len())? > MAX_BYTES {
        return fail("input exceeds 32 MiB");
    }
    Ok(String::from_utf8(bytes)?)
}

/// Captures a short command response without allowing unbounded output allocation.
pub fn capture(command: &mut Command) -> Result<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut bytes = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        stdout.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    }
    if u64::try_from(bytes.len())? > MAX_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        return fail("command response exceeds 32 MiB");
    }
    let status = child.wait()?;
    if !status.success() {
        return fail(format!("command failed ({status}): {command:?}"));
    }
    Ok(String::from_utf8(bytes)?.trim().to_owned())
}

/// Executes Git in the selected source checkout.
pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    capture(Command::new("git").current_dir(root).args(args))
}
