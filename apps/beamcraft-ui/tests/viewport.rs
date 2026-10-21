// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0.

//! V03 end to end: the accepted revision of a real store, exported by the `beamcraft` CLI in a
//! separate process, rebuilt by the engine, turned into an instanced scene and rendered offscreen
//! with an actual pixel readback. Without a native adapter the render step is skipped and says so;
//! without the CLI binary next to this package's binary the store test is skipped and says so.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_engine::synthetic::plate_scene;
use beamcraft_engine::{Limits, Representation, ViewPin, ViewSnapshot};
use beamcraft_model::digest::Sha256;
use beamcraft_ui::camera::Camera;
use beamcraft_ui::render::{Gpu, Renderer, Uploaded, wait};
use beamcraft_ui::scene::Scene;
use beamcraft_ui::{Error, source};
use std::path::{Path, PathBuf};
use std::process::Command;

fn gpu() -> Option<Gpu> {
    match pollster::block_on(Gpu::headless()) {
        Ok(gpu) => {
            let info = gpu.adapter.get_info();
            eprintln!(
                "render test adapter: {} / {:?} / {:?}",
                info.name, info.device_type, info.backend
            );
            Some(gpu)
        }
        Err(Error::NoAdapter) => {
            eprintln!("SKIP render: no native wgpu adapter");
            None
        }
        Err(e) => panic!("adapter or device failed: {e}"),
    }
}

/// Renders the framed scene; returns a corner pixel, the centre pixel and the lit pixel count.
fn render(gpu: &Gpu, scene: &Scene) -> ([u8; 4], [u8; 4], usize) {
    let (target, extent) = scene.framing();
    let view = Camera::new(target, extent, 1.0).unwrap().view(1.0).unwrap();
    let renderer = Renderer::new(&gpu.adapter, &gpu.device, 128, 128).unwrap();
    let uploaded = Uploaded::new(&gpu.device, &gpu.queue, scene).unwrap();
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let index = renderer.draw(&gpu.device, &gpu.queue, &uploaded, view);
    wait(&gpu.device, index).unwrap();
    let (pixels, row) = renderer.readback(&gpu.device, &gpu.queue).unwrap();
    assert!(pollster::block_on(scope.pop()).is_none());
    let pixel = |x: usize, y: usize| -> [u8; 4] {
        let at = y * row as usize + x * 4;
        pixels[at..at + 4].try_into().unwrap()
    };
    let lit = (0..128)
        .flat_map(|y| (0..128).map(move |x| (x, y)))
        .filter(|(x, y)| is_lit(pixel(*x, *y)))
        .count();
    (pixel(0, 0), pixel(64, 64), lit)
}

fn is_background([r, g, b, a]: [u8; 4]) -> bool {
    (4..=6).contains(&r) && (7..=9).contains(&g) && (12..=14).contains(&b) && a == 255
}

fn is_lit([r, g, b, a]: [u8; 4]) -> bool {
    r > 20 && g > 60 && b > 100 && r < g && g < b && a == 255
}

#[test]
fn an_engine_snapshot_renders_lit_geometry_on_a_background() {
    let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
    let snapshot =
        ViewSnapshot::build(pin, plate_scene(1, false).unwrap(), &Limits::default()).unwrap();
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    let Some(gpu) = gpu() else { return };
    let (corner, centre, _) = render(&gpu, &scene);
    assert!(is_background(corner), "background {corner:?}");
    assert!(is_lit(centre), "lit plate {centre:?}");
}

/// Draws `uploaded` framed on `scene` and returns the lit pixel count.
fn lit_pixels(gpu: &Gpu, scene: &Scene, uploaded: &Uploaded) -> usize {
    let (target, extent) = scene.framing();
    let view = Camera::new(target, extent, 1.0).unwrap().view(1.0).unwrap();
    let renderer = Renderer::new(&gpu.adapter, &gpu.device, 128, 128).unwrap();
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let index = renderer.draw(&gpu.device, &gpu.queue, uploaded, view);
    wait(&gpu.device, index).unwrap();
    let (pixels, row) = renderer.readback(&gpu.device, &gpu.queue).unwrap();
    assert!(pollster::block_on(scope.pop()).is_none());
    (0..128usize)
        .flat_map(|y| (0..128usize).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let at = y * row as usize + x * 4;
            is_lit(pixels[at..at + 4].try_into().unwrap())
        })
        .count()
}

#[test]
fn a_refresh_reuses_the_meshes_on_the_device_and_still_draws_them() {
    let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
    let first =
        ViewSnapshot::build(pin, plate_scene(12, false).unwrap(), &Limits::default()).unwrap();
    let before = Scene::from_snapshot(&first, Representation::Full).unwrap();
    // The next revision adds four plates; its first twelve meshes are already on the device.
    let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 2);
    let second =
        ViewSnapshot::build(pin, plate_scene(16, false).unwrap(), &Limits::default()).unwrap();
    let Some(gpu) = gpu() else { return };
    let old = Uploaded::upload(&gpu.device, &gpu.queue, &before, None).unwrap();
    let resident = old.resident();
    let after = Scene::from_snapshot_reusing(&second, Representation::Full, &resident).unwrap();
    let kept = after.meshes.iter().filter(|m| m.resident).count();
    assert_eq!(kept, before.meshes.len());
    assert!(
        after
            .meshes
            .iter()
            .filter(|m| m.resident)
            .all(|m| m.vertices.is_empty())
    );
    let new = Uploaded::upload(&gpu.device, &gpu.queue, &after, Some(&old)).unwrap();
    assert_eq!(new.shared_vertex_buffers(&old), kept);
    // Only the new meshes' vertices and every instance buffer were sent again.
    let full = Scene::from_snapshot(&second, Representation::Full).unwrap();
    let sent: usize = full
        .meshes
        .iter()
        .map(|m| if resident.contains(&m.digest) { 0 } else { m.vertices.len() } + m.instances.len())
        .sum();
    assert_eq!(new.bytes, sent as u64);
    // The reused buffers draw the same image as a fresh upload of the whole scene.
    let fresh = Uploaded::new(&gpu.device, &gpu.queue, &full).unwrap();
    let lit = lit_pixels(&gpu, &full, &new);
    assert!(lit > 0);
    assert_eq!(lit, lit_pixels(&gpu, &full, &fresh));
    // A resident mesh the device does not hold is refused, never drawn from nothing.
    assert!(Uploaded::upload(&gpu.device, &gpu.queue, &after, None).is_err());
}

fn cli() -> Option<PathBuf> {
    let ui = Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"));
    let cli = ui
        .parent()?
        .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
    if cli.is_file() {
        Some(cli)
    } else {
        eprintln!("SKIP store test: build beamcraft-cli first (cargo test --workspace does)");
        None
    }
}

fn run(cli: &Path, args: &[&str]) -> String {
    let output = Command::new(cli).args(args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_accepted_revision_of_a_store_is_the_viewport_scene() {
    let Some(cli) = cli() else { return };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("v03-tests")
        .join(format!("store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = root.to_str().unwrap();
    let plate = "019a7bf0-0000-7000-8000-000000000010";
    run(
        &cli,
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--new-id",
            plate,
        ],
    );
    for i in 1..=5 {
        let offset = format!("{},0,0", i * 1200);
        run(
            &cli,
            &[
                "copy-element",
                "--store",
                store,
                "--id",
                plate,
                "--offset",
                &offset,
            ],
        );
    }
    run(
        &cli,
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "300",
            "--length",
            "300",
            "--thickness",
            "20",
            "--translation",
            "0,2000,0",
        ],
    );
    let snapshot = source::load(&cli, &root).unwrap();
    // The viewport sees the engine's snapshot of the head: the same token plate-inspect reports.
    let inspect = run(&cli, &["plate-inspect", "--store", store]);
    assert!(inspect.contains(snapshot.token().as_str()));
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    // Units and instancing: six plates share one mesh, the seventh has its own; bounds are in
    // millimetres (0 .. 6000 + 1000 in X, 0 .. 2300 in Y, 0 .. 20 in Z).
    assert_eq!(scene.elements, 7);
    assert_eq!(scene.counts(), (2, 7));
    let mut instances: Vec<u32> = scene.meshes.iter().map(|m| m.instance_count).collect();
    instances.sort();
    assert_eq!(instances, [1, 6]);
    let size = [0, 1, 2].map(|a| scene.max[a] - scene.min[a]);
    assert!((size[0] - 7000.0).abs() < 1e-3, "{size:?}");
    assert!((size[1] - 2300.0).abs() < 1e-3, "{size:?}");
    assert!((size[2] - 20.0).abs() < 1e-3, "{size:?}");
    assert_eq!(scene.origin_mm, [3500.0, 1150.0, 10.0]);
    // A store without a revision is the typed empty state, not a failure.
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    match source::load(&cli, &empty) {
        Err(Error::NoRevision) => (),
        other => panic!("expected the empty-store state, got {other:?}"),
    }
    let Some(gpu) = gpu() else { return };
    let (corner, _, lit) = render(&gpu, &scene);
    assert!(is_background(corner), "background {corner:?}");
    // Seven plates framed in a 128 x 128 target cover a visible share of it.
    assert!(lit > 200, "lit pixels {lit}");
    let _ = std::fs::remove_dir_all(&root);
}
