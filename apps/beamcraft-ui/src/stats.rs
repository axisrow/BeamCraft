// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Process-wide counters of the work the window does: frames, viewport draws, GPU uploads,
//! pipeline and target builds, CLI processes. They are plain relaxed atomics, always on and too
//! cheap to measure; only `--perf-run` reads them (see [`crate::perf`]). They count work, they
//! never decide it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// One relaxed counter or gauge.
#[derive(Debug)]
pub struct Counter(AtomicU64);

impl Counter {
    /// A counter at zero.
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Adds `n`.
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    /// Adds the whole nanoseconds of `elapsed`.
    pub fn add_duration(&self, elapsed: Duration) {
        self.add(u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX));
    }

    /// Replaces the value (gauges).
    pub fn set(&self, n: u64) {
        self.0.store(n, Ordering::Relaxed);
    }

    /// Subtracts one, never below zero (gauges).
    pub fn decrement(&self) {
        let _ = self
            .0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1));
    }

    /// The current value.
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

/// Every counter of the process.
#[derive(Debug, Default)]
pub struct Stats {
    /// Window frames (`App::ui` calls).
    pub frames: Counter,
    /// 3D viewport passes encoded and submitted.
    pub viewport_draws: Counter,
    /// Queue submissions of the viewport (draws and uploads).
    pub submits: Counter,
    /// Scene uploads to the device.
    pub uploads: Counter,
    /// Vertex and instance bytes written by those uploads.
    pub upload_bytes: Counter,
    /// UI-thread nanoseconds spent uploading a scene.
    pub upload_ui_ns: Counter,
    /// Shader and render pipeline builds.
    pub pipeline_builds: Counter,
    /// Offscreen target (texture set) builds.
    pub target_builds: Counter,
    /// UI-thread nanoseconds spent resizing the offscreen target.
    pub resize_ns: Counter,
    /// CLI processes started.
    pub cli_runs: Counter,
    /// Wall-clock nanoseconds of those processes, start to exit.
    pub cli_ns: Counter,
    /// CLI processes killed because their result was no longer wanted.
    pub cli_killed: Counter,
    /// Gauge: CLI processes running now.
    pub cli_active: Counter,
    /// Refresh results applied by the window.
    pub reloads_applied: Counter,
    /// Current selection answers with an error in the pick, the details or the properties.
    pub select_errors: Counter,
    /// Gauge: CPU bytes of display buffers (expanded vertices, instances) the loaded scene keeps.
    pub retained_display_bytes: Counter,
    /// Gauge: mesh blob bytes of the loaded view snapshot.
    pub snapshot_mesh_bytes: Counter,
}

/// The counters of this process.
pub static STATS: Stats = Stats {
    frames: Counter::new(),
    viewport_draws: Counter::new(),
    submits: Counter::new(),
    uploads: Counter::new(),
    upload_bytes: Counter::new(),
    upload_ui_ns: Counter::new(),
    pipeline_builds: Counter::new(),
    target_builds: Counter::new(),
    resize_ns: Counter::new(),
    cli_runs: Counter::new(),
    cli_ns: Counter::new(),
    cli_killed: Counter::new(),
    cli_active: Counter::new(),
    reloads_applied: Counter::new(),
    select_errors: Counter::new(),
    retained_display_bytes: Counter::new(),
    snapshot_mesh_bytes: Counter::new(),
};

/// The counters' values at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sample {
    /// See [`Stats::frames`].
    pub frames: u64,
    /// See [`Stats::viewport_draws`].
    pub viewport_draws: u64,
    /// See [`Stats::submits`].
    pub submits: u64,
    /// See [`Stats::uploads`].
    pub uploads: u64,
    /// See [`Stats::upload_bytes`].
    pub upload_bytes: u64,
    /// See [`Stats::upload_ui_ns`].
    pub upload_ui_ns: u64,
    /// See [`Stats::pipeline_builds`].
    pub pipeline_builds: u64,
    /// See [`Stats::target_builds`].
    pub target_builds: u64,
    /// See [`Stats::resize_ns`].
    pub resize_ns: u64,
    /// See [`Stats::cli_runs`].
    pub cli_runs: u64,
    /// See [`Stats::cli_ns`].
    pub cli_ns: u64,
    /// See [`Stats::cli_killed`].
    pub cli_killed: u64,
    /// See [`Stats::cli_active`] (a gauge).
    pub cli_active: u64,
    /// See [`Stats::reloads_applied`].
    pub reloads_applied: u64,
    /// See [`Stats::select_errors`].
    pub select_errors: u64,
    /// See [`Stats::retained_display_bytes`] (a gauge).
    pub retained_display_bytes: u64,
    /// See [`Stats::snapshot_mesh_bytes`] (a gauge).
    pub snapshot_mesh_bytes: u64,
}

impl Stats {
    /// Reads every counter.
    pub fn sample(&self) -> Sample {
        Sample {
            frames: self.frames.get(),
            viewport_draws: self.viewport_draws.get(),
            submits: self.submits.get(),
            uploads: self.uploads.get(),
            upload_bytes: self.upload_bytes.get(),
            upload_ui_ns: self.upload_ui_ns.get(),
            pipeline_builds: self.pipeline_builds.get(),
            target_builds: self.target_builds.get(),
            resize_ns: self.resize_ns.get(),
            cli_runs: self.cli_runs.get(),
            cli_ns: self.cli_ns.get(),
            cli_killed: self.cli_killed.get(),
            cli_active: self.cli_active.get(),
            reloads_applied: self.reloads_applied.get(),
            select_errors: self.select_errors.get(),
            retained_display_bytes: self.retained_display_bytes.get(),
            snapshot_mesh_bytes: self.snapshot_mesh_bytes.get(),
        }
    }
}

impl Sample {
    /// What the counters did since `earlier`; gauges keep their current value.
    #[must_use]
    pub fn since(&self, earlier: &Self) -> Self {
        Self {
            frames: self.frames.saturating_sub(earlier.frames),
            viewport_draws: self.viewport_draws.saturating_sub(earlier.viewport_draws),
            submits: self.submits.saturating_sub(earlier.submits),
            uploads: self.uploads.saturating_sub(earlier.uploads),
            upload_bytes: self.upload_bytes.saturating_sub(earlier.upload_bytes),
            upload_ui_ns: self.upload_ui_ns.saturating_sub(earlier.upload_ui_ns),
            pipeline_builds: self.pipeline_builds.saturating_sub(earlier.pipeline_builds),
            target_builds: self.target_builds.saturating_sub(earlier.target_builds),
            resize_ns: self.resize_ns.saturating_sub(earlier.resize_ns),
            cli_runs: self.cli_runs.saturating_sub(earlier.cli_runs),
            cli_ns: self.cli_ns.saturating_sub(earlier.cli_ns),
            cli_killed: self.cli_killed.saturating_sub(earlier.cli_killed),
            cli_active: self.cli_active,
            reloads_applied: self.reloads_applied.saturating_sub(earlier.reloads_applied),
            select_errors: self.select_errors.saturating_sub(earlier.select_errors),
            retained_display_bytes: self.retained_display_bytes,
            snapshot_mesh_bytes: self.snapshot_mesh_bytes,
        }
    }
}

/// CPU time and memory of this process as Linux reports them in `/proc/self`; `None` elsewhere
/// or when the files cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    /// User CPU time of this process, in clock ticks (`USER_HZ`).
    pub utime: u64,
    /// System CPU time of this process, in clock ticks.
    pub stime: u64,
    /// User CPU time of its waited-for children (the CLI processes), in clock ticks.
    pub cutime: u64,
    /// System CPU time of its waited-for children, in clock ticks.
    pub cstime: u64,
    /// Resident set size, KiB.
    pub rss_kib: u64,
    /// Peak resident set size, KiB.
    pub hwm_kib: u64,
}

fn read_bounded(path: &str) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(64 * 1024).read_to_string(&mut text).ok()?;
    Some(text)
}

/// Fields 14 to 17 of `/proc/self/stat` (utime, stime, cutime, cstime). The second field is the
/// command name in parentheses and may itself contain spaces or parentheses, so the fields are
/// counted after the last `)`.
fn parse_stat(text: &str) -> Option<[u64; 4]> {
    let (_, after) = text.rsplit_once(')')?;
    // After the name: field 3 (state) is the first token, so utime (field 14) is the twelfth.
    let mut fields = after.split_whitespace().skip(11);
    let mut out = [0u64; 4];
    for value in &mut out {
        *value = fields.next()?.parse().ok()?;
    }
    Some(out)
}

fn status_kib(text: &str, key: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with(key))?;
    line.get(key.len()..)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Samples [`Process`] (Linux only).
pub fn process() -> Option<Process> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let [utime, stime, cutime, cstime] = parse_stat(&read_bounded("/proc/self/stat")?)?;
    let status = read_bounded("/proc/self/status")?;
    Some(Process {
        utime,
        stime,
        cutime,
        cstime,
        rss_kib: status_kib(&status, "VmRSS:")?,
        hwm_kib: status_kib(&status, "VmHWM:")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields_are_counted_after_the_last_parenthesis() {
        let line = "4242 (beam craft) (x)) S 1 2 3 4 5 6 7 8 9 10 111 222 333 444 20 0 1 0";
        assert_eq!(parse_stat(line), Some([111, 222, 333, 444]));
        assert_eq!(parse_stat("4242 (x) S 1 2"), None);
        assert_eq!(parse_stat("no parenthesis"), None);
    }

    #[test]
    fn status_values_are_read_in_kib() {
        let text = "Name:\tbeamcraft-ui\nVmHWM:\t  123456 kB\nVmRSS:\t   65432 kB\n";
        assert_eq!(status_kib(text, "VmRSS:"), Some(65432));
        assert_eq!(status_kib(text, "VmHWM:"), Some(123456));
        assert_eq!(status_kib(text, "VmSwap:"), None);
    }

    #[test]
    fn deltas_subtract_counters_and_keep_gauges() {
        let earlier = Sample {
            frames: 10,
            retained_display_bytes: 7,
            ..Sample::default()
        };
        let later = Sample {
            frames: 25,
            retained_display_bytes: 3,
            ..Sample::default()
        };
        let delta = later.since(&earlier);
        assert_eq!(delta.frames, 15);
        assert_eq!(delta.retained_display_bytes, 3);
        assert_eq!(earlier.since(&later).frames, 0);
    }
}
