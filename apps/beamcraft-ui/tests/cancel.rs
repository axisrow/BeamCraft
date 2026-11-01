// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Issue 26, finding 9: a lookup that is no longer wanted (Esc, an accepted edit) stops its CLI
//! process at once instead of letting it run to the end, and a lookup given up before it starts
//! never starts one. Its own test binary, so the CLI processes it starts cannot meet another
//! test's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// One random value per process boot: this standalone binary has no shared support module, and
/// a pid alone is not unique across runs (the #157/#408 recycling class). Unix-only like the
/// test it names, so non-unix targets never see it unused.
#[cfg(unix)]
fn boot_nonce() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    static NONCE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *NONCE.get_or_init(|| RandomState::new().hash_one(0u64))
}

#[cfg(unix)]
#[test]
fn a_lookup_that_is_no_longer_wanted_stops_its_process() {
    use beamcraft_ui::command::{self, run_cancellable};
    use beamcraft_ui::stats::Stats;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    let stats = Stats::default();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cancel-{}", boot_nonce()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // A stand-in CLI that never answers; `exec` makes the sleep the process itself, so killing
    // the child ends it.
    let slow = dir.join("slow-cli");
    fs::write(&slow, "#!/bin/sh\nexec sleep 30\n").unwrap();
    fs::set_permissions(&slow, fs::Permissions::from_mode(0o755)).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let giving_up = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        flag.store(true, Ordering::Relaxed);
    });
    let (killed, runs) = (stats.cli_killed.get(), stats.cli_runs.get());
    let started = Instant::now();
    let result = run_cancellable(
        &slow,
        &[],
        command::DEADLINE,
        command::MAX_OUTPUT,
        &cancel,
        &stats,
    );
    giving_up.join().unwrap();
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(stats.cli_runs.get(), runs + 1);
    assert_eq!(stats.cli_killed.get(), killed + 1);
    assert_eq!(stats.cli_active.get(), 0, "the process is gone");

    // Given up before it starts, a lookup starts no process at all.
    let (info, properties) = command::lookup(
        &slow,
        &dir,
        "019a7bf0-0000-7000-8000-000000000001",
        &cancel,
        &stats,
    );
    assert_eq!(info.unwrap_err().code, "CANCELLED");
    assert_eq!(properties.unwrap_err().code, "CANCELLED");
    assert_eq!(stats.cli_runs.get(), runs + 1);
    let _ = fs::remove_dir_all(&dir);
}
