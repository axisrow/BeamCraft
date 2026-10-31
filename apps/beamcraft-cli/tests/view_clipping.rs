// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! End-to-end tests of the headless V15 cut plane: the real binary, the conservative mask,
//! and the evidence that a cut never changes the model or its measurements.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, decode};
use std::process::Command;

fn run(args: &[&str]) -> (Json, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .output()
        .expect("run beamcraft");
    let json =
        decode(&out.stdout, &beamcraft_json::Limits::DEFAULT).expect("stdout is bounded JSON");
    (json, out.status.code().expect("exit code"))
}

fn text<'a>(j: &'a Json, key: &str) -> &'a str {
    j.get(key).unwrap().string().unwrap()
}

fn count(j: &Json, key: &str) -> u64 {
    j.get(key).unwrap().unsigned().unwrap()
}

fn rejected(args: &[&str], f04: &str, wire: &str) {
    let (json, exit) = run(args);
    assert_eq!(exit, 2, "{args:?}");
    assert_eq!(text(&json, "outcome"), "rejected");
    assert_eq!(text(&json, "code"), f04, "{args:?}");
    let diagnostics = json.get("diagnostics").unwrap().array().unwrap();
    assert_eq!(text(diagnostics.first().unwrap(), "code"), wire, "{args:?}");
}

#[test]
fn clip_view_masks_conservatively_and_leaves_the_model_identical() {
    let (cut, exit) = run(&["clip-view", "--plates", "6", "--cut", "68,0,0,1,0,0"]);
    assert_eq!(exit, 0);
    assert_eq!(text(&cut, "outcome"), "view_clip");
    assert!(cut.get("model_unchanged").unwrap().boolean().unwrap());
    assert!(!cut.get("work_area_applied").unwrap().boolean().unwrap());
    assert_eq!(count(&cut, "element_count"), 6);
    assert_eq!(count(&cut, "plane_count"), 1);
    // The synthetic row starts every plate at x=0 with widths from 65 mm up: the 68 mm plane
    // hides only the 65 mm body and keeps every partially intersecting one with its mesh.
    assert_eq!(count(&cut, "visible_count"), 5);
    assert_eq!(count(&cut, "clipped_count"), 1);
    let planes = cut.get("effective_planes").unwrap().array().unwrap();
    let first = planes.first().unwrap();
    let normal: Vec<f64> = first
        .get("normal")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|n| n.float().unwrap())
        .collect();
    assert_eq!(normal, vec![1.0, 0.0, 0.0]);
    // The token equals the uncut snapshot of the same pin: a cut is not revision data.
    let (plain, _) = run(&["snapshot-view", "--plates", "6"]);
    assert_eq!(text(&cut, "snapshot_token"), text(&plain, "snapshot_token"));

    // A work-area box that holds the whole scene hides nothing but still contributes its
    // six half-spaces; one right of the scene hides everything.
    let (inside, exit) = run(&[
        "clip-view",
        "--plates",
        "6",
        "--area",
        "-1000,-1000,-1000,1000,1000,1000",
    ]);
    assert_eq!(exit, 0);
    assert_eq!(count(&inside, "plane_count"), 6);
    assert_eq!(count(&inside, "visible_count"), 6);
    assert!(inside.get("work_area_applied").unwrap().boolean().unwrap());
    let (outside, exit) = run(&[
        "clip-view",
        "--plates",
        "6",
        "--area",
        "100000,100000,100000,100001,100001,100001",
    ]);
    assert_eq!(exit, 0);
    assert_eq!(count(&outside, "visible_count"), 0);
    assert_eq!(count(&outside, "clipped_count"), 6);
    // A plane and a box combine within the budget.
    let (both, exit) = run(&[
        "clip-view",
        "--plates",
        "6",
        "--cut",
        "68,0,0,1,0,0;0,0,5000,0,0,-1",
        "--area",
        "-1000,-1000,-1000,1000,1000,1000",
    ]);
    assert_eq!(exit, 0);
    assert_eq!(count(&both, "plane_count"), 8);
    assert!(count(&both, "visible_count") <= count(&cut, "visible_count"));
}

#[test]
fn clip_view_rejections_carry_the_f04_and_view_codes() {
    rejected(
        &["clip-view", "--plates", "6"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["clip-view", "--plates", "6", "--cut", "1,2,3,0,0"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["clip-view", "--plates", "6", "--cut", "0,0,0,0,0,0"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["clip-view", "--plates", "6", "--cut", "0,0,0,x,0,0"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["clip-view", "--plates", "6", "--area", "0,0,0,0,0"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["clip-view", "--plates", "6", "--area", "1,1,1,0,0,0"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &[
            "clip-view",
            "--plates",
            "6",
            "--cut",
            "0,0,0,1,0,0;1,0,0,1,0,0;2,0,0,1,0,0;3,0,0,1,0,0;4,0,0,1,0,0;5,0,0,1,0,0;6,0,0,1,0,0;7,0,0,1,0,0;8,0,0,1,0,0",
        ],
        "LIMIT_EXCEEDED",
        "VIEW_ELEMENT_LIMIT",
    );
    rejected(
        &["clip-view", "--cut", "0,0,0,1,0,0"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &[
            "clip-view",
            "--plates",
            "6",
            "--cut",
            "0,0,0,1,0,0",
            "--bogus",
            "1",
        ],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
}
