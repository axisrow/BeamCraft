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

/// The background bands of the renderer's clear colour, as the pixel tests use them.
fn is_background([r, g, b, a]: [u8; 4]) -> bool {
    (4..=6).contains(&r) && (7..=9).contains(&g) && (12..=14).contains(&b) && a == 255
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
