// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `beamcraft-ui --store ROOT --perf-run PLAN`: a scripted measurement inside the real window.
//!
//! The plan is a comma-separated list of phases, run in order:
//!
//! - `load`: from window creation until the first refresh has been applied;
//! - `idle:S`: S seconds without input;
//! - `ui:K`: K frames that change no camera, scene or size (what typing or hovering does);
//! - `orbit:K`: K frames that each orbit the camera a little;
//! - `reload:K`: K refreshes of an unchanged store, one after the other (the R key);
//! - `select:K`: K click selections, each along a ray straight down through the centre of a box
//!   element of the loaded snapshot (the path a click takes), until its result is shown;
//! - `cancel:K`: K times: select, press Esc at once (the lookup is no longer wanted) and select
//!   again; the phase also waits for every CLI process to end, so work that was not wanted but
//!   still ran is counted;
//! - `edit:K`: K plates created through the Create panel's command path, each followed by the
//!   refresh it triggers. This publishes K plates to the store;
//! - `resize:K`: K window resizes, alternating between two sizes.
//!
//! Each phase reports what the process counters ([`crate::stats`]) did, its wall time, the
//! CPU time eframe measured for its frames and, on Linux, the CPU time of the process and of its
//! CLI children and the resident set size. The driver only asks the window to act; it never
//! takes a shortcut the window would not take. It is a pure state machine so it can be tested
//! without a window.

use crate::stats::{Process, Sample};
use crate::{Error, json_string};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

/// Most phases in one plan.
pub const MAX_PHASES: usize = 32;
/// Largest count of one phase.
pub const MAX_COUNT: u32 = 10_000;
/// Longest idle phase.
pub const MAX_IDLE: Duration = Duration::from_secs(600);
/// Longest one step (a refresh, an edit) may take before the run fails.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(300);
/// Longest wait for a requested window size before the step counts as timed out.
pub const RESIZE_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest the whole run may take.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(3600);
/// How early the wake-up that ends an idle phase may arrive and still end it. egui schedules a
/// delayed repaint up to one predicted frame early; without the tolerance the driver would
/// re-arm the wake-up and add frames of its own to a phase that measures the absence of frames.
pub const IDLE_TOLERANCE: Duration = Duration::from_millis(100);
/// The two window sizes (logical points) the resize phase alternates between; the first differs
/// from the window's initial size.
pub const RESIZE_SIZES: [[f32; 2]; 2] = [[1200.0, 760.0], [1400.0, 900.0]];

/// One phase of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Until the first refresh is applied.
    Load,
    /// No input for this long.
    Idle(Duration),
    /// Frames without any viewport change.
    Ui(u32),
    /// Frames that orbit the camera.
    Orbit(u32),
    /// Frames drawn in one render mode (V16), each with a light orbit: the frame-time baseline
    /// of that mode over the loaded scene.
    Mode(beamcraft_engine::RenderMode, u32),
    /// Refreshes of an unchanged store.
    Reload(u32),
    /// Click selections.
    Select(u32),
    /// Selections abandoned at once and asked again.
    Cancel(u32),
    /// Created plates, each with its refresh.
    Edit(u32),
    /// Window resizes.
    Resize(u32),
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Idle(_) => "idle",
            Self::Ui(_) => "ui",
            Self::Orbit(_) => "orbit",
            Self::Mode(mode, _) => mode.as_token(),
            Self::Reload(_) => "reload",
            Self::Select(_) => "select",
            Self::Cancel(_) => "cancel",
            Self::Edit(_) => "edit",
            Self::Resize(_) => "resize",
        }
    }

    fn parameter(self) -> u64 {
        match self {
            Self::Load => 0,
            Self::Idle(d) => d.as_secs(),
            Self::Ui(k)
            | Self::Orbit(k)
            | Self::Mode(_, k)
            | Self::Reload(k)
            | Self::Select(k)
            | Self::Cancel(k)
            | Self::Edit(k)
            | Self::Resize(k) => u64::from(k),
        }
    }
}

fn usage() -> Error {
    Error::Input(
        "perf plan: load,idle:S,ui:K,orbit:K,shaded:K,wireframe:K,transparent:K,grey:K,selected:K,reload:K,select:K,cancel:K,edit:K,resize:K (comma-separated, 1 to 32)",
    )
}

/// Parses a plan such as `load,idle:20,ui:120,orbit:120,reload:5,edit:3,resize:20`.
pub fn parse(plan: &str) -> Result<Vec<Phase>, Error> {
    if plan.is_empty() || plan.len() > 1024 {
        return Err(usage());
    }
    let mut phases = Vec::new();
    for item in plan.split(',') {
        if phases.len() == MAX_PHASES {
            return Err(usage());
        }
        if item == "load" {
            phases.push(Phase::Load);
            continue;
        }
        let (name, count) = item.split_once(':').ok_or_else(usage)?;
        let count: u32 = count.parse().map_err(|_| usage())?;
        if count == 0 || count > MAX_COUNT {
            return Err(usage());
        }
        phases.push(match name {
            "idle" => {
                let seconds = Duration::from_secs(u64::from(count));
                if seconds > MAX_IDLE {
                    return Err(usage());
                }
                Phase::Idle(seconds)
            }
            "ui" => Phase::Ui(count),
            "orbit" => Phase::Orbit(count),
            "reload" => Phase::Reload(count),
            "select" => Phase::Select(count),
            "cancel" => Phase::Cancel(count),
            "edit" => Phase::Edit(count),
            "resize" => Phase::Resize(count),
            // Any render-mode name is K frames drawn in that mode.
            other => {
                let mode = beamcraft_engine::RenderMode::parse(other).ok_or_else(usage)?;
                Phase::Mode(mode, count)
            }
        });
    }
    Ok(phases)
}

/// Worker-side timing of one refresh: the CLI processes it ran and its wall time.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RefreshTiming {
    /// CLI processes started for this refresh.
    pub processes: u32,
    /// Wall time of the whole refresh on the worker (processes, snapshot rebuild, scene build).
    pub total_ms: f64,
}

/// What the window reports to the driver at the start of each frame.
#[derive(Debug, Clone)]
pub struct Observed<'a> {
    /// Now.
    pub now: Instant,
    /// Refresh results the window has consumed, whatever their outcome.
    pub reloads_finished: u64,
    /// Whether a refresh is running or queued.
    pub reload_busy: bool,
    /// Commit results the window has consumed.
    pub commits_finished: u64,
    /// Whether a commit is running.
    pub commit_busy: bool,
    /// Whether the last consumed commit succeeded.
    pub last_commit_ok: bool,
    /// Selection results the window has consumed (a stale one too).
    pub selects_finished: u64,
    /// Whether a selection lookup is running.
    pub select_busy: bool,
    /// The offscreen target size in pixels.
    pub target: [u32; 2],
    /// CPU seconds eframe measured for the previous frame.
    pub frame_cpu: Option<f32>,
    /// The window's counters, sampled this frame.
    pub stats: Sample,
    /// Worker timings of every refresh consumed so far, oldest first.
    pub refresh_timings: &'a [RefreshTiming],
}

/// What the driver asks the window to do this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    /// Start a refresh, as the R key does.
    pub reload: bool,
    /// Create a plate through the Create panel's command path; the value is its ordinal.
    pub commit: Option<u32>,
    /// Select box element number `n` of the loaded snapshot, as a click on it would.
    pub select: Option<u32>,
    /// Abandon the selection lookup in flight, as Esc does, and select box element `n + 1`.
    pub cancel: Option<u32>,
    /// Orbit the camera a little.
    pub orbit: bool,
    /// Draw this frame in this render mode, as a key press would (a mode phase).
    pub mode: Option<beamcraft_engine::RenderMode>,
    /// Ask for this window inner size (logical points).
    pub resize: Option<[f32; 2]>,
    /// Ask for another frame right away.
    pub repaint_now: bool,
    /// Ask for a frame after at most this long (re-issued every frame: egui forgets it).
    pub wake_after: Option<Duration>,
    /// The run is over: the report (JSON) and whether every phase succeeded.
    pub finished: Option<(String, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Reload {
        finished: u64,
    },
    Select {
        finished: u64,
    },
    /// A cancel step's first click; Esc follows while its lookup runs a CLI process.
    CancelClick {
        finished: u64,
    },
    Cancel {
        finished: u64,
    },
    Edit {
        commits: u64,
        reloads: u64,
    },
    Resize {
        from: [u32; 2],
    },
}

#[derive(Debug, Clone)]
struct Running {
    started: Instant,
    stats: Sample,
    process: Option<Process>,
    timings_seen: usize,
    frame_cpu_s: f64,
    frames: u32,
    done_steps: u32,
    pending: Option<(Pending, Instant)>,
    steps_ms: Vec<f64>,
    timeouts: u32,
    /// The phase's last result was applied in the previous frame; it ends with this one.
    settling: bool,
}

impl Running {
    fn new(now: Instant, timings_seen: usize, stats: Sample) -> Self {
        Self {
            started: now,
            stats,
            process: crate::stats::process(),
            timings_seen,
            frame_cpu_s: 0.0,
            frames: 0,
            done_steps: 0,
            pending: None,
            steps_ms: Vec::new(),
            timeouts: 0,
            settling: false,
        }
    }
}

/// The run of one plan.
#[derive(Debug, Clone)]
pub struct PerfRun {
    phases: Vec<Phase>,
    index: usize,
    running: Option<Running>,
    created: Instant,
    reports: Vec<String>,
    header: String,
    done: bool,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn ns_ms(ns: u64) -> f64 {
    ns as f64 / 1e6
}

fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    let upper = sorted.get(n / 2).copied()?;
    if n % 2 == 1 {
        return Some(upper);
    }
    Some((sorted.get(n / 2 - 1).copied()? + upper) / 2.0)
}

fn opt_f(value: Option<f64>) -> String {
    value.map_or_else(|| "null".into(), |v| format!("{v:.3}"))
}

fn opt_u(value: Option<u64>) -> String {
    value.map_or_else(|| "null".into(), |v| v.to_string())
}

fn list(values: &[f64]) -> String {
    let mut out = String::from("[");
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{v:.3}");
    }
    out.push(']');
    out
}

impl PerfRun {
    /// A run of `phases`, measured from now (window creation). `header` is the JSON members
    /// (adapter, samples, ...) written before the phases, without braces.
    pub fn new(phases: Vec<Phase>, header: String) -> Self {
        let now = Instant::now();
        // A leading load phase starts with the process: its counters count from zero.
        let running = (phases.first() == Some(&Phase::Load)).then(|| {
            let mut running = Running::new(now, 0, Sample::default());
            running.process = None;
            running
        });
        Self {
            phases,
            index: 0,
            running,
            created: now,
            reports: Vec::new(),
            header,
            done: false,
        }
    }

    /// Whether the run has ended.
    pub const fn done(&self) -> bool {
        self.done
    }

    fn report(
        &self,
        phase: Phase,
        running: &Running,
        now: Instant,
        timings: &[RefreshTiming],
        stats: &Sample,
    ) -> String {
        let delta = stats.since(&running.stats);
        let process = crate::stats::process();
        let cpu = |f: fn(&Process) -> u64| {
            process.map(|p| f(&p).saturating_sub(running.process.as_ref().map_or(0, f)))
        };
        let worker: Vec<f64> = timings.iter().map(|t| t.total_ms).collect();
        let processes: u64 = timings.iter().map(|t| u64::from(t.processes)).sum();
        format!(
            concat!(
                "{{\"phase\":\"{}\",\"parameter\":{},\"wall_ms\":{:.3},\"frames\":{},",
                "\"viewport_draws\":{},\"submits\":{},\"uploads\":{},\"upload_bytes\":{},",
                "\"upload_ui_ms\":{:.3},\"pipeline_builds\":{},\"target_builds\":{},",
                "\"resize_ms\":{:.3},\"cli_runs\":{},\"cli_ms\":{:.3},\"cli_killed\":{},",
                "\"reloads_applied\":{},\"select_errors\":{},",
                "\"frame_cpu_ms\":{:.3},\"utime_ticks\":{},\"stime_ticks\":{},",
                "\"child_utime_ticks\":{},\"child_stime_ticks\":{},\"rss_kib\":{},\"hwm_kib\":{},",
                "\"retained_display_bytes\":{},\"snapshot_mesh_bytes\":{},\"steps\":{},",
                "\"steps_ms\":{},\"step_median_ms\":{},\"timeouts\":{},\"refreshes\":{},",
                "\"refresh_processes\":{},\"refresh_worker_ms\":{},\"refresh_worker_median_ms\":{}}}"
            ),
            phase.name(),
            phase.parameter(),
            ms(now.saturating_duration_since(running.started)),
            delta.frames,
            delta.viewport_draws,
            delta.submits,
            delta.uploads,
            delta.upload_bytes,
            ns_ms(delta.upload_ui_ns),
            delta.pipeline_builds,
            delta.target_builds,
            ns_ms(delta.resize_ns),
            delta.cli_runs,
            ns_ms(delta.cli_ns),
            delta.cli_killed,
            delta.reloads_applied,
            delta.select_errors,
            running.frame_cpu_s * 1000.0,
            opt_u(cpu(|p| p.utime)),
            opt_u(cpu(|p| p.stime)),
            opt_u(cpu(|p| p.cutime)),
            opt_u(cpu(|p| p.cstime)),
            opt_u(process.map(|p| p.rss_kib)),
            opt_u(process.map(|p| p.hwm_kib)),
            delta.retained_display_bytes,
            delta.snapshot_mesh_bytes,
            running.done_steps,
            list(&running.steps_ms),
            opt_f(median(&running.steps_ms)),
            running.timeouts,
            timings.len(),
            processes,
            list(&worker),
            opt_f(median(&worker)),
        )
    }

    fn finish(&mut self, ok: bool, message: Option<&str>) -> Request {
        self.done = true;
        let mut json = format!("{{\"outcome\":\"ui_perf_run\",{},", self.header);
        if let Some(message) = message {
            let _ = write!(json, "\"error\":{},", json_string(message));
        }
        json.push_str("\"phases\":[");
        json.push_str(&self.reports.join(","));
        json.push_str("]}");
        Request {
            finished: Some((json, ok)),
            ..Request::default()
        }
    }

    /// Advances the run by one frame.
    pub fn step(&mut self, observed: &Observed<'_>) -> Request {
        if self.done {
            return Request::default();
        }
        let now = observed.now;
        if now.saturating_duration_since(self.created) > RUN_TIMEOUT {
            return self.finish(false, Some("the perf run exceeded its time limit"));
        }
        let Some(&phase) = self.phases.get(self.index) else {
            return self.finish(true, None);
        };
        let mut running = match self.running.take() {
            Some(running) => running,
            None => Running::new(now, observed.refresh_timings.len(), observed.stats),
        };
        // eframe reports the previous frame, which already belonged to this phase.
        running.frame_cpu_s += f64::from(observed.frame_cpu.unwrap_or(0.0));
        running.frames = running.frames.saturating_add(1);
        let mut request = Request::default();
        let reached = running.settling
            || match phase {
                Phase::Load => observed.reloads_finished >= 1 && !observed.reload_busy,
                Phase::Idle(length) => {
                    let elapsed = now.saturating_duration_since(running.started);
                    request.wake_after = Some(length.saturating_sub(elapsed));
                    elapsed + IDLE_TOLERANCE >= length
                }
                Phase::Ui(count) | Phase::Orbit(count) => {
                    // The frame that starts the phase is the first of its count.
                    if running.frames > count {
                        true
                    } else {
                        request.orbit = matches!(phase, Phase::Orbit(_));
                        request.repaint_now = true;
                        false
                    }
                }
                Phase::Mode(mode, count) => {
                    // The frame that starts the phase is the first of its count.
                    if running.frames > count {
                        true
                    } else {
                        request.orbit = true;
                        request.mode = Some(mode);
                        request.repaint_now = true;
                        false
                    }
                }
                Phase::Reload(count)
                | Phase::Select(count)
                | Phase::Cancel(count)
                | Phase::Edit(count)
                | Phase::Resize(count) => {
                    match self.advance(phase, &mut running, observed) {
                        Ok(Some(next)) => request = next,
                        Ok(None) => {}
                        Err(message) => {
                            self.reports.push(self.report(
                                phase,
                                &running,
                                now,
                                &[],
                                &observed.stats,
                            ));
                            return self.finish(false, Some(message));
                        }
                    }
                    running.done_steps >= count
                }
            };
        // A result is applied in the frame the driver sees it, and that frame draws after the
        // driver: such a phase ends one frame later, so its last draw is its own.
        let complete = if reached
            && !running.settling
            && matches!(
                phase,
                Phase::Load
                    | Phase::Reload(_)
                    | Phase::Select(_)
                    | Phase::Cancel(_)
                    | Phase::Edit(_)
                    | Phase::Resize(_)
            ) {
            running.settling = true;
            request.repaint_now = true;
            false
        } else {
            reached
        };
        if !complete {
            if request.wake_after.is_none() && !request.repaint_now {
                // A wake-up for the timeouts; the step itself is announced by its own event.
                request.wake_after = Some(Duration::from_millis(250));
            }
            self.running = Some(running);
            return request;
        }
        let timings = observed
            .refresh_timings
            .get(running.timings_seen..)
            .unwrap_or_default();
        self.reports
            .push(self.report(phase, &running, now, timings, &observed.stats));
        self.index += 1;
        // The next phase starts now, so no frame falls between two phases. It acts from the next
        // frame on; an idle phase asks only for the frame that ends it.
        if self.index < self.phases.len() {
            self.running = Some(Running::new(
                now,
                observed.refresh_timings.len(),
                observed.stats,
            ));
        }
        let mut request = match self.phases.get(self.index) {
            Some(Phase::Idle(length)) => Request {
                wake_after: Some(*length),
                ..Request::default()
            },
            _ => Request {
                repaint_now: true,
                ..Request::default()
            },
        };
        // A mode phase must not leak its drawing style into the next phase's frames: the
        // boundary frame draws shaded again, unless the next phase names its own mode.
        if matches!(phase, Phase::Mode(..)) {
            request.mode = match self.phases.get(self.index) {
                Some(Phase::Mode(mode, _)) => Some(*mode),
                _ => Some(beamcraft_engine::RenderMode::Shaded),
            };
        }
        request
    }

    /// One step of a reload, edit or resize phase: starts the next action or checks the
    /// pending one. `Ok(Some(request))` asks the window to act.
    fn advance(
        &mut self,
        phase: Phase,
        running: &mut Running,
        observed: &Observed<'_>,
    ) -> Result<Option<Request>, &'static str> {
        let now = observed.now;
        if let Some((pending, since)) = running.pending {
            let waited = now.saturating_duration_since(since);
            if let Pending::CancelClick { finished } = pending {
                // Esc once the lookup has started a CLI process (or has already answered), the
                // way a user gives up a lookup that takes too long; then the next click.
                if observed.stats.cli_active > 0 || observed.selects_finished > finished {
                    running.pending = Some((
                        Pending::Cancel {
                            finished: observed.selects_finished,
                        },
                        since,
                    ));
                    return Ok(Some(Request {
                        cancel: Some(running.done_steps),
                        ..Request::default()
                    }));
                }
                if waited >= STEP_TIMEOUT {
                    return Err("a perf step did not finish in time");
                }
                return Ok(None);
            }
            let finished = match pending {
                Pending::Reload { finished } => {
                    observed.reloads_finished > finished && !observed.reload_busy
                }
                Pending::Select { finished } => {
                    observed.selects_finished > finished && !observed.select_busy
                }
                // Abandoned work counts too: the step ends only when no CLI process runs.
                Pending::Cancel { finished } => {
                    observed.selects_finished > finished
                        && !observed.select_busy
                        && observed.stats.cli_active == 0
                }
                Pending::CancelClick { .. } => false,
                Pending::Edit { commits, reloads } => {
                    if observed.commits_finished > commits && !observed.last_commit_ok {
                        return Err("a perf edit was refused");
                    }
                    observed.commits_finished > commits
                        && observed.reloads_finished > reloads
                        && !observed.reload_busy
                        && !observed.commit_busy
                }
                Pending::Resize { from } => observed.target != from,
            };
            if finished {
                running.steps_ms.push(ms(waited));
                running.done_steps += 1;
                running.pending = None;
                return Ok(Some(Request {
                    repaint_now: true,
                    ..Request::default()
                }));
            }
            if matches!(pending, Pending::Resize { .. }) && waited >= RESIZE_TIMEOUT {
                running.timeouts += 1;
                running.done_steps += 1;
                running.pending = None;
                return Ok(None);
            }
            if waited >= STEP_TIMEOUT {
                return Err("a perf step did not finish in time");
            }
            return Ok(None);
        }
        if observed.reload_busy || observed.commit_busy || observed.select_busy {
            // Never start a step on top of unrelated work.
            return Ok(None);
        }
        let ordinal = running.done_steps;
        let (pending, request) = match phase {
            Phase::Reload(_) => (
                Pending::Reload {
                    finished: observed.reloads_finished,
                },
                Request {
                    reload: true,
                    ..Request::default()
                },
            ),
            Phase::Select(_) => (
                Pending::Select {
                    finished: observed.selects_finished,
                },
                Request {
                    select: Some(ordinal),
                    ..Request::default()
                },
            ),
            Phase::Cancel(_) => (
                Pending::CancelClick {
                    finished: observed.selects_finished,
                },
                Request {
                    select: Some(ordinal),
                    ..Request::default()
                },
            ),
            Phase::Edit(_) => (
                Pending::Edit {
                    commits: observed.commits_finished,
                    reloads: observed.reloads_finished,
                },
                Request {
                    commit: Some(ordinal),
                    ..Request::default()
                },
            ),
            _ => {
                let index = usize::try_from(ordinal % 2).unwrap_or(0);
                (
                    Pending::Resize {
                        from: observed.target,
                    },
                    Request {
                        resize: RESIZE_SIZES.get(index).copied(),
                        ..Request::default()
                    },
                )
            }
        };
        running.pending = Some((pending, now));
        Ok(Some(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(now: Instant) -> Observed<'static> {
        Observed {
            now,
            reloads_finished: 1,
            reload_busy: false,
            commits_finished: 0,
            commit_busy: false,
            last_commit_ok: true,
            selects_finished: 0,
            select_busy: false,
            target: [800, 600],
            frame_cpu: Some(0.001),
            stats: Sample::default(),
            refresh_timings: &[],
        }
    }

    #[test]
    fn plans_parse_with_bounds() {
        assert_eq!(
            parse("load,idle:20,ui:3,orbit:4,reload:5,select:6,cancel:7,edit:1,resize:2").unwrap(),
            vec![
                Phase::Load,
                Phase::Idle(Duration::from_secs(20)),
                Phase::Ui(3),
                Phase::Orbit(4),
                Phase::Reload(5),
                Phase::Select(6),
                Phase::Cancel(7),
                Phase::Edit(1),
                Phase::Resize(2),
            ]
        );
        assert_eq!(
            parse("shaded:2,wireframe:3,transparent:1,grey:2,selected:4").unwrap(),
            vec![
                Phase::Mode(beamcraft_engine::RenderMode::Shaded, 2),
                Phase::Mode(beamcraft_engine::RenderMode::Wireframe, 3),
                Phase::Mode(beamcraft_engine::RenderMode::Transparent, 1),
                Phase::Mode(beamcraft_engine::RenderMode::Grey, 2),
                Phase::Mode(beamcraft_engine::RenderMode::SelectedOnly, 4),
            ]
        );
        for bad in [
            "",
            "idle",
            "idle:0",
            "idle:601",
            "ui:10001",
            "ui:-1",
            "nap:3",
            "load,",
            &"ui:1,".repeat(33),
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_mode_phase_orbits_and_draws_in_its_mode() {
        let t0 = Instant::now();
        let mut run = PerfRun::new(parse("wireframe:2,selected:1").unwrap(), "\"x\":1".into());
        let mut modes = Vec::new();
        loop {
            let request = run.step(&observed(t0));
            if let Some(mode) = request.mode {
                modes.push(mode);
            }
            if let Some((report, ok)) = request.finished {
                assert!(ok);
                assert!(report.contains("\"phase\":\"wireframe\""), "{report}");
                assert!(report.contains("\"phase\":\"selected\""), "{report}");
                break;
            }
        }
        // The frame that starts a phase is its first, so two mode frames per count of two; the
        // boundary frames re-state the mode of the next phase, and the last one resets shaded.
        assert_eq!(
            modes,
            vec![
                beamcraft_engine::RenderMode::Wireframe,
                beamcraft_engine::RenderMode::Wireframe,
                beamcraft_engine::RenderMode::SelectedOnly,
                beamcraft_engine::RenderMode::SelectedOnly,
                beamcraft_engine::RenderMode::Shaded,
            ]
        );
        // A mode phase resets the drawing style on its boundary, so a following phase without
        // a mode of its own is not measured in a leaked mode.
        let mut run = PerfRun::new(parse("wireframe:1,ui:1").unwrap(), "\"x\":1".into());
        let mut modes = Vec::new();
        loop {
            let request = run.step(&observed(t0));
            if let Some(mode) = request.mode {
                modes.push(mode);
            }
            if request.finished.is_some() {
                break;
            }
        }
        assert_eq!(
            modes,
            vec![
                beamcraft_engine::RenderMode::Wireframe,
                beamcraft_engine::RenderMode::Shaded,
            ]
        );
    }

    #[test]
    fn idle_waits_its_length_and_asks_to_be_woken() {
        let t0 = Instant::now();
        let mut run = PerfRun::new(parse("idle:2").unwrap(), "\"x\":1".into());
        let first = run.step(&observed(t0));
        assert_eq!(first.wake_after, Some(Duration::from_secs(2)));
        let later = run.step(&observed(t0 + Duration::from_millis(1500)));
        assert!(later.wake_after.unwrap() <= Duration::from_millis(500));
        // A wake-up a predicted frame early still ends the phase.
        let done = run.step(&observed(t0 + Duration::from_millis(1984)));
        assert!(done.finished.is_none() && done.repaint_now);
        let end = run.step(&observed(t0 + Duration::from_secs(2)));
        let (json, ok) = end.finished.unwrap();
        assert!(ok && run.done());
        assert!(
            json.starts_with(
                "{\"outcome\":\"ui_perf_run\",\"x\":1,\"phases\":[{\"phase\":\"idle\""
            )
        );
    }

    #[test]
    fn a_phase_before_an_idle_one_asks_only_for_the_wake_up_that_ends_it() {
        let t0 = Instant::now();
        let mut run = PerfRun::new(parse("ui:1,idle:5").unwrap(), "\"x\":1".into());
        assert!(run.step(&observed(t0)).repaint_now);
        let into_idle = run.step(&observed(t0));
        assert!(!into_idle.repaint_now);
        assert_eq!(into_idle.wake_after, Some(Duration::from_secs(5)));
    }

    #[test]
    fn ui_and_orbit_count_frames_and_only_orbit_moves_the_camera() {
        let t0 = Instant::now();
        let mut run = PerfRun::new(parse("ui:2,orbit:2").unwrap(), "\"x\":1".into());
        let a = run.step(&observed(t0));
        let b = run.step(&observed(t0));
        assert!(a.repaint_now && !a.orbit && b.repaint_now && !b.orbit);
        assert!(run.step(&observed(t0)).finished.is_none());
        let c = run.step(&observed(t0));
        assert!(c.orbit && c.repaint_now);
    }

    #[test]
    fn a_reload_step_waits_for_its_result() {
        let t0 = Instant::now();
        let mut run = PerfRun::new(parse("reload:1").unwrap(), "\"x\":1".into());
        let start = run.step(&observed(t0));
        assert!(start.reload);
        let mut busy = observed(t0);
        busy.reload_busy = true;
        assert!(!run.step(&busy).reload);
        // The window consumes the result (and records its timing) before the driver steps.
        let timings = [RefreshTiming {
            processes: 2,
            total_ms: 35.0,
        }];
        let mut back = observed(t0 + Duration::from_millis(40));
        back.reloads_finished = 2;
        back.refresh_timings = &timings;
        let applied = run.step(&back);
        assert!(applied.repaint_now && !applied.reload && applied.finished.is_none());
        // The frame after the result ends the phase (its draw is the phase's own), then the run.
        assert!(run.step(&back).finished.is_none());
        let (json, ok) = run.step(&back).finished.unwrap();
        assert!(ok);
        assert!(json.contains("\"steps\":1,\"steps_ms\":[40.000]"), "{json}");
        assert!(json.contains("\"refresh_processes\":2"), "{json}");
    }

    #[test]
    fn a_cancel_step_ends_only_when_no_cli_process_runs() {
        let t0 = Instant::now();
        let mut run = PerfRun::new(parse("select:1,cancel:1").unwrap(), "\"x\":1".into());
        assert_eq!(run.step(&observed(t0)).select, Some(0));
        let mut selected = observed(t0);
        selected.selects_finished = 1;
        assert!(run.step(&selected).repaint_now);
        assert!(
            run.step(&selected).repaint_now,
            "the select phase settles and ends"
        );
        // The step clicks first, and presses Esc only once the lookup runs a CLI process.
        assert_eq!(run.step(&selected).select, Some(0));
        let waiting = run.step(&selected);
        assert!(waiting.cancel.is_none() && waiting.wake_after.is_some());
        let mut looking = observed(t0);
        looking.stats.cli_active = 1;
        assert_eq!(run.step(&looking).cancel, Some(0));
        let mut answered = observed(t0);
        answered.selects_finished = 2;
        answered.stats.cli_active = 1;
        // An abandoned lookup's process is still running: the step waits for it.
        let waiting = run.step(&answered);
        assert!(!waiting.repaint_now && waiting.wake_after.is_some());
        let mut over = observed(t0);
        over.selects_finished = 2;
        assert!(run.step(&over).repaint_now, "now it is over");
    }

    #[test]
    fn a_refused_edit_ends_the_run_with_an_error() {
        let t0 = Instant::now();
        let mut run = PerfRun::new(parse("edit:2").unwrap(), "\"x\":1".into());
        assert_eq!(run.step(&observed(t0)).commit, Some(0));
        let mut refused = observed(t0);
        refused.commits_finished = 1;
        refused.last_commit_ok = false;
        let (json, ok) = run.step(&refused).finished.unwrap();
        assert!(!ok);
        assert!(json.contains("\"error\":\"a perf edit was refused\""));
    }

    #[test]
    fn a_resize_that_never_arrives_times_out_and_the_run_continues() {
        let t0 = Instant::now();
        let mut run = PerfRun::new(parse("resize:2").unwrap(), "\"x\":1".into());
        assert_eq!(run.step(&observed(t0)).resize, Some(RESIZE_SIZES[0]));
        let late = observed(t0 + RESIZE_TIMEOUT);
        assert!(run.step(&late).resize.is_none());
        assert_eq!(run.step(&late).resize, Some(RESIZE_SIZES[1]));
        let mut resized = observed(t0 + RESIZE_TIMEOUT + Duration::from_millis(30));
        resized.target = [700, 500];
        assert!(run.step(&resized).repaint_now);
    }
}
