// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.3 regular grid edits through the actual binary and immutable revisions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use support::text;
const GRID: &str = "01990000-0000-7000-8000-000000002001";
const AXES: &str = "01990000-0000-7000-8000-000000002010,01990000-0000-7000-8000-000000002011,01990000-0000-7000-8000-000000002012";
const NEW: &str = "01990000-0000-7000-8000-000000002013";
fn list<'a>(j: &'a Json, key: &str) -> &'a [Json] {
    j.get(key).unwrap().array().unwrap()
}
fn ok(f: &support::StoreFixture, c: &str, a: &[&str]) -> Json {
    let (status, j) = f.run(c, a);
    assert_eq!(status, 0, "{c}: {j:?}");
    j
}
fn grid(f: &support::StoreFixture) -> Json {
    list(&ok(f, "list-grids", &[]), "grids")[0].clone()
}
fn lines(g: &Json) -> &[Json] {
    list(g.get("u").unwrap(), "lines")
}
fn create(f: &support::StoreFixture) {
    ok(
        f,
        "create-grid",
        &[
            "--name",
            "Main",
            "--new-id",
            GRID,
            "--u-expression",
            "0 2*6000",
            "--u-axis-ids",
            AXES,
        ],
    );
}
fn rejected(f: &support::StoreFixture, a: &[&str], code: &str) {
    let before = ok(f, "open-revision", &[]);
    let (status, j) = f.run("update-grid", a);
    assert_eq!(status, 2, "{j:?}");
    assert_eq!(text(&j, "code"), code);
    assert_eq!(before, ok(f, "open-revision", &[]));
}
#[test]
fn one_line_drag_rewrites_the_expression_and_preserves_all_kept_axis_ids() {
    let f = support::StoreFixture::new("t01-regular-edit");
    create(&f);
    let before = grid(&f);
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            GRID,
            "--u-expression",
            "0 6500 5500",
            "--u-labels",
            "1 2 3",
        ],
    );
    let after = grid(&f);
    assert_eq!(text(after.get("u").unwrap(), "expression"), "0 6500 5500");
    for (old, new) in lines(&before).iter().zip(lines(&after)) {
        assert_eq!(text(old, "axis_id"), text(new, "axis_id"));
    }
    assert_eq!(
        lines(&after)[1].get("start_mm").unwrap().array().unwrap()[0]
            .float()
            .unwrap(),
        6500.0
    );
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            GRID,
            "--u-expression",
            "0 6500 5500 6000",
            "--u-labels",
            "1 2 3 4",
            "--u-new-axis-ids",
            NEW,
        ],
    );
    let inserted = grid(&f);
    assert_eq!(text(&lines(&inserted)[3], "axis_id"), NEW);
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            GRID,
            "--u-expression",
            "0 12000 6000",
            "--u-labels",
            "1 3 4",
        ],
    );
    let removed = grid(&f);
    assert_eq!(
        text(&lines(&removed)[1], "axis_id"),
        text(&lines(&inserted)[2], "axis_id")
    );
    assert_eq!(text(&lines(&removed)[2], "axis_id"), NEW);
}
#[test]
fn name_frame_and_span_update_together_without_moving_parts() {
    let f = support::StoreFixture::new("t01-regular-frame");
    create(&f);
    ok(
        &f,
        "create-plate",
        &["--width", "1000", "--length", "500", "--thickness", "10"],
    );
    let before = ok(&f, "inspect-elements", &[]);
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            GRID,
            "--name",
            "Renamed",
            "--origin",
            "1000,2000",
            "--rotation",
            "90",
            "--u-span",
            "-1000,8000",
        ],
    );
    let after = ok(&f, "inspect-elements", &[]);
    assert_eq!(before.get("plates"), after.get("plates"));
    let g = grid(&f);
    assert_eq!(text(&g, "name"), "Renamed");
    let start = lines(&g)[0].get("start_mm").unwrap().array().unwrap();
    assert!((start[0].float().unwrap() - 2000.0).abs() < 1e-8);
    assert!((start[1].float().unwrap() - 2000.0).abs() < 1e-8);
}
#[test]
fn malformed_order_labels_modes_and_identities_refuse_without_publication() {
    let f = support::StoreFixture::new("t01-regular-errors");
    create(&f);
    rejected(
        &f,
        &["--id", GRID, "--u-expression", "0 NaN", "--u-labels", "1 2"],
        "NON_FINITE_NUMBER",
    );
    rejected(
        &f,
        &[
            "--id",
            GRID,
            "--u-expression",
            "0 12001 -1",
            "--u-labels",
            "1 2 3",
        ],
        "INVALID_ARGUMENT",
    );
    rejected(
        &f,
        &[
            "--id",
            GRID,
            "--u-expression",
            "0 0.5 12000",
            "--u-labels",
            "1 2 3",
        ],
        "INVALID_ARGUMENT",
    );
    rejected(
        &f,
        &["--id", GRID, "--u-expression", "0 6000", "--u-labels", ","],
        "INVALID_ARGUMENT",
    );
    rejected(
        &f,
        &[
            "--id",
            GRID,
            "--u-expression",
            "0 6000",
            "--u-labels",
            "1 4",
            "--u-new-axis-ids",
            "01990000-0000-7000-8000-000000002010",
        ],
        "DUPLICATE_ID",
    );
    rejected(
        &f,
        &["--id", GRID, "--v-expression", "0", "--v-labels", "A"],
        "UNSUPPORTED_PROPERTY",
    );
    rejected(&f, &["--id", GRID, "--u-span", "0,0.5"], "INVALID_ARGUMENT");
}
#[test]
fn exact_update_retry_is_independent_of_later_deletion() {
    let f = support::StoreFixture::new("t01-update-retry");
    create(&f);
    let args = [
        "--id",
        GRID,
        "--u-expression",
        "0 6500 5500",
        "--u-labels",
        "1 2 3",
        "--u-new-axis-ids",
        "",
        "--op-id",
        "01990000-0000-7000-8000-000000002100",
    ];
    ok(&f, "update-grid", &args);
    let before = ok(&f, "open-revision", &[]);
    assert_eq!(
        text(&ok(&f, "update-grid", &args), "outcome"),
        "grid_update_replayed"
    );
    assert_eq!(before, ok(&f, "open-revision", &[]));
    ok(&f, "delete-element", &["--id", GRID]);
    assert_eq!(
        text(&ok(&f, "update-grid", &args), "outcome"),
        "grid_update_replayed"
    );
}
#[test]
fn a_regular_family_edit_and_frame_move_are_one_undo_unit() {
    let f = support::StoreFixture::new("t01-update-undo");
    create(&f);
    let original = ok(&f, "list-grids", &[]);
    let draft = f.root.join("draft").to_str().unwrap().to_owned();
    ok(&f, "create-workspace", &["--workspace-dir", &draft]);
    let run = |c: &str, a: &[&str]| {
        let (status, j) = support::StoreFixture::run_in(&draft, c, a);
        assert_eq!(status, 0, "{c}: {j:?}");
        j
    };
    let updated = run(
        "update-grid",
        &[
            "--id",
            GRID,
            "--origin",
            "1000,2000",
            "--u-expression",
            "0 6500 5500",
            "--u-labels",
            "1 2 3",
        ],
    );
    let history = |c: &str| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([c, "--workspace-dir", &draft])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    };
    history("undo-workspace");
    assert_eq!(original.get("grids"), run("list-grids", &[]).get("grids"));
    history("redo-workspace");
    assert_eq!(updated.get("grids"), run("list-grids", &[]).get("grids"));
}

#[test]
fn fitting_repairs_a_disconnected_grid_and_replays_without_growing_it_again() {
    let f = support::StoreFixture::new("t01-grid-fit");
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "Fit",
            "--new-id",
            GRID,
            "--u-expression",
            "0 2*6000",
            "--v-expression",
            "0 2*5000",
        ],
    );
    // Reproduce an older edit which enlarged both families but retained their short spans.
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            GRID,
            "--u-expression",
            "0 5*6000",
            "--u-labels",
            "1",
            "--v-expression",
            "0 9*5000",
            "--v-labels",
            "A",
        ],
    );
    let disconnected = grid(&f);
    let fit = [
        "--id",
        GRID,
        "--fit-spans",
        "true",
        "--op-id",
        "01990000-0000-7000-8000-000000002200",
    ];
    ok(&f, "update-grid", &fit);
    let fitted = grid(&f);
    assert_eq!(list(fitted.get("u").unwrap(), "lines").len(), 6);
    assert_eq!(list(fitted.get("v").unwrap(), "lines").len(), 10);
    for (family, end, dimension) in [("u", 47000.0, 1), ("v", 32000.0, 0)] {
        let before = list(disconnected.get(family).unwrap(), "lines");
        for (old, new) in before
            .iter()
            .zip(list(fitted.get(family).unwrap(), "lines"))
        {
            assert_eq!(old.get("axis_id"), new.get("axis_id"));
            assert_eq!(old.get("label"), new.get("label"));
            assert_eq!(
                new.get("start_mm").unwrap().array().unwrap()[dimension]
                    .float()
                    .unwrap(),
                -2000.0
            );
            assert_eq!(
                new.get("end_mm").unwrap().array().unwrap()[dimension]
                    .float()
                    .unwrap(),
                end
            );
        }
    }
    let head = ok(&f, "open-revision", &[]);
    assert_eq!(
        text(&ok(&f, "update-grid", &fit), "outcome"),
        "grid_update_replayed"
    );
    assert_eq!(head, ok(&f, "open-revision", &[]));
    rejected(
        &f,
        &["--id", GRID, "--fit-spans", "false"],
        "INVALID_ARGUMENT",
    );
    rejected(
        &f,
        &["--id", GRID, "--fit-spans", "yes"],
        "INVALID_ARGUMENT",
    );
}
