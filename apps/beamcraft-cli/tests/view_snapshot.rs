// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! End-to-end tests of the headless V02 commands: the real binary, the real kernel, the engine API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_engine::synthetic::plate_scene;
use beamcraft_engine::{Limits, ViewPin, ViewSnapshot};
use beamcraft_json::{Json, decode};
use beamcraft_model::digest::Sha256;
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
fn view_snapshot_reports_the_engine_result_for_a_synthetic_scene() {
    let (json, exit) = run(&[
        "snapshot-view",
        "--plates",
        "20",
        "--hide",
        "support",
        "--stand-in-simplified",
        "true",
        "--select",
        "01990000-0000-7000-8000-000000000001,01990000-0000-7000-8000-000000000004",
    ]);
    assert_eq!(exit, 0);
    assert_eq!(text(&json, "outcome"), "view_snapshot");
    assert!(json.get("synthetic").unwrap().boolean().unwrap());
    let stand_ins = json.get("stand_ins").unwrap();
    assert!(stand_ins.get("support_kind").unwrap().boolean().unwrap());
    assert!(stand_ins.get("simplified").unwrap().boolean().unwrap());
    assert_eq!(count(&json, "element_count"), 20);
    assert_eq!(count(&json, "visible_count"), 16);
    assert_eq!(count(&json, "hidden_count"), 4);
    assert_eq!(count(&json, "triangle_count"), 16 * 12);
    assert!(json.get("all_exact").unwrap().boolean().unwrap());

    // The token is the engine's own: rebuild the same snapshot in process and compare.
    let pin = ViewPin::new(Sha256::digest(b"beamcraft-synthetic-scene").unwrap(), 1);
    let snapshot =
        ViewSnapshot::build(pin, plate_scene(20, true).unwrap(), &Limits::default()).unwrap();
    assert_eq!(text(&json, "snapshot_token"), snapshot.token().as_str());
    assert_eq!(text(json.get("pin").unwrap(), "generation"), "1");

    let elements = json.get("elements").unwrap().array().unwrap();
    assert_eq!(elements.len(), 16);
    let first = elements.first().unwrap();
    assert_eq!(text(first, "id"), "01990000-0000-7000-8000-000000000000");
    assert_eq!(text(first, "kind"), "plate");
    assert_eq!(count(first, "instance"), 0);
    assert_eq!(count(first, "first_triangle"), 0);
    assert_eq!(count(&elements[1], "first_triangle"), 12);
    assert!(elements.iter().all(|e| text(e, "kind") == "plate"));
    let bounds = json.get("bounds").unwrap();
    assert_eq!(bounds.get("min").unwrap().array().unwrap().len(), 3);
    let eb = snapshot.engineering_bounds().unwrap();
    let eng = json.get("engineering_bounds").unwrap();
    for (key, expected) in [("min", eb.min), ("max", eb.max)] {
        let got: Vec<f64> = eng
            .get(key)
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|n| n.float().unwrap())
            .collect();
        assert_eq!(got, expected.to_vec());
    }
    assert_eq!(eb.min, [0.0, 0.0, 0.0]);
    let selection = json.get("selection").unwrap();
    assert_eq!(selection.get("ids").unwrap().array().unwrap().len(), 2);
    // Element 4 is a hidden support: it has no instance slot.
    assert_eq!(
        selection.get("instances").unwrap().array().unwrap(),
        &[Json::Number("1".into())]
    );
    let required = json.get("required_meshes").unwrap().array().unwrap();
    assert!(!required.is_empty());
    assert_eq!(count(&required[0], "bytes"), 392);
    assert_eq!(count(&required[0], "triangles"), 12);
}

#[test]
fn the_representation_selector_changes_what_is_loaded_not_the_token() {
    let base = [
        "snapshot-view",
        "--plates",
        "20",
        "--stand-in-simplified",
        "true",
    ];
    let (full, _) = run(&base);
    let mut args = base.to_vec();
    args.extend(["--representation", "simplified"]);
    let (simplified, exit) = run(&args);
    assert_eq!(exit, 0);
    assert_eq!(
        text(simplified.get("settings").unwrap(), "representation"),
        "simplified"
    );
    assert!(!simplified.get("all_exact").unwrap().boolean().unwrap());
    assert!(
        simplified
            .get("elements")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .any(|e| !e.get("exact").unwrap().boolean().unwrap())
    );
    assert_eq!(
        text(&full, "snapshot_token"),
        text(&simplified, "snapshot_token"),
        "view settings are not revision data"
    );
    assert_eq!(
        full.get("engineering_bounds"),
        simplified.get("engineering_bounds")
    );
    assert_ne!(
        full.get("required_meshes"),
        simplified.get("required_meshes")
    );
}

#[test]
fn pin_options_are_honoured_and_change_the_token() {
    let revision = "0".repeat(64);
    let (a, _) = run(&[
        "snapshot-view",
        "--plates",
        "2",
        "--generation",
        "9",
        "--revision",
        &revision,
    ]);
    let (b, _) = run(&["snapshot-view", "--plates", "2"]);
    assert_eq!(text(a.get("pin").unwrap(), "generation"), "9");
    assert_eq!(text(a.get("pin").unwrap(), "revision"), revision);
    assert_ne!(text(&a, "snapshot_token"), text(&b, "snapshot_token"));
}

#[test]
fn the_largest_scene_fits_the_control_limits() {
    let (json, exit) = run(&[
        "snapshot-view",
        "--plates",
        "500",
        "--stand-in-simplified",
        "true",
    ]);
    assert_eq!(exit, 0);
    assert_eq!(count(&json, "element_count"), 500);
    assert_eq!(json.get("elements").unwrap().array().unwrap().len(), 500);
}

#[test]
fn view_changes_resends_a_mesh_only_for_a_shape_change() {
    let (moved, exit) = run(&[
        "inspect-view-changes",
        "--plates",
        "8",
        "--edit",
        "move-first",
    ]);
    assert_eq!(exit, 0);
    assert_eq!(text(&moved, "outcome"), "view_changes");
    assert_ne!(text(&moved, "from_token"), text(&moved, "to_token"));
    assert_eq!(text(moved.get("to_pin").unwrap(), "generation"), "2");
    assert_eq!(count(&moved, "change_count"), 1);
    let change = &moved.get("changes").unwrap().array().unwrap()[0];
    assert_eq!(text(change, "change"), "changed");
    assert!(
        change
            .get("fields")
            .unwrap()
            .get("placement")
            .unwrap()
            .boolean()
            .unwrap()
    );
    assert!(
        !change
            .get("fields")
            .unwrap()
            .get("shape")
            .unwrap()
            .boolean()
            .unwrap()
    );
    assert!(moved.get("new_meshes").unwrap().array().unwrap().is_empty());

    let (shape, exit) = run(&[
        "inspect-view-changes",
        "--plates",
        "8",
        "--edit",
        "resize-first,add-one,remove-last,add-one",
    ]);
    assert_eq!(exit, 0);
    let kinds: Vec<&str> = shape
        .get("changes")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|c| text(c, "change"))
        .collect();
    assert_eq!(kinds, ["changed", "added"]);
    assert_eq!(shape.get("new_meshes").unwrap().array().unwrap().len(), 2);

    let (removed, exit) = run(&[
        "inspect-view-changes",
        "--plates",
        "8",
        "--edit",
        "remove-last",
    ]);
    assert_eq!(exit, 0);
    let change = &removed.get("changes").unwrap().array().unwrap()[0];
    assert_eq!(text(change, "change"), "removed");
    assert_eq!(change.get("entry").unwrap(), &Json::Null);
}

#[test]
fn rejections_carry_the_f04_code_and_the_view_code() {
    let unknown_but_valid = "01990000-0000-7000-8000-00000000ffff";
    let stale = format!("vs1-{}", "0".repeat(64));
    rejected(
        &["snapshot-view", "--plates", "0"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["snapshot-view", "--plates", "501"],
        "LIMIT_EXCEEDED",
        "VIEW_ELEMENT_LIMIT",
    );
    rejected(
        &["snapshot-view", "--plates", "x"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["snapshot-view"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["snapshot-view", "--plates"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["snapshot-view", "--plates", "2", "--plates", "3"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["snapshot-view", "--plates", "2", "--bogus", "1"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["snapshot-view", "--plates", "2", "--edit", "move-first"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["snapshot-view", "--plates", "2", "--hide", "weld"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &[
            "snapshot-view",
            "--plates",
            "2",
            "--representation",
            "coarse",
        ],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &[
            "snapshot-view",
            "--plates",
            "2",
            "--stand-in-simplified",
            "maybe",
        ],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["snapshot-view", "--plates", "2", "--select", "not-an-id"],
        "INVALID_ID",
        "VIEW_ID_INVALID",
    );
    rejected(
        &[
            "snapshot-view",
            "--plates",
            "2",
            "--select",
            unknown_but_valid,
        ],
        "NOT_FOUND",
        "VIEW_UNKNOWN_ELEMENT",
    );
    rejected(
        &["snapshot-view", "--plates", "2", "--revision", "zz"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["snapshot-view", "--plates", "2", "--generation", "-1"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["inspect-view-changes", "--plates", "2"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &["inspect-view-changes", "--plates", "2", "--edit", "nope"],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &[
            "inspect-view-changes",
            "--plates",
            "2",
            "--edit",
            "move-first",
            "--since",
            "bad",
        ],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
    rejected(
        &[
            "inspect-view-changes",
            "--plates",
            "2",
            "--edit",
            "move-first",
            "--since",
            &stale,
        ],
        "STALE_WORKSPACE",
        "GEO_STALE_TOKEN",
    );
    rejected(
        &[
            "inspect-view-changes",
            "--plates",
            "1",
            "--edit",
            "remove-last,remove-last",
        ],
        "INVALID_ARGUMENT",
        "VIEW_ARGUMENT_INVALID",
    );
}
