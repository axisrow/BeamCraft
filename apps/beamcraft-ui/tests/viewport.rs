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
use beamcraft_engine::{Limits, RenderMode, Representation, ViewPin, ViewSnapshot};
use beamcraft_model::digest::Sha256;
use beamcraft_ui::camera::{Camera, View};
use beamcraft_ui::render::{Gpu, Renderer, Uploaded, wait};
use beamcraft_ui::scene::{Scene, SelectedDraw};
use beamcraft_ui::stats::Stats;
use beamcraft_ui::tools::{ToolKind, Tools};
use beamcraft_ui::{Error, command, source};
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
    render_view(gpu, scene, view)
}

/// Renders `scene` with `view`; same evidence as [`render`].
fn render_view(gpu: &Gpu, scene: &Scene, view: View) -> ([u8; 4], [u8; 4], usize) {
    let stats = Stats::default();
    let renderer = Renderer::new(&gpu.adapter, &gpu.device, 128, 128, &stats).unwrap();
    let uploaded = Uploaded::new(&gpu.device, &gpu.queue, scene, &stats).unwrap();
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let index = renderer.draw(
        &gpu.device,
        &gpu.queue,
        &uploaded,
        view,
        &stats,
        RenderMode::Shaded,
    );
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

/// A pixel of the model: the logo's cream and grey-teal, lit, never darker than its unlit side
/// (grey-teal at the 0.55 ambient term is about 66, 100, 108).
fn is_lit([r, g, b, a]: [u8; 4]) -> bool {
    r > 50 && g > 80 && b > 80 && a == 255
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

/// V13: the orthographic projection draws the same framed scene, in its own scale.
#[test]
fn an_orthographic_camera_renders_lit_geometry_on_a_background() {
    let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
    let snapshot =
        ViewSnapshot::build(pin, plate_scene(1, false).unwrap(), &Limits::default()).unwrap();
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    let Some(gpu) = gpu() else { return };
    let (target, extent) = scene.framing();
    let mut camera = Camera::new(target, extent, 1.0).unwrap();
    camera.set_projection(true);
    let (corner, centre, lit) = render_view(&gpu, &scene, camera.view(1.0).unwrap());
    assert!(is_background(corner), "background {corner:?}");
    assert!(is_lit(centre), "lit plate {centre:?}");
    assert!(lit > 0);
}

/// Draws `uploaded` framed on `scene` and returns the lit pixel count.
fn lit_pixels(gpu: &Gpu, scene: &Scene, uploaded: &Uploaded) -> usize {
    let (target, extent) = scene.framing();
    let view = Camera::new(target, extent, 1.0).unwrap().view(1.0).unwrap();
    let renderer = Renderer::new(&gpu.adapter, &gpu.device, 128, 128, &Stats::default()).unwrap();
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let index = renderer.draw(
        &gpu.device,
        &gpu.queue,
        uploaded,
        view,
        &Stats::default(),
        RenderMode::Shaded,
    );
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
fn the_five_render_modes_draw_five_different_pictures_of_one_scene() {
    let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
    let snapshot =
        ViewSnapshot::build(pin, plate_scene(1, false).unwrap(), &Limits::default()).unwrap();
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    let Some(gpu) = gpu() else { return };
    let (target, extent) = scene.framing();
    let view = Camera::new(target, extent, 1.0).unwrap().view(1.0).unwrap();
    let stats = Stats::default();
    let renderer = Renderer::new(&gpu.adapter, &gpu.device, 128, 128, &stats).unwrap();
    let uploaded = Uploaded::new(&gpu.device, &gpu.queue, &scene, &stats).unwrap();
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let picture = |mode| -> (usize, [u8; 4]) {
        let index = renderer.draw(&gpu.device, &gpu.queue, &uploaded, view, &stats, mode);
        wait(&gpu.device, index).unwrap();
        let (pixels, row) = renderer.readback(&gpu.device, &gpu.queue).unwrap();
        let pixel = |x: usize, y: usize| -> [u8; 4] {
            pixels[y * row as usize + x * 4..][..4].try_into().unwrap()
        };
        let lit = (0..128usize)
            .flat_map(|y| (0..128usize).map(move |x| (x, y)))
            .filter(|(x, y)| is_lit(pixel(*x, *y)))
            .count();
        (lit, pixel(64, 64))
    };
    // Shaded is the existing picture: the plate fills the centre, lit.
    let (shaded_lit, shaded_centre) = picture(RenderMode::Shaded);
    assert!(is_lit(shaded_centre), "shaded centre {shaded_centre:?}");
    // Wireframe draws the edges only: some lit pixels, far fewer than the fill.
    let (wire_lit, _) = picture(RenderMode::Wireframe);
    assert!(
        wire_lit > 0 && wire_lit < shaded_lit,
        "wire {wire_lit} of {shaded_lit}"
    );
    // Grey is lit and colourless.
    let (grey_lit, grey_centre) = picture(RenderMode::Grey);
    assert!(is_lit(grey_centre), "grey centre {grey_centre:?}");
    assert!(grey_lit > 0);
    let [r, g, b, _] = grey_centre;
    assert!(
        (i16::from(r) - i16::from(g)).abs() <= 2 && (i16::from(g) - i16::from(b)).abs() <= 2,
        "grey is colourless: {grey_centre:?}"
    );
    // Transparent blends with the background: a picture of its own.
    let (transparent_lit, transparent_centre) = picture(RenderMode::Transparent);
    assert!(transparent_lit > 0);
    assert!(
        transparent_centre != shaded_centre && !is_background(transparent_centre),
        "transparent centre {transparent_centre:?}"
    );
    // Selected only shows nothing until the selected element is drawn alone.
    let (empty_lit, _) = picture(RenderMode::SelectedOnly);
    assert_eq!(empty_lit, 0);
    let id = &snapshot.elements()[0].id;
    let selected = SelectedDraw::of(&snapshot, id, scene.origin_mm).unwrap();
    let index = renderer
        .draw_one(
            &gpu.device,
            &gpu.queue,
            &uploaded,
            view,
            &stats,
            &selected.digest,
            &selected.instance,
        )
        .expect("the selected element's mesh is on the device");
    wait(&gpu.device, index).unwrap();
    let (pixels, row) = renderer.readback(&gpu.device, &gpu.queue).unwrap();
    let selected_lit = (0..128usize)
        .flat_map(|y| (0..128usize).map(move |x| (x, y)))
        .filter(|(x, y)| is_lit(pixels[y * row as usize + x * 4..][..4].try_into().unwrap()))
        .count();
    assert_eq!(
        selected_lit, shaded_lit,
        "the one element fills like the scene"
    );
    assert!(pollster::block_on(scope.pop()).is_none());
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
    let fresh = Uploaded::new(&gpu.device, &gpu.queue, &full, &Stats::default()).unwrap();
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
    let stats = Stats::default();
    let snapshot = source::load(&cli, &root, &stats).unwrap();
    // The viewport sees the engine's snapshot of the head: the same token inspect-elements reports.
    let inspect = run(&cli, &["inspect-elements", "--store", store]);
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
    match source::load(&cli, &empty, &stats) {
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

#[test]
fn an_attached_reference_is_uploaded_and_renders_visible_pixels() {
    let Some(cli) = cli() else { return };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("v03-tests")
        .join(format!("reference-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = root.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let file = root.join("box.usda");
    // Contributor-original synthetic source, MPL-2.0; no third-party model.
    let bytes = b"#usda 1.0\n(defaultPrim = \"Box\"\nmetersPerUnit = 0.001\nupAxis = \"Z\")\ndef Cube \"Box\" { double size = 2000 }\n";
    std::fs::write(&file, bytes).unwrap();
    let id = "019a7bf0-0000-7000-8000-000000000137";
    run(
        &cli,
        &[
            "attach-reference",
            "--store",
            store.to_str().unwrap(),
            "--source",
            file.to_str().unwrap(),
            "--new-id",
            id,
            "--translation",
            "5000,1000,2000",
        ],
    );
    let snapshot = source::load(&cli, &store, &Stats::default()).unwrap();
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    assert_eq!(
        scene.elements, 1,
        "a reference-only store is not an empty viewport"
    );
    assert_eq!(scene.counts(), (1, 1));
    assert_eq!(scene.triangles, 12);
    assert_eq!(scene.origin_mm, [5000.0, 1000.0, 2000.0]);
    assert_eq!(scene.min, [-1000.0; 3]);
    assert_eq!(scene.max, [1000.0; 3]);
    assert_eq!(scene.meshes[0].vertex_count, 36);
    let mut identity_instance = vec![0u8; 24];
    identity_instance.extend_from_slice(&1.0f32.to_le_bytes());
    assert_eq!(scene.meshes[0].instances, identity_instance);
    assert_eq!(
        Scene::from_snapshot(&snapshot, Representation::Simplified).unwrap(),
        scene,
        "both display representations include the reference",
    );
    run(
        &cli,
        &[
            "move-reference",
            "--store",
            store.to_str().unwrap(),
            "--id",
            id,
            "--translation",
            "9000,2000,3000",
        ],
    );
    let moved = source::load(&cli, &store, &Stats::default()).unwrap();
    assert_ne!(snapshot.token(), moved.token());
    assert_eq!(
        snapshot.meshes(),
        moved.meshes(),
        "a move reuses the foreign mesh"
    );
    let moved = Scene::from_snapshot(&moved, Representation::Full).unwrap();
    assert_eq!(moved.origin_mm, [9000.0, 2000.0, 3000.0]);
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(&root);
    let Some(gpu) = gpu() else { return };
    let (corner, centre, lit) = render(&gpu, &scene);
    eprintln!("reference render: {lit} lit pixels in the 128 x 128 target");
    assert!(is_background(corner), "background {corner:?}");
    assert!(is_lit(centre), "reference at the centre {centre:?}");
    assert!(
        lit > 200,
        "the reference must cover visible pixels, got {lit}"
    );
    assert_eq!(render(&gpu, &moved), (corner, centre, lit));
}

/// Full readback of the framed scene, for pixel-by-pixel comparisons.
fn render_pixels(gpu: &Gpu, scene: &Scene) -> Vec<u8> {
    let (target, extent) = scene.framing();
    let view = Camera::new(target, extent, 1.0).unwrap().view(1.0).unwrap();
    let stats = Stats::default();
    let renderer = Renderer::new(&gpu.adapter, &gpu.device, 128, 128, &stats).unwrap();
    let uploaded = Uploaded::new(&gpu.device, &gpu.queue, scene, &stats).unwrap();
    let index = renderer.draw(
        &gpu.device,
        &gpu.queue,
        &uploaded,
        view,
        &stats,
        RenderMode::Shaded,
    );
    wait(&gpu.device, index).unwrap();
    renderer.readback(&gpu.device, &gpu.queue).unwrap().0
}

/// A 45-degree sloped beam, created through the UI's own tool state machine, renders a
/// silhouette that the roll moves (issue #134): the committed member carries `--roll` in its
/// exact CLI argument list, the snapshot bounds match the unrolled analytic section placement,
/// and the rolled render differs pixel by pixel from the unrolled one.
#[test]
fn an_oriented_beam_created_through_the_ui_renders_a_rotated_silhouette() {
    let Some(cli) = cli() else { return };
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("v03-tests-oriented");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let stats = Stats::default();
    // Both stores go through the same path the Commit button runs: form -> preview -> commit.
    let member_of = |store: &Path, roll: &str| {
        let mut tools = Tools::default();
        let definitions = command::definitions(&cli, store, &stats).unwrap();
        tools.apply_definitions(Ok(definitions));
        let digest = tools.definitions.profiles[0].digest.clone();
        assert!(tools.definitions.profiles[0].rect_mm.is_some());
        tools.active = ToolKind::Member;
        tools.member.profile = Some(digest);
        tools
            .member
            .set_endpoints([0.0, 0.0, 0.0], [4000.0, 0.0, 4000.0]);
        tools.member.roll = roll.into();
        let staged = tools.preview(store).unwrap().clone();
        (tools, staged)
    };
    let store_of = |name: &str| {
        let store = base.join(name);
        std::fs::create_dir_all(&store).unwrap();
        // The first revision comes from the pinned rectangle profile the forms list.
        run(
            &cli,
            &[
                "define-profile",
                "--store",
                store.to_str().unwrap(),
                "--width",
                "200",
                "--height",
                "300",
            ],
        );
        store
    };

    let rolled_store = store_of("rolled");
    let (mut tools, staged) = member_of(&rolled_store, "0.6");
    assert!(
        staged
            .args
            .windows(2)
            .any(|w| w[0] == "--roll" && w[1] == "0.6"),
        "{staged:?}"
    );
    let created =
        command::commit(&cli, &tools.begin_commit(&rolled_store).unwrap(), &stats).unwrap();
    assert!(!created.replayed);

    let plain_store = store_of("plain");
    let (mut tools, _) = member_of(&plain_store, "");
    command::commit(&cli, &tools.begin_commit(&plain_store).unwrap(), &stats).unwrap();

    // The unrolled section frame, by hand: width 200 along X_m = +Y, height 300 along
    // Y_m = (-1,0,1)/sqrt(2), so the box reaches 100 in Y and 150/sqrt(2) outside the axis.
    let plain = Scene::from_snapshot(
        &source::load(&cli, &plain_store, &stats).unwrap(),
        Representation::Full,
    )
    .unwrap();
    let half = 150.0 / 2.0_f64.sqrt();
    for (got, want) in plain
        .min
        .iter()
        .zip([-2000.0 - half, -100.0, -2000.0 - half])
    {
        assert!((f64::from(*got) - want).abs() < 1.0, "min {got} != {want}");
    }
    for (got, want) in plain.max.iter().zip([2000.0 + half, 100.0, 2000.0 + half]) {
        assert!((f64::from(*got) - want).abs() < 1.0, "max {got} != {want}");
    }

    let Some(gpu) = gpu() else {
        let _ = std::fs::remove_dir_all(&base);
        return;
    };
    let rolled = Scene::from_snapshot(
        &source::load(&cli, &rolled_store, &stats).unwrap(),
        Representation::Full,
    )
    .unwrap();
    let plain_pixels = render_pixels(&gpu, &plain);
    let rolled_pixels = render_pixels(&gpu, &rolled);
    let lit = |pixels: &[u8]| {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| is_lit(**p))
            .count()
    };
    assert!(lit(&plain_pixels) > 200, "plain beam must be visible");
    assert!(lit(&rolled_pixels) > 200, "rolled beam must be visible");
    assert_ne!(
        plain_pixels, rolled_pixels,
        "the roll must move the silhouette"
    );
    let _ = std::fs::remove_dir_all(&base);
}
