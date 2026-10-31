// SPDX-License-Identifier: MPL-2.0
//! Monotonic subprocess watchdog and a crash-safe, cross-worktree file lock.

use crate::{Result, fail, report::Stage};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Set by the executable's interrupt handler; never installs per-test global handlers.
pub static CANCELLED: AtomicBool = AtomicBool::new(false);

/// Acquires a per-user lock shared by repositories and worktrees; process exit releases it.
pub fn lock() -> Result<File> {
    let home = std::env::var_os("XDG_CACHE_HOME")
        .or_else(|| std::env::var_os("LOCALAPPDATA"))
        .or_else(|| std::env::var_os("HOME"))
        .ok_or("no per-user lock directory")?;
    let path = std::path::PathBuf::from(home).join(".beamcraft-test-gate");
    std::fs::create_dir_all(&path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.join("heavy.lock"))?;
    let mut announced = false;
    loop {
        if CANCELLED.load(Ordering::Relaxed) {
            return fail("cancelled while waiting for the gate lock");
        }
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                if !announced {
                    eprintln!(
                        "Another heavy gate is running; waiting before starting any build or test timer."
                    );
                    announced = true;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

/// Terminates a supervised tree, not merely its Cargo parent.
pub fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    let pid = child.id().to_string();
    #[cfg(unix)]
    {
        // Nextest puts individual cases in their own groups. Enumerate descendants before
        // killing the parent so cancellation also reaches those separate groups.
        let listing = crate::capture(Command::new("ps").args(["-A", "-o", "pid=,ppid="]));
        let mut descendants = vec![child.id()];
        if let Ok(listing) = listing {
            let rows: Vec<(u32, u32)> = listing
                .lines()
                .filter_map(|line| {
                    let mut fields = line.split_whitespace();
                    Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
                })
                .collect();
            loop {
                let previous = descendants.len();
                for (id, parent) in &rows {
                    if *id > 1 && descendants.contains(parent) && !descendants.contains(id) {
                        descendants.push(*id);
                    }
                }
                if descendants.len() == previous {
                    break;
                }
            }
        }
        for id in descendants.into_iter().rev() {
            let value = id.to_string();
            let _ = Command::new("sh")
                .args([
                    "-c",
                    "kill -KILL -- -\"$1\" 2>/dev/null; kill -KILL \"$1\" 2>/dev/null; :",
                    "gate-kill",
                    &value,
                ])
                .status();
        }
    }
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs a command with file-backed stdout/stderr; timeout and cancellation retain diagnostics.
pub fn run(name: &str, command: &mut Command, output: &Path, limit: Duration) -> Result<Stage> {
    if CANCELLED.load(Ordering::Relaxed) {
        return Ok(Stage {
            name: name.into(),
            exit_code: 130,
            elapsed_ms: 0,
            timed_out: false,
        });
    }
    std::fs::create_dir_all(output)?;
    let stdout = File::create(output.join(format!("{name}.stdout.log")))?;
    let stderr = File::create(output.join(format!("{name}.stderr.log")))?;
    command.stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    eprintln!("gate: {name}");
    let started = Instant::now();
    let mut child = command.spawn()?;
    loop {
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let interrupted = CANCELLED.load(Ordering::Relaxed);
        if interrupted || started.elapsed() > limit {
            kill_tree(&mut child);
            return Ok(Stage {
                name: name.into(),
                exit_code: if interrupted { 130 } else { 124 },
                elapsed_ms,
                timed_out: !interrupted,
            });
        }
        if let Some(status) = child.try_wait()? {
            return Ok(Stage {
                name: name.into(),
                exit_code: status.code().unwrap_or(1),
                elapsed_ms,
                timed_out: false,
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
