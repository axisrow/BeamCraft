// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `beamcraft-ui [--cli PATH] [--config-dir DIR]` opens the window on the start page: the start
//! window (environment and author), then the recent projects, the projects of a folder and
//! Create; the chosen model opens in the same window (see `beamcraft_ui::start`).
//! `beamcraft-ui --store ROOT [--cli PATH]` opens the window on the published revision;
//! `--actor-id UUID` records that author on its edits.
//! `beamcraft-ui --workspace DIR [--cli PATH]` opens it on a personal workspace (a store made by
//! `beamcraft create-workspace`), which also enables undo and redo. Adding `--published ROOT`
//! (the store the workspace was forked from) enables Publish (`beamcraft publish-workspace`).
//! `beamcraft-ui --store ROOT --metrics FRAMES [--width W] [--height H] [--cli PATH]` renders the
//! same snapshot offscreen, headless, and prints baseline GPU metrics as one JSON object.
//! `beamcraft-ui --synthetic COUNT --metrics FRAMES [--width W] [--height H]` measures the engine
//! snapshot of COUNT synthetic kernel plates instead of a store (labelled `synthetic` in the
//! output); it exists for scale baselines a store cannot reach yet.
//! `beamcraft-ui --store ROOT --screenshot FILE [--width W] [--height H] [--yaw RAD]
//! [--pitch RAD] [--projection perspective|ortho] [--zoom F32] [--pan X,Y]
//! [--colour-set TEXT] [--background light|dark] [--grids true|false] [--cli PATH]` renders the
//! store's published head once offscreen with the camera fitted over the whole scene and its
//! project grids, paints the grid lines and labels like the window (issue 543; `--grids false`
//! leaves them out), and writes
//! `FILE` as a PNG (created new, never overwritten), printing one JSON summary; `--yaw` and
//! `--pitch` (radians, `pitch` above the horizon) point the fitted camera from another
//! direction, `--projection ortho` switches to the orthographic projection, `--zoom EXP` is
//! the logarithmic zoom (frame lengths scale by e to its power, range -10 to 10) and
//! `--pan MM,MM` shifts the fitted target in screen-plane millimetres, +X to the right and
//! +Y up;
//! `--colour-set` names a built-in set (`Catalogue`, the default, or `Part classes`) or a
//! colour-sets file holding exactly one set of its own, and `--background light` renders over
//! a light paper tone instead of the dark clear colour (issue 537).
//! `--samples 1|4` overrides the viewport's MSAA default (4x on a hardware adapter that supports
//! it, 1x on a software one) in the window, in `--metrics` and in `--screenshot`.
//! `... --perf-run PLAN` opens the window as above, runs the scripted measurement `PLAN` in it
//! (see `beamcraft_ui::perf`), prints its report as one JSON object and closes; the exit status
//! is 1 when a phase failed.

use beamcraft_engine::synthetic::plate_scene;
use beamcraft_engine::{Limits, Representation, ViewPin, ViewSnapshot};
use beamcraft_model::digest::Sha256;
use beamcraft_ui::app::{App, Source, load_icon};
use beamcraft_ui::scene::Scene;
use beamcraft_ui::shell::Shell;
use beamcraft_ui::stats::Stats;
use beamcraft_ui::{Error, colour_sets, json_string, metrics, perf, screenshot, source};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn usage() -> Error {
    Error::Input(
        "usage: beamcraft-ui [--cli PATH] [--config-dir DIR] [--samples 1|4] (start page) | beamcraft-ui (--store ROOT | --workspace DIR [--published ROOT] | --source FILE --screenshot FILE) [--actor-id UUID] [--cli PATH] [--config-dir DIR] [--samples 1|4] [--metrics FRAMES [--width W] [--height H] | --screenshot FILE [--width W] [--height H] [--yaw RAD] [--pitch RAD] [--projection perspective|ortho] [--zoom EXP] [--pan MM,MM] [--colour-set TEXT] [--background light|dark] [--grids true|false] | --window-shot FILE.png | --perf-run PLAN] | --self-test FILE.json | --synthetic COUNT --metrics FRAMES [--samples 1|4]",
    )
}

fn options() -> Result<BTreeMap<String, String>, Error> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .take(29)
        .map(|a| a.into_string().map_err(|_| usage()))
        .collect::<Result<_, _>>()?;
    if !args.len().is_multiple_of(2) || args.len() > 28 {
        return Err(usage());
    }
    let mut map = BTreeMap::new();
    for pair in args.chunks(2) {
        let [key, value] = pair else {
            return Err(usage());
        };
        let known = matches!(
            key.as_str(),
            "--store"
                | "--source"
                | "--workspace"
                | "--published"
                | "--cli"
                | "--metrics"
                | "--width"
                | "--height"
                | "--synthetic"
                | "--perf-run"
                | "--samples"
                | "--screenshot"
                | "--yaw"
                | "--pitch"
                | "--projection"
                | "--zoom"
                | "--pan"
                | "--colour-set"
                | "--background"
                | "--grids"
                | "--config-dir"
                | "--actor-id"
                | "--window-shot"
                | "--self-test"
        );
        if !known || value.len() > 4096 || map.insert(key.clone(), value.clone()).is_some() {
            return Err(usage());
        }
    }
    Ok(map)
}

fn number(map: &BTreeMap<String, String>, key: &str, default: u32) -> Result<u32, Error> {
    map.get(key)
        .map_or(Ok(default), |v| v.parse().map_err(|_| usage()))
}

/// What a run printed and its exit status, or `None` for a window that just closed.
fn run() -> Result<Option<(String, u8)>, Error> {
    let map = options()?;
    // The scripted check of the view workflow runs on the window's state alone, without a window.
    if let Some(path) = map.get("--self-test") {
        if map.len() != 1 {
            return Err(usage());
        }
        let report = beamcraft_ui::selftest::run();
        let bytes = report
            .encode(&beamcraft_json::Limits::DEFAULT)
            .map_err(|_| Error::Input("the self-test report does not encode"))?;
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(Error::Io)?;
        }
        std::fs::write(&path, &bytes).map_err(Error::Io)?;
        let passed = !String::from_utf8_lossy(&bytes).contains("\"passed\":false");
        return Ok(Some((
            String::from_utf8_lossy(&bytes).into_owned(),
            if passed { 0 } else { 1 },
        )));
    }
    // The window's own picture after it settled (the start page or a model), unlike
    // `--screenshot`, which renders the model alone.
    if let Some(path) = map.get("--window-shot") {
        if map.contains_key("--metrics")
            || map.contains_key("--perf-run")
            || map.contains_key("--screenshot")
        {
            return Err(usage());
        }
        let _ = SHOT.set((
            PathBuf::from(path),
            std::sync::Arc::new(std::sync::Mutex::new(None)),
        ));
    }
    // One window, one set of counters; shared with its worker threads (see beamcraft_ui::stats).
    let stats = Arc::new(Stats::default());
    let samples = map
        .get("--samples")
        .map(|v| match v.as_str() {
            "1" => Ok(1),
            "4" => Ok(4),
            _ => Err(usage()),
        })
        .transpose()?;
    if map.contains_key("--synthetic") {
        if map.contains_key("--store")
            || !map.contains_key("--metrics")
            || map.contains_key("--perf-run")
        {
            return Err(usage());
        }
        let count = number(&map, "--synthetic", 1)?;
        let frames = number(&map, "--metrics", 300)?;
        let size = [
            number(&map, "--width", 1280)?,
            number(&map, "--height", 800)?,
        ];
        let pin = ViewPin::new(
            Sha256::digest(b"synthetic").map_err(|_| Error::Input("digest"))?,
            0,
        );
        let snapshot = ViewSnapshot::build(pin, plate_scene(count, false)?, &Limits::default())?;
        let scene = Scene::from_snapshot(&snapshot, Representation::Full)?;
        let json = metrics::measure(&scene, frames, size, samples, &stats)?.to_json();
        let json = json.replacen(
            "\"outcome\":\"viewport_metrics\"",
            "\"outcome\":\"viewport_metrics\",\"source\":\"synthetic\"",
            1,
        );
        return Ok(Some((json, 0)));
    }
    // A workspace is a store of its own: the window shows and edits it, and undo and redo act on it.
    let workspace = map.get("--workspace").map(PathBuf::from);
    let published = map.get("--published").map(PathBuf::from);
    if published.is_some() && workspace.is_none() {
        return Err(usage());
    }
    // A received IFC file is only a screenshot input: one read-only display, never a store
    // the window would open or edit.
    let ifc_source = map.get("--source").map(PathBuf::from);
    if ifc_source.is_some()
        && (map.contains_key("--store")
            || map.contains_key("--workspace")
            || map.contains_key("--published")
            || !map.contains_key("--screenshot"))
    {
        return Err(usage());
    }
    let cli = match map.get("--cli") {
        Some(path) => PathBuf::from(path),
        None => source::default_cli()?,
    };
    let config_dir = map.get("--config-dir").map(PathBuf::from);
    let store = match (map.get("--store"), &workspace) {
        (Some(store), None) => Some(PathBuf::from(store)),
        (None, Some(dir)) => Some(dir.clone()),
        // A --source run has no store: the screenshot branch below consumes it, and every
        // other path refuses it earlier.
        (None, None) if map.contains_key("--source") => None,
        // No model named: the window opens on the start page.
        (None, None)
            if !map.contains_key("--metrics")
                && !map.contains_key("--perf-run")
                && !map.contains_key("--screenshot")
                && !map.contains_key("--actor-id") =>
        {
            start_page(
                cli,
                map.get("--config-dir").map(PathBuf::from),
                samples,
                stats,
            )?;
            return shot_reply();
        }
        _ => return Err(usage()),
    };
    if map.contains_key("--metrics") {
        if map.contains_key("--perf-run")
            || map.contains_key("--screenshot")
            || map.contains_key("--yaw")
            || map.contains_key("--pitch")
            || map.contains_key("--grids")
            || config_dir.is_some()
        {
            return Err(usage());
        }
        let frames = number(&map, "--metrics", 300)?;
        let size = [
            number(&map, "--width", 1280)?,
            number(&map, "--height", 800)?,
        ];
        let snapshot = source::load(&cli, store.as_deref().ok_or_else(usage)?, &stats)?;
        let scene = Scene::from_snapshot(&snapshot, Representation::Full)?;
        return Ok(Some((
            metrics::measure(&scene, frames, size, samples, &stats)?.to_json(),
            0,
        )));
    }
    if let Some(out) = map.get("--screenshot") {
        if map.contains_key("--perf-run")
            || map.contains_key("--actor-id")
            || config_dir.is_some()
            || workspace.is_some()
            || published.is_some()
        {
            return Err(usage());
        }
        let size = [
            number(&map, "--width", 1280)?,
            number(&map, "--height", 800)?,
        ];
        let angles = match (map.get("--yaw"), map.get("--pitch")) {
            (None, None) => None,
            (yaw, pitch) => {
                let yaw: f32 = yaw
                    .map(|v| v.parse())
                    .transpose()
                    .map_err(|_| usage())?
                    .unwrap_or(0.0);
                let pitch: f32 = pitch
                    .map(|v| v.parse())
                    .transpose()
                    .map_err(|_| usage())?
                    .unwrap_or(0.0);
                if !yaw.is_finite()
                    || !pitch.is_finite()
                    || !(0.0..=1.52).contains(&pitch)
                    || yaw.abs() > 1e6
                {
                    return Err(usage());
                }
                Some((yaw, pitch))
            }
        };
        // `None` keeps the fitted orthographic default; both named projections switch.
        let ortho = match map.get("--projection").map(String::as_str) {
            None => None,
            Some("perspective") => Some(false),
            Some("ortho") => Some(true),
            Some(_) => return Err(usage()),
        };
        let zoom: f32 = map
            .get("--zoom")
            .map(|v| v.parse())
            .transpose()
            .map_err(|_| usage())?
            .unwrap_or(0.0);
        let pan = match map.get("--pan") {
            None => [0.0; 2],
            Some(text) => {
                let parts: Vec<f32> = text
                    .split(',')
                    .map(|v| v.parse())
                    .collect::<Result<_, _>>()
                    .map_err(|_| usage())?;
                let [x, y] = parts.as_slice() else {
                    return Err(usage());
                };
                if !x.is_finite() || !y.is_finite() {
                    return Err(usage());
                }
                [*x, *y]
            }
        };
        if !zoom.is_finite() || zoom.abs() > 10.0 {
            // The refusal names the scale and the range itself (issue 607): the whole usage
            // line says neither.
            return Err(Error::Input(
                "--zoom EXP is the logarithmic zoom: lengths in the frame scale by e to the power of EXP; the accepted range is -10 to 10",
            ));
        }
        let background = match map.get("--background").map(String::as_str) {
            None | Some("dark") => screenshot::Background::Dark,
            Some("light") => screenshot::Background::Light,
            Some(_) => return Err(usage()),
        };
        // The project grids ride the screenshot like the window's (issue 543); `--grids false`
        // leaves them out of both the picture and the fit. A source IFC file carries no
        // project grids: they belong to a store, so only the model is drawn and fitted.
        let (snapshot, grids) = if let Some(source) = &ifc_source {
            if map.get("--grids").map(String::as_str) == Some("true") {
                return Err(usage());
            }
            (
                source::load_source(&cli, source, &stats)?,
                beamcraft_ui::grid::GridScene::default(),
            )
        } else {
            let store = store.as_deref().ok_or_else(usage)?;
            match map.get("--grids").map(String::as_str) {
                None | Some("true") => source::load_with_grids(&cli, store, &stats)?,
                Some("false") => (
                    source::load(&cli, store, &stats)?,
                    beamcraft_ui::grid::GridScene::default(),
                ),
                Some(_) => return Err(usage()),
            }
        };
        // The default catalogue colours ride the screenshot (issue 535): the same tints the
        // window gives a freshly opened model, evaluated by the window's own path. A
        // `--colour-set` names a built-in preset or a colour-sets file holding exactly one
        // set of its own (issue 537); the set is evaluated by the same path the window uses,
        // and a headless screenshot reads no colour-sets file on its own.
        let set = match map.get("--colour-set").map(String::as_str) {
            None | Some(colour_sets::CATALOGUE_SET_NAME) => colour_sets::catalogue_set(),
            Some(colour_sets::CLASS_SET_NAME) => colour_sets::class_set(),
            Some(path) => {
                let bytes = std::fs::read(path)
                    .map_err(|_| Error::Input("the colour-sets file could not be read"))?;
                let sets = colour_sets::Sets::decode(&bytes).map_err(Error::Input)?;
                let mut own = sets.list().iter().filter(|set| {
                    set.name != colour_sets::CATALOGUE_SET_NAME
                        && set.name != colour_sets::CLASS_SET_NAME
                });
                let Some(set) = own.next().cloned() else {
                    return Err(Error::Input(
                        "a colour-sets file must hold exactly one set of its own",
                    ));
                };
                if own.next().is_some() {
                    return Err(Error::Input(
                        "a colour-sets file must hold exactly one set of its own",
                    ));
                }
                set
            }
        };
        // The catalogue preset is the snapshot's own kinds: derived, not evaluated (the same
        // shift the window made for issue 562 items 20 and 34). Every other set evaluates on
        // the store head the way the window does, and a headless screenshot reads no
        // colour-sets file on its own. A source file has no store to evaluate against, so
        // only the catalogue preset rides its picture.
        if ifc_source.is_some() && set.name != colour_sets::CATALOGUE_SET_NAME {
            return Err(usage());
        }
        let overlay = if set.name == colour_sets::CATALOGUE_SET_NAME {
            colour_sets::catalogue_overlay(snapshot.elements())
        } else {
            colour_sets::evaluate(
                &cli,
                store.as_deref().ok_or_else(usage)?,
                &set,
                &AtomicBool::new(false),
                &stats,
            )
            .map_err(Error::Cli)?
        };
        let scene = Scene::from_snapshot_showing(
            &snapshot,
            Representation::Full,
            &BTreeSet::new(),
            Some(&overlay),
            beamcraft_engine::KindFilter::SHOW_ALL,
        )?;
        // The resolved D06 annotations ride the screenshot like the grid guides (issue 562
        // item 37): the same `annotation-list` query the window's panel runs, painted by the
        // same painter. A store without annotations or a failed query renders without them;
        // a source file has no store to query, so it renders without them.
        let annotations = if ifc_source.is_some() {
            Vec::new()
        } else {
            beamcraft_ui::command::run(
                &cli,
                &[
                    "annotation-list".to_owned(),
                    "--store".to_owned(),
                    store
                        .as_deref()
                        .ok_or_else(usage)?
                        .to_string_lossy()
                        .into_owned(),
                ],
                std::time::Duration::from_secs(30),
                &stats,
            )
            .ok()
            .and_then(|out| {
                beamcraft_ui::app::annotation_panel::parse_rows(out.code, &out.stdout).ok()
            })
            .unwrap_or_default()
        };
        let saved = screenshot::capture(
            &scene,
            &grids,
            Path::new(out),
            size,
            samples,
            &screenshot::CameraPose {
                angles,
                ortho,
                zoom,
                pan,
            },
            background,
            &annotations,
            &stats,
        )?
        .to_json();
        return Ok(Some((saved, 0)));
    }
    let plan = map.get("--perf-run").map(|p| perf::parse(p)).transpose()?;
    let (report, reports) = std::sync::mpsc::channel();
    let source = Source {
        // Every path that reaches the window named a store or a workspace; a --source run
        // returned in the screenshot branch above.
        store: store.ok_or_else(usage)?,
        cli,
        workspace,
        published,
        samples,
        actor: map.get("--actor-id").cloned(),
        environment: None,
        author: None,
        config_dir,
    };
    let perf_run = plan.is_some();
    window(
        egui::vec2(1400.0, 900.0),
        Box::new(move |cc| {
            let perf = plan.map(|phases| (phases, report));
            Ok(Box::new(App::with_perf_run(cc, source, perf, stats)?))
        }),
    )?;
    if !perf_run {
        return shot_reply();
    }
    match reports.try_recv() {
        Ok((json, ok)) => Ok(Some((json, if ok { 0 } else { 1 }))),
        Err(_) => Err(Error::Input(
            "the window closed before the perf run finished",
        )),
    }
}

/// The window opened without a model: the start page, then the model chosen on it.
fn start_page(
    cli: PathBuf,
    config_dir: Option<PathBuf>,
    samples: Option<u32>,
    stats: Arc<Stats>,
) -> Result<Option<(String, u8)>, Error> {
    window(
        beamcraft_ui::start::LAUNCHER_SIZE,
        Box::new(move |cc| Ok(Box::new(Shell::new(cc, cli, config_dir, samples, stats)))),
    )?;
    Ok(None)
}

/// The `--window-shot FILE` of this run: the file and where its outcome lands.
static SHOT: std::sync::OnceLock<(PathBuf, beamcraft_ui::capture::Outcome)> =
    std::sync::OnceLock::new();

/// The reply of a `--window-shot` run once its window closed.
fn shot_reply() -> Result<Option<(String, u8)>, Error> {
    let Some((path, outcome)) = SHOT.get() else {
        return Ok(None);
    };
    let result = outcome.lock().ok().and_then(|mut o| o.take());
    match result {
        Some(Ok(())) => Ok(Some((
            format!(
                "{{\"outcome\":\"screenshot_saved\",\"path\":{},\"frames\":{}}}",
                json_string(&path.to_string_lossy()),
                beamcraft_ui::capture::FRAMES
            ),
            0,
        ))),
        Some(Err(message)) => Err(Error::Graphics(message)),
        None => Err(Error::Graphics(
            "the window closed before its picture was taken".into(),
        )),
    }
}

/// Runs one native window until it closes.
fn window(size: egui::Vec2, create: eframe::AppCreator<'_>) -> Result<(), Error> {
    let create: eframe::AppCreator<'_> = match SHOT.get() {
        Some((path, outcome)) => {
            let (path, outcome) = (path.clone(), std::sync::Arc::clone(outcome));
            Box::new(move |cc| {
                let inner = create(cc)?;
                Ok(Box::new(beamcraft_ui::capture::Capture::new(
                    inner, path, outcome,
                )))
            })
        }
        None => create,
    };
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends =
        wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::METAL;
    // The wireframe render mode draws triangle edges: a native desktop feature (Metal, DX12,
    // and Vulkan with `fillModeNonSolid`), requested explicitly like the renderer's own
    // headless device does.
    setup.device_descriptor = std::sync::Arc::new(|_adapter| wgpu::DeviceDescriptor {
        label: Some("BeamCraft viewport"),
        required_features: wgpu::Features::POLYGON_MODE_LINE,
        ..Default::default()
    });
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(size)
        .with_title("BeamCraft")
        .with_icon(load_icon());
    // macOS keeps the native frame and its traffic lights over a transparent title bar, with
    // our header drawn under them; elsewhere the window is frameless and draws its own controls.
    let viewport = if beamcraft_ui::theme::NATIVE_WINDOW_CONTROLS {
        viewport
            .with_decorations(true)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
    } else {
        viewport.with_decorations(false)
    };
    // The Wayland app ID and X11 window class match the desktop entry `install-linux.sh`
    // registers, so the running window groups with its launcher and shows its icon.
    #[cfg(target_os = "linux")]
    let viewport = viewport.with_app_id("com.apibim.BeamCraft");
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::WgpuConfiguration {
            wgpu_setup: setup.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    // Window-system libraries are loaded at run time by third-party crates, which panic when one
    // is missing; report that as an error instead of crashing.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        eframe::run_native("BeamCraft", options, create)
    }))
    .map_err(|_| {
        Error::Graphics("the window could not be created (missing window-system library?)".into())
    })?
    .map_err(|e| Error::Graphics(e.to_string()))
}

fn main() -> ExitCode {
    // The window reads layers in this process through `session` (environments, foreign layers),
    // so the steel discipline joins the platform registry first, as in the CLI.
    steel::layer::install();
    match run() {
        Ok(Some((json, code))) => {
            println!("{json}");
            ExitCode::from(code)
        }
        Ok(None) => ExitCode::SUCCESS,
        // A relayed CLI rejection keeps its own code, message and diagnostics (issue 555
        // finding 6): `screenshot-view` re-issues them unchanged.
        Err(Error::Rejected(r)) => {
            println!(
                "{{\"outcome\":\"rejected\",\"code\":{},\"message\":{},\"diagnostics\":{}}}",
                json_string(&r.code),
                json_string(&r.message),
                r.diagnostics
            );
            ExitCode::from(2)
        }
        Err(e) => {
            let code = match e {
                Error::NoAdapter | Error::Graphics(_) => 3,
                _ => 2,
            };
            println!(
                "{{\"outcome\":\"rejected\",\"message\":{}}}",
                json_string(&e.to_string())
            );
            ExitCode::from(code)
        }
    }
}
