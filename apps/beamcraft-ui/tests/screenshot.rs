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
    // The file is a real PNG of the reported size with a visible share of lit pixels.
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "PNG signature");
    let image = image::load_from_memory(&bytes).unwrap();
    assert_eq!((image.width(), image.height()), (1280, 800));
    let lit = image
        .to_rgba8()
        .pixels()
        .filter(|p| !is_background(p.0))
        .count();
    assert!(lit > 200, "lit pixels {lit}");
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
