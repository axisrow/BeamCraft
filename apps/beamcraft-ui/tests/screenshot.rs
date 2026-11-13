// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0.

//! `screenshot-view` end to end: the CLI command finds the `beamcraft-ui` binary next to
//! itself, renders the store's published head through the native viewport renderer offscreen,
//! and writes a PNG. Requires a native/software adapter and both binaries next to each other;
//! missing prerequisites fail acceptance, they never skip.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support;
use image::GenericImageView;
use std::path::Path;
use std::process::Command;

/// The background bands of the renderer's clear colour, as the pixel tests use them: the dark
/// clear colour quantized, with the same one-step tolerance the other pixel tests use (issue
/// 562 item 36 moved it from rgb(5, 8, 13) to rgb(23, 33, 43)).
fn is_background([r, g, b, a]: [u8; 4]) -> bool {
    let [dr, dg, db] = [
        beamcraft_ui::render::DARK_BACKGROUND.r,
        beamcraft_ui::render::DARK_BACKGROUND.g,
        beamcraft_ui::render::DARK_BACKGROUND.b,
    ]
    .map(|c| (c * 255.0).round() as u8);
    let near = |channel: u8, expected: u8| {
        i16::from(channel) - i16::from(expected) <= 1
            && i16::from(expected) - i16::from(channel) <= 1
    };
    near(r, dr) && near(g, dg) && near(b, db) && a == 255
}

/// Runs the CLI next to this package's binary; fails the test on a nonzero exit.
fn run_expect_success(cli: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new(cli).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    output.stdout
}

/// Runs the CLI expecting the typed rejection with `code`, and returns its exit status.
fn run_expect_rejection(cli: &Path, args: &[&str], code: &str) -> i32 {
    let output = Command::new(cli).args(args).output().unwrap();
    let body = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(output.status.code(), Some(2), "{args:?}: {body:?}");
    assert_eq!(
        beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT)
            .unwrap()
            .get("code")
            .unwrap()
            .string()
            .unwrap(),
        code,
        "{args:?}: {body:?}"
    );
    output.status.code().unwrap()
}

/// A colour-sets file body holding the sets given: each one rule, `type eq plate`, in the
/// set's colour, the same shape `colour_sets::Sets::decode` reads.
fn sets_file(sets: &[(&str, [u8; 3])]) -> Vec<u8> {
    use beamcraft_json::Json;
    let plate_rule = Json::object([
        ("field", Json::text("type")),
        ("op", Json::text("eq")),
        ("value", Json::text("plate")),
    ]);
    Json::object([
        ("version", Json::uint(1)),
        (
            "sets",
            Json::Array(
                sets.iter()
                    .map(|(name, colour)| {
                        Json::object([
                            ("name", Json::text(*name)),
                            (
                                "rules",
                                Json::Array(vec![Json::object([
                                    ("rule", plate_rule.clone()),
                                    (
                                        "colour",
                                        Json::Array(
                                            colour
                                                .iter()
                                                .map(|c| Json::uint(u64::from(*c)))
                                                .collect(),
                                        ),
                                    ),
                                    ("transparency", Json::Number("0.0".into())),
                                ])]),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
    .encode(&beamcraft_json::Limits::DEFAULT)
    .unwrap()
}

#[test]
fn screenshot_view_renders_a_received_ifc_with_a_see_through_hole() {
    // The display issue: BeamCraft could write an IFC but nothing could render it back.
    // `screenshot-view --source` reads the committed export path end to end: the CLI reads
    // the store, writes the IFC, reads its placed bodies and renders them through the same
    // viewport renderer, and no store is opened by the source branch.
    let Some(cli) = support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))) else {
        panic!("the beamcraft CLI is required next to the beamcraft-ui binary");
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("screenshot-tests")
        .join(format!("ifc-{}", support::boot_nonce()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = root.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let store = store.to_str().unwrap();
    run_expect_success(
        &cli,
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "120",
            "--length",
            "80",
            "--thickness",
            "10",
            "--name",
            "PL-A",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000a001",
            "--op-id",
            "019a7bf0-0000-7000-8000-00000000c001",
        ],
    );
    run_expect_success(
        &cli,
        &[
            "add-hole",
            "--store",
            store,
            "--id",
            "019a7bf0-0000-7000-8000-00000000a001",
            "--hole-round",
            "60,40,18",
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000c002",
        ],
    );
    let source = root.join("repro.ifc");
    run_expect_success(
        &cli,
        &[
            "export-ifc",
            "--store",
            store,
            "--out",
            source.to_str().unwrap(),
        ],
    );
    let out = root.join("ifc.png");
    let stdout = run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--source",
            source.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--yaw",
            "0",
            "--pitch",
            "1.52",
            "--projection",
            "ortho",
            "--background",
            "light",
        ],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(
        reply.get("outcome").unwrap().string().unwrap(),
        "screenshot_saved"
    );
    assert_eq!(reply.get("elements").unwrap().unsigned().unwrap(), 1);
    assert!(reply.get("lit_pixels").unwrap().unsigned().unwrap() > 200);
    let rgba = image::open(&out).unwrap().to_rgba8();
    assert_eq!((rgba.width(), rgba.height()), (1280, 800));
    // The plate fills the fitted frame; its centre is the 18 mm hole at (60, 40), and the
    // paper shows through it: every pixel within 30 px of the centre is background, while
    // the plate body left and right of the hole is drawn.
    let paper = |p: [u8; 4]| p[0] >= 240 && p[1] >= 240 && p[2] >= 240;
    let (cx, cy) = (640i64, 400i64);
    for dy in -30i64..=30 {
        for dx in -30i64..=30 {
            if dx * dx + dy * dy <= 900 {
                let pixel = rgba.get_pixel((cx + dx) as u32, (cy + dy) as u32).0;
                assert!(
                    paper(pixel),
                    "the hole is not see-through at ({dx},{dy}): {pixel:?}"
                );
            }
        }
    }
    let plate_left = rgba.get_pixel((cx - 150) as u32, cy as u32).0;
    let plate_right = rgba.get_pixel((cx + 150) as u32, cy as u32).0;
    assert!(!paper(plate_left), "{plate_left:?}");
    assert!(!paper(plate_right), "{plate_right:?}");
    // The refusals: a source that is not a STEP file, both a store and a source, and the
    // store-only extras.
    let garbage = root.join("garbage.ifc");
    std::fs::write(&garbage, "not a step file\n").unwrap();
    run_expect_rejection(
        &cli,
        &[
            "screenshot-view",
            "--source",
            garbage.to_str().unwrap(),
            "--out",
            root.join("no.png").to_str().unwrap(),
        ],
        "INVALID_ARGUMENT",
    );
    run_expect_rejection(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--source",
            source.to_str().unwrap(),
            "--out",
            root.join("no.png").to_str().unwrap(),
        ],
        "INVALID_ARGUMENT",
    );
    run_expect_rejection(
        &cli,
        &[
            "screenshot-view",
            "--source",
            source.to_str().unwrap(),
            "--out",
            root.join("no.png").to_str().unwrap(),
            "--grids",
            "true",
        ],
        "INVALID_ARGUMENT",
    );
    assert!(!root.join("no.png").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn screenshot_view_renders_the_head_through_the_ui_binary() {
    let Some(cli) = support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))) else {
        panic!("the beamcraft CLI is required next to the beamcraft-ui binary");
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("screenshot-tests")
        .join(format!("store-{}", support::boot_nonce()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = root.to_str().unwrap();
    run_expect_success(
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
        ],
    );
    // A member on the plate, so the render shows both catalogue hues (issue 535).
    run_expect_success(
        &cli,
        &[
            "define-profile",
            "--store",
            store,
            "--width",
            "200",
            "--height",
            "80",
            "--name",
            "RECT 200x80",
        ],
    );
    run_expect_success(
        &cli,
        &[
            "create-member",
            "--store",
            store,
            "--profile",
            "RECT 200x80",
            "--start",
            "100,250,10",
            "--end",
            "300,250,10",
        ],
    );
    let out = root.join("shot.png");
    let out_str = out.to_str().unwrap();
    let stdout = run_expect_success(
        &cli,
        &["screenshot-view", "--store", store, "--out", out_str],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(
        reply.get("outcome").unwrap().string().unwrap(),
        "screenshot_saved"
    );
    assert_eq!(reply.get("width").unwrap().unsigned().unwrap(), 1280);
    assert_eq!(reply.get("height").unwrap().unsigned().unwrap(), 800);
    assert!(reply.get("lit_pixels").unwrap().unsigned().unwrap() > 200);
    // The file is a real PNG of the reported size with a visible share of lit pixels, and the
    // default catalogue colours ride the screenshot (issue 535): the plate reads yellow and the
    // member reads blue-grey, as the window's freshly opened model does. The tint multiplies the
    // shaded base colour, so the test separates the hues by channel dominance (measured bands:
    // plate 160,146,55 and 64,73,30; member 85,110,132 and 34,55,73; background 5,8,13).
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "PNG signature");
    let image = image::load_from_memory(&bytes).unwrap();
    assert_eq!((image.width(), image.height()), (1280, 800));
    let rgba = image.to_rgba8();
    let lit = rgba.pixels().filter(|p| !is_background(p.0)).count();
    assert!(lit > 200, "lit pixels {lit}");
    let yellow = rgba
        .pixels()
        .filter(|p| p.0[0] > 50 && p.0[0] > p.0[2] + 30)
        .count();
    assert!(yellow > 10_000, "yellow plate pixels {yellow}");
    let member = rgba
        .pixels()
        .filter(|p| p.0[2] > 50 && p.0[2] > p.0[0] + 30)
        .count();
    assert!(member > 2_000, "member pixels {member}");
    // An explicit size is honoured.
    let small = root.join("small.png");
    let stdout = run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            small.to_str().unwrap(),
            "--width",
            "320",
            "--height",
            "240",
        ],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(reply.get("width").unwrap().unsigned().unwrap(), 320);
    assert_eq!(reply.get("height").unwrap().unsigned().unwrap(), 240);
    assert_eq!(
        image::load_from_memory(&std::fs::read(&small).unwrap())
            .unwrap()
            .dimensions(),
        (320, 240)
    );
    // An explicit orbit direction is honoured and still frames the whole scene.
    let side = root.join("side.png");
    let stdout = run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            side.to_str().unwrap(),
            "--yaw",
            "-0.35",
            "--pitch",
            "0.25",
        ],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert!(reply.get("lit_pixels").unwrap().unsigned().unwrap() > 200);
    // The orthographic projection with zoom and pan renders a close-up of the same scene.
    let ortho = root.join("ortho.png");
    let stdout = run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            ortho.to_str().unwrap(),
            "--projection",
            "ortho",
            "--zoom",
            "1.27",
            "--pan",
            "7,-241",
        ],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert!(reply.get("lit_pixels").unwrap().unsigned().unwrap() > 200);
    // Every documented option in one invocation: ten options make twenty argv entries, the
    // exact bound the CLI's KEY VALUE-pairs admission has to cover for a fully specified
    // call, with neutral zoom and pan so the fitted ortho frame still holds the scene.
    let full = root.join("full.png");
    let stdout = run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            full.to_str().unwrap(),
            "--width",
            "640",
            "--height",
            "480",
            "--samples",
            "1",
            "--yaw",
            "-0.35",
            "--pitch",
            "0.25",
            "--projection",
            "ortho",
            "--zoom",
            "0",
            "--pan",
            "0,0",
        ],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(reply.get("width").unwrap().unsigned().unwrap(), 640);
    assert_eq!(reply.get("height").unwrap().unsigned().unwrap(), 480);
    assert!(reply.get("lit_pixels").unwrap().unsigned().unwrap() > 200);
    // An unknown projection value is refused by the ui and relayed as an input refusal.
    run_expect_rejection(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            root.join("badproj.png").to_str().unwrap(),
            "--projection",
            "isometric",
        ],
        "INVALID_ARGUMENT",
    );
    // An out-of-range pitch is refused by the ui and relayed as an input refusal.
    run_expect_rejection(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            root.join("bad.png").to_str().unwrap(),
            "--pitch",
            "2.0",
        ],
        "INVALID_ARGUMENT",
    );
    // An existing output file is refused before the render runs.
    run_expect_rejection(
        &cli,
        &["screenshot-view", "--store", store, "--out", out_str],
        "INVALID_ARGUMENT",
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn screenshot_view_takes_a_colour_set_and_a_light_background() {
    let Some(cli) = support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))) else {
        panic!("the beamcraft CLI is required next to the beamcraft-ui binary");
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("screenshot-tests")
        .join(format!("appearance-{}", support::boot_nonce()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = root.to_str().unwrap();
    // The same store shape as the catalogue test: a plate and a member on it.
    run_expect_success(
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
        ],
    );
    run_expect_success(
        &cli,
        &[
            "define-profile",
            "--store",
            store,
            "--width",
            "200",
            "--height",
            "80",
            "--name",
            "RECT 200x80",
        ],
    );
    run_expect_success(
        &cli,
        &[
            "create-member",
            "--store",
            store,
            "--profile",
            "RECT 200x80",
            "--start",
            "100,250,10",
            "--end",
            "300,250,10",
        ],
    );
    let one = root.join("one-set.json");
    std::fs::write(&one, sets_file(&[("Ink plates", [200, 60, 40])])).unwrap();
    let two = root.join("two-sets.json");
    std::fs::write(
        &two,
        sets_file(&[("First", [200, 60, 40]), ("Second", [10, 20, 30])]),
    )
    .unwrap();

    // A file set plus the light background (issue 537): the plate reads the set's red, the
    // catalogue tints are gone, and the clear colour is the paper value rgb(245,246,248)
    // within the quantization band the pixel tests use.
    let styled = root.join("styled.png");
    let stdout = run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            styled.to_str().unwrap(),
            "--colour-set",
            one.to_str().unwrap(),
            "--background",
            "light",
        ],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(
        reply.get("outcome").unwrap().string().unwrap(),
        "screenshot_saved"
    );
    assert!(reply.get("lit_pixels").unwrap().unsigned().unwrap() > 200);
    let image = image::load_from_memory(&std::fs::read(&styled).unwrap()).unwrap();
    let rgba = image.to_rgba8();
    let paper = rgba
        .pixels()
        .filter(|p| {
            (244..=246).contains(&p.0[0])
                && (245..=247).contains(&p.0[1])
                && (247..=249).contains(&p.0[2])
                && p.0[3] == 255
        })
        .count();
    assert!(paper > 1280 * 800 / 2, "paper background pixels {paper}");
    // The set's red tint reaches the plate: channel dominance over the whole plate area,
    // separated like the catalogue test's yellow (shaded faces included).
    let ink = rgba
        .pixels()
        .filter(|p| {
            u16::from(p.0[0]) > 60
                && u16::from(p.0[0]) > u16::from(p.0[1]) + 40
                && u16::from(p.0[0]) > u16::from(p.0[2]) + 40
        })
        .count();
    assert!(ink > 10_000, "set-coloured plate pixels {ink}");
    // Neither catalogue hue survives: the file set replaced the default overlay.
    let yellow = rgba
        .pixels()
        .filter(|p| u16::from(p.0[1]) > 100 && u16::from(p.0[1]) > u16::from(p.0[2]) + 50)
        .count();
    assert!(yellow < 100, "catalogue yellow pixels left {yellow}");
    // A built-in preset name picks the preset and renders normally.
    let preset = root.join("preset.png");
    run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            preset.to_str().unwrap(),
            "--colour-set",
            "Part classes",
        ],
    );
    // Refusals: an unreadable path, a file with two sets of its own, a bad background value.
    run_expect_rejection(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            root.join("missing.png").to_str().unwrap(),
            "--colour-set",
            root.join("missing.json").to_str().unwrap(),
        ],
        "INVALID_ARGUMENT",
    );
    run_expect_rejection(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            root.join("twosets.png").to_str().unwrap(),
            "--colour-set",
            two.to_str().unwrap(),
        ],
        "INVALID_ARGUMENT",
    );
    run_expect_rejection(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            root.join("medium.png").to_str().unwrap(),
            "--background",
            "medium",
        ],
        "INVALID_ARGUMENT",
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Issue 545: a plate kept degenerate (a self-crossing contour, saved under D-7) no longer
/// makes the whole revision unviewable; the screenshot leaves it out and draws the rest.
#[test]
fn screenshot_view_draws_the_rest_of_a_revision_with_a_degenerate_plate() {
    let Some(cli) = support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))) else {
        panic!("the beamcraft CLI is required next to the beamcraft-ui binary");
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("screenshot-tests")
        .join(format!("degenerate-{}", support::boot_nonce()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = root.to_str().unwrap();
    run_expect_success(
        &cli,
        &[
            "create-plate",
            "--store",
            store,
            "--contour",
            "0,0;200,200;200,0;0,200",
            "--thickness",
            "12",
            "--name",
            "BOWTIE",
        ],
    );
    run_expect_success(
        &cli,
        &[
            "define-profile",
            "--store",
            store,
            "--width",
            "200",
            "--height",
            "80",
            "--name",
            "RECT 200x80",
        ],
    );
    run_expect_success(
        &cli,
        &[
            "create-member",
            "--store",
            store,
            "--profile",
            "RECT 200x80",
            "--start",
            "0,0,0",
            "--end",
            "2000,0,0",
        ],
    );
    let out = root.join("degenerate.png");
    let stdout = run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            out.to_str().unwrap(),
        ],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(
        reply.get("outcome").unwrap().string().unwrap(),
        "screenshot_saved"
    );
    assert_eq!(reply.get("elements").unwrap().unsigned().unwrap(), 1);
    let rgba = image::open(&out).unwrap().to_rgba8();
    let member = rgba
        .pixels()
        .filter(|p| p.0[2] > 50 && p.0[2] > p.0[0] + 30)
        .count();
    assert!(member > 2_000, "member pixels {member}");
}

/// The horizontal and vertical extent, in pixels, of everything that is not the light paper.
fn ink_extent(path: &Path) -> (u32, u32) {
    let rgba = image::open(path).unwrap().to_rgba8();
    let paper = |p: &image::Rgba<u8>| p.0[0] >= 240 && p.0[1] >= 240 && p.0[2] >= 240;
    let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0, u32::MAX, 0);
    for (x, y, p) in rgba.enumerate_pixels() {
        if !paper(p) {
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
    }
    (x1.saturating_sub(x0), y1.saturating_sub(y0))
}

/// Issue 543: the headless screenshot draws the project grids the window draws, with their
/// labels, and fits the camera over model and grids together; `--grids false` keeps the
/// model-only picture.
#[test]
fn screenshot_view_draws_and_fits_the_project_grids() {
    let Some(cli) = support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))) else {
        panic!("the beamcraft CLI is required next to the beamcraft-ui binary");
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("screenshot-tests")
        .join(format!("grids-{}", support::boot_nonce()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = root.to_str().unwrap();
    run_expect_success(
        &cli,
        &[
            "create-grid",
            "--store",
            store,
            "--name",
            "G1",
            "--u-expression",
            "0 3*4000",
            "--v-expression",
            "0 2*6000",
        ],
    );
    run_expect_success(
        &cli,
        &[
            "define-profile",
            "--store",
            store,
            "--width",
            "200",
            "--height",
            "300",
            "--name",
            "R200x300",
        ],
    );
    run_expect_success(
        &cli,
        &[
            "create-member",
            "--store",
            store,
            "--profile",
            "R200x300",
            "--start",
            "0,0,0",
            "--end",
            "8000,0,0",
        ],
    );
    let shot = |name: &str, grids: &str| {
        let out = root.join(name);
        let stdout = run_expect_success(
            &cli,
            &[
                "screenshot-view",
                "--store",
                store,
                "--out",
                out.to_str().unwrap(),
                "--yaw",
                "0",
                "--pitch",
                "1.52",
                "--projection",
                "ortho",
                "--background",
                "light",
                "--grids",
                grids,
            ],
        );
        let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
        (out, reply.get("grid_lines").unwrap().unsigned().unwrap())
    };
    let (with, lines) = shot("grids.png", "true");
    assert_eq!(lines, 7);
    let (without, none) = shot("no-grids.png", "false");
    assert_eq!(none, 0);
    let (gw, gh) = ink_extent(&with);
    let (mw, mh) = ink_extent(&without);
    // With grids the frame holds the whole 16 m x 16 m guide square: ink spans a near-square
    // region on both axes, where the model-only frame is one bar.
    assert!(gw > 300 && gh > 300, "grid ink extent {gw}x{gh}");
    assert!(mw.min(mh) < 100, "model-only ink extent {mw}x{mh}");
    // An unknown value is refused before any render.
    let bad = root.join("bad.png");
    run_expect_rejection(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            bad.to_str().unwrap(),
            "--grids",
            "maybe",
        ],
        "INVALID_ARGUMENT",
    );
    assert!(!bad.exists());
}

#[test]
fn screenshot_view_relays_the_cli_refusal_of_another_layer_mapping() {
    // Issue 555 finding 6: a store pinned to another layer mapping is refused by the CLI's
    // export with UNSUPPORTED_VERSION; the window relays it and `screenshot-view` re-issues
    // the same code and the original message instead of INVALID_ARGUMENT.
    use beamcraft_host::revision::{
        ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7,
    };
    let Some(cli) = support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))) else {
        panic!("the beamcraft CLI is required next to the beamcraft-ui binary");
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("screenshot-tests")
        .join(format!("schema-{}", support::boot_nonce()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = root.to_str().unwrap();
    run_expect_success(
        &cli,
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "100",
            "--length",
            "100",
            "--thickness",
            "10",
        ],
    );
    // Re-commit the head with every schema pin replaced, as a store of another build arrives.
    let opened = run_expect_success(&cli, &["open-revision", "--store", store]);
    let opened = beamcraft_json::decode(&opened, &beamcraft_json::Limits::DEFAULT).unwrap();
    let build = opened
        .get("schema")
        .unwrap()
        .get("build_digest")
        .unwrap()
        .string()
        .unwrap()
        .to_owned();
    let other = "b".repeat(64);
    let revisions = RevisionStore::new(beamcraft_host::storage::BlobStore::new(&root).unwrap());
    let head = revisions.open().unwrap();
    let content = head.manifest().content();
    let mut ids = beamcraft_host::ids::IdSource::system();
    let mut entries = Vec::new();
    for entry in content.entries() {
        let text = String::from_utf8(head.layer_bytes(entry.name()).unwrap())
            .unwrap()
            .replace(&build, &other);
        revisions.blobs().publish(text.as_bytes()).unwrap();
        entries.push(LayerEntry::from_bytes(entry.name(), entry.role(), text.as_bytes()).unwrap());
    }
    let draft = RevisionDraft {
        project: content.project().clone(),
        workspace: content.workspace().clone(),
        revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
        op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
        workspace_generation: content.workspace_generation() + 1,
        schema: content.schema(),
        root_layer: content.root_layer().to_owned(),
        entries,
    };
    revisions
        .commit_with(
            &draft,
            &ExpectedBase::Revision(head.info().revision().clone()),
            None,
            &mut NoFault,
        )
        .unwrap();
    let out = root.join("refused.png");
    let output = Command::new(&cli)
        .args([
            "screenshot-view",
            "--store",
            store,
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let body = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(output.status.code(), Some(2), "{body}");
    let reply = beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(
        reply.get("code").unwrap().string().unwrap(),
        "UNSUPPORTED_VERSION",
        "{body}"
    );
    assert!(
        reply
            .get("message")
            .unwrap()
            .string()
            .unwrap()
            .contains("layer schema digest mismatch"),
        "{body}"
    );
    assert!(!out.exists());
}

#[test]
fn screenshot_view_flags_an_empty_frame_and_names_the_zoom_range() {
    // Issue 607: a frame in which every pixel is the background is flagged in the summary
    // (the PNG is still written, the outcome stays `screenshot_saved`), and the refusal of an
    // out-of-range zoom names the accepted range instead of printing the usage line.
    let Some(cli) = support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))) else {
        panic!("the beamcraft CLI is required next to the beamcraft-ui binary");
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("screenshot-tests")
        .join(format!("empty-{}", support::boot_nonce()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = root.to_str().unwrap();
    run_expect_success(
        &cli,
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "120",
            "--length",
            "80",
            "--thickness",
            "10",
        ],
    );
    // The model panned far off the frame: the empty render is flagged, still saved, exit 0.
    let empty = root.join("empty.png");
    let stdout = run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            empty.to_str().unwrap(),
            "--pan",
            "10000,0",
        ],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(
        reply.get("outcome").unwrap().string().unwrap(),
        "screenshot_saved"
    );
    assert_eq!(reply.get("lit_pixels").unwrap().unsigned().unwrap(), 0);
    assert!(
        reply
            .get("warning")
            .unwrap()
            .string()
            .unwrap()
            .contains("none of the model"),
        "{stdout:?}"
    );
    assert!(
        std::fs::read(&empty)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
    // A frame that holds the model carries no warning.
    let filled = root.join("filled.png");
    let stdout = run_expect_success(
        &cli,
        &[
            "screenshot-view",
            "--store",
            store,
            "--out",
            filled.to_str().unwrap(),
        ],
    );
    let reply = beamcraft_json::decode(&stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert!(reply.get("lit_pixels").unwrap().unsigned().unwrap() > 200);
    assert!(reply.get("warning").is_err(), "{stdout:?}");
    // The refusal of an out-of-range zoom names the range, not the whole usage line.
    let refused = root.join("refused.png");
    let output = Command::new(&cli)
        .args([
            "screenshot-view",
            "--store",
            store,
            "--out",
            refused.to_str().unwrap(),
            "--zoom",
            "11",
        ])
        .output()
        .unwrap();
    let body = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(output.status.code(), Some(2), "{body}");
    let reply = beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(
        reply.get("code").unwrap().string().unwrap(),
        "INVALID_ARGUMENT",
        "{body}"
    );
    let message = reply.get("message").unwrap().string().unwrap();
    assert!(message.contains("-10 to 10"), "{message}");
    assert!(!message.contains("usage:"), "{message}");
    assert!(!refused.exists());
}
