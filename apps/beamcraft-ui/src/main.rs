// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `beamcraft-ui` with no arguments opens a per-user default workspace under the system data
//! directory (the published store and the workspace fork are created on demand), so a
//! double-clicked app bundle launches with undo, redo and Publish.
//! `beamcraft-ui --store ROOT [--cli PATH]` opens the window on the published revision.
//! `beamcraft-ui --workspace DIR [--cli PATH]` opens it on a personal workspace (a store made by
//! `beamcraft workspace-create`), which also enables undo and redo. Adding `--published ROOT`
//! (the store the workspace was forked from) enables Publish (`beamcraft workspace-publish`).
//! `beamcraft-ui --store ROOT --metrics FRAMES [--width W] [--height H] [--cli PATH]` renders the
//! same snapshot offscreen, headless, and prints baseline GPU metrics as one JSON object.
//! `beamcraft-ui --synthetic COUNT --metrics FRAMES [--width W] [--height H]` measures the engine
//! snapshot of COUNT synthetic kernel plates instead of a store (labelled `synthetic` in the
//! output); it exists for scale baselines a store cannot reach yet.
//! `--samples 1|4` overrides the viewport's MSAA default (4x on a hardware adapter that supports
//! it, 1x on a software one) in the window and in `--metrics`.
//! `... --perf-run PLAN` opens the window as above, runs the scripted measurement `PLAN` in it
//! (see `beamcraft_ui::perf`), prints its report as one JSON object and closes; the exit status
//! is 1 when a phase failed.

use beamcraft_engine::synthetic::plate_scene;
use beamcraft_engine::{Limits, Representation, ViewPin, ViewSnapshot};
use beamcraft_model::digest::Sha256;
use beamcraft_ui::app::{App, Source, load_icon};
use beamcraft_ui::scene::Scene;
use beamcraft_ui::stats::Stats;
use beamcraft_ui::{Error, json_string, metrics, perf, source};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

fn usage() -> Error {
    Error::Input(
        "usage: beamcraft-ui (--store ROOT | --workspace DIR [--published ROOT]) [--cli PATH] [--samples 1|4] [--metrics FRAMES [--width W] [--height H] | --perf-run PLAN] | --synthetic COUNT --metrics FRAMES [--samples 1|4]",
    )
}

fn options() -> Result<BTreeMap<String, String>, Error> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .take(17)
        .map(|a| a.into_string().map_err(|_| usage()))
        .collect::<Result<_, _>>()?;
    if !args.len().is_multiple_of(2) || args.len() > 16 {
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
                | "--workspace"
                | "--published"
                | "--cli"
                | "--metrics"
                | "--width"
                | "--height"
                | "--synthetic"
                | "--perf-run"
                | "--samples"
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
    let cli = match map.get("--cli") {
        Some(path) => PathBuf::from(path),
        None => source::default_cli()?,
    };
    let (store, workspace, published) = if map.is_empty() {
        // No arguments at all (a double-clicked app bundle): open a per-user default
        // workspace, created on demand, so the packaged application launches.
        let root = source::default_root()?;
        let (workspace, published) = source::default_workspace_at(&root, &cli, &stats)?;
        (workspace.clone(), Some(workspace), Some(published))
    } else {
        match (map.get("--store"), &workspace) {
            (Some(store), None) => (PathBuf::from(store), None, None),
            (None, Some(dir)) => (dir.clone(), workspace.clone(), published),
            _ => return Err(usage()),
        }
    };
    if map.contains_key("--metrics") {
        if map.contains_key("--perf-run") {
            return Err(usage());
        }
        let frames = number(&map, "--metrics", 300)?;
        let size = [
            number(&map, "--width", 1280)?,
            number(&map, "--height", 800)?,
        ];
        let snapshot = source::load(&cli, &store, &stats)?;
        let scene = Scene::from_snapshot(&snapshot, Representation::Full)?;
        return Ok(Some((
            metrics::measure(&scene, frames, size, samples, &stats)?.to_json(),
            0,
        )));
    }
    let plan = map.get("--perf-run").map(|p| perf::parse(p)).transpose()?;
    let (report, reports) = std::sync::mpsc::channel();
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends =
        wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::METAL;
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1400.0, 900.0])
        .with_title("BeamCraft")
        .with_icon(load_icon());
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::WgpuConfiguration {
            wgpu_setup: setup.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let source = Source {
        store,
        cli,
        workspace,
        published,
        samples,
    };
    let perf_run = plan.is_some();
    // Window-system libraries are loaded at run time by third-party crates, which panic when one
    // is missing; report that as an error instead of crashing.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        eframe::run_native(
            "BeamCraft",
            options,
            Box::new(move |cc| {
                let perf = plan.map(|phases| (phases, report));
                Ok(Box::new(App::with_perf_run(cc, source, perf, stats)?))
            }),
        )
    }))
    .map_err(|_| {
        Error::Graphics("the window could not be created (missing window-system library?)".into())
    })?
    .map_err(|e| Error::Graphics(e.to_string()))?;
    if !perf_run {
        return Ok(None);
    }
    match reports.try_recv() {
        Ok((json, ok)) => Ok(Some((json, if ok { 0 } else { 1 }))),
        Err(_) => Err(Error::Input(
            "the window closed before the perf run finished",
        )),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(Some((json, code))) => {
            println!("{json}");
            ExitCode::from(code)
        }
        Ok(None) => ExitCode::SUCCESS,
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
