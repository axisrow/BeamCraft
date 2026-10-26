// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.2 line editing through the real CLI and atomic revisions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
const GRID: &str = "01990000-0000-7000-8000-000000001001";
const U: &str = "01990000-0000-7000-8000-000000001002";
const V: &str = "01990000-0000-7000-8000-000000001003";
const OP: &str = "01990000-0000-7000-8000-000000001004";
fn text(j: &Json, key: &str) -> String {
    j.get(key).unwrap().string().unwrap().to_owned()
}
fn list<'a>(j: &'a Json, key: &str) -> &'a [Json] {
    j.get(key).unwrap().array().unwrap()
}
fn ok(f: &support::StoreFixture, command: &str, args: &[&str]) -> Json {
    let (status, j) = f.run(command, args);
    assert_eq!(status, 0, "{command}: {j:?}");
    j
}
fn grid(f: &support::StoreFixture) -> Json {
    list(&ok(f, "list-grids", &[]), "grids")[0].clone()
}
fn line<'a>(g: &'a Json, family: &str, index: usize) -> &'a Json {
    &list(g.get(family).unwrap(), "lines")[index]
}
fn rejected(f: &support::StoreFixture, command: &str, args: &[&str], code: &str) {
    let before = ok(f, "open-revision", &[]);
    let (status, j) = f.run(command, args);
    assert_eq!(status, 2, "{command}: {j:?}");
    assert_eq!(text(&j, "code"), code);
    assert_eq!(before, ok(f, "open-revision", &[]));
}
fn create(f: &support::StoreFixture) {
    ok(
        f,
        "create-grid",
        &[
            "--name",
            "Drawn",
            "--new-id",
            GRID,
            "--u-from",
            "1000,2000",
            "--u-to",
            "1000,10000",
            "--u-label",
            "А",
            "--u-axis-id",
            U,
        ],
    );
}
#[test]
fn a_fully_drawn_grid_can_be_created_and_edited_without_replacing_identities() {
    let f = support::StoreFixture::new("t01-drawn");
    create(&f);
    ok(
        &f,
        "draw-grid-line",
        &[
            "--id",
            GRID,
            "--family",
            "v",
            "--new-axis-id",
            V,
            "--label",
            "1/2",
            "--from",
            "0,5000",
            "--to",
            "9000,5000",
        ],
    );
    let before = grid(&f);
    assert_eq!(text(line(&before, "u", 0), "axis_id"), U);
    assert_eq!(text(line(&before, "v", 0), "axis_id"), V);
    ok(
        &f,
        "reshape-grid-line",
        &[
            "--id",
            GRID,
            "--axis-id",
            V,
            "--from",
            "-1000,6000",
            "--to",
            "12000,6000",
        ],
    );
    let shaped = grid(&f);
    assert_eq!(text(line(&shaped, "v", 0), "axis_id"), V);
    assert_eq!(text(line(&shaped, "v", 0), "label"), "1/2");
    assert_eq!(line(&shaped, "u", 0), line(&before, "u", 0));
    ok(
        &f,
        "relabel-grid-line",
        &["--id", GRID, "--axis-id", V, "--label", "Б/1"],
    );
    let renamed = grid(&f);
    assert_eq!(text(line(&renamed, "v", 0), "axis_id"), V);
    assert_eq!(
        line(&renamed, "v", 0).get("start_mm"),
        line(&shaped, "v", 0).get("start_mm")
    );
    ok(&f, "remove-grid-line", &["--id", GRID, "--axis-id", V]);
    let removed = grid(&f);
    assert!(list(removed.get("v").unwrap(), "lines").is_empty());
    assert_eq!(line(&removed, "u", 0), line(&before, "u", 0));
    rejected(
        &f,
        "remove-grid-line",
        &["--id", GRID, "--axis-id", U],
        "INVALID_ARGUMENT",
    );
}
#[test]
fn invalid_edits_and_regular_family_mode_refuse_without_writes() {
    let f = support::StoreFixture::new("t01-edit-refusals");
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "Mixed",
            "--new-id",
            GRID,
            "--u-expression",
            "0",
            "--u-axis-ids",
            U,
        ],
    );
    rejected(
        &f,
        "draw-grid-line",
        &[
            "--id", GRID, "--family", "u", "--label", "X", "--from", "0,0", "--to", "0,1000",
        ],
        "UNSUPPORTED_PROPERTY",
    );
    rejected(
        &f,
        "reshape-grid-line",
        &[
            "--id",
            GRID,
            "--axis-id",
            U,
            "--from",
            "0,0",
            "--to",
            "0,1000",
        ],
        "UNSUPPORTED_PROPERTY",
    );
    rejected(
        &f,
        "draw-grid-line",
        &[
            "--id",
            GRID,
            "--family",
            "v",
            "--new-axis-id",
            U,
            "--label",
            "X",
            "--from",
            "0,0",
            "--to",
            "0,1000",
        ],
        "DUPLICATE_ID",
    );
    rejected(
        &f,
        "draw-grid-line",
        &[
            "--id", GRID, "--family", "v", "--label", "X", "--from", "0,0", "--to", "0,0.5",
        ],
        "INVALID_ARGUMENT",
    );
    rejected(
        &f,
        "draw-grid-line",
        &[
            "--id", GRID, "--family", "v", "--label", "1", "--from", "0,0", "--to", "0,1000",
        ],
        "INVALID_ARGUMENT",
    );
    rejected(
        &f,
        "reshape-grid-line",
        &[
            "--id",
            GRID,
            "--axis-id",
            V,
            "--from",
            "0,0",
            "--to",
            "0,1000",
        ],
        "NOT_FOUND",
    );
}
#[test]
fn line_edits_replay_from_request_evidence_and_world_points_follow_the_frame() {
    let f = support::StoreFixture::new("t01-line-frame");
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "Rotated",
            "--new-id",
            GRID,
            "--origin",
            "1000,2000",
            "--rotation",
            "90",
            "--u-from",
            "1000,2000",
            "--u-to",
            "-7000,2000",
            "--u-label",
            "А",
            "--u-axis-id",
            U,
        ],
    );
    let args = [
        "--id",
        GRID,
        "--family",
        "v",
        "--new-axis-id",
        V,
        "--label",
        "B",
        "--from",
        "1000,3000",
        "--to",
        "-1000,3000",
        "--op-id",
        OP,
    ];
    ok(&f, "draw-grid-line", &args);
    let before = ok(&f, "open-revision", &[]);
    assert_eq!(
        text(&ok(&f, "draw-grid-line", &args), "outcome"),
        "grid_line_drawn_replayed"
    );
    assert_eq!(before, ok(&f, "open-revision", &[]));
    let g = grid(&f);
    let p = line(&g, "v", 0).get("start_mm").unwrap().array().unwrap();
    assert!((p[0].float().unwrap() - 1000.0).abs() < 1e-8);
    assert!((p[1].float().unwrap() - 3000.0).abs() < 1e-8);
    ok(&f, "remove-grid-line", &["--id", GRID, "--axis-id", V]);
    assert_eq!(
        text(&ok(&f, "draw-grid-line", &args), "outcome"),
        "grid_line_drawn_replayed"
    );
    assert!(list(grid(&f).get("v").unwrap(), "lines").is_empty());
}
#[test]
fn a_family_relabel_changes_neither_axis_ids_nor_geometry() {
    let f = support::StoreFixture::new("t01-family-labels");
    ok(
        &f,
        "create-grid",
        &["--name", "Regular", "--u-expression", "0 6000"],
    );
    let before = grid(&f);
    let id = text(&before, "id");
    ok(
        &f,
        "relabel-grid-line",
        &["--id", &id, "--family", "u", "--labels", "А Б"],
    );
    let after = grid(&f);
    for i in 0..2 {
        assert_eq!(
            line(&after, "u", i).get("axis_id"),
            line(&before, "u", i).get("axis_id")
        );
        assert_eq!(
            line(&after, "u", i).get("start_mm"),
            line(&before, "u", i).get("start_mm")
        );
    }
    rejected(
        &f,
        "relabel-grid-line",
        &["--id", &id, "--family", "u", "--labels", "A"],
        "INVALID_ARGUMENT",
    );
}

#[test]
fn drawing_and_reshaping_are_individual_undo_units() {
    let f = support::StoreFixture::new("t01-drawn-undo");
    create(&f);
    let draft = f.root.join("draft").to_str().unwrap().to_owned();
    ok(&f, "create-workspace", &["--workspace-dir", &draft]);
    let run = |command: &str, args: &[&str]| {
        let (status, j) = support::StoreFixture::run_in(&draft, command, args);
        assert_eq!(status, 0, "{command}: {j:?}");
        j
    };
    let history = |command: &str| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([command, "--workspace-dir", &draft])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    };
    let drawn = run(
        "draw-grid-line",
        &[
            "--id",
            GRID,
            "--family",
            "v",
            "--new-axis-id",
            V,
            "--label",
            "B",
            "--from",
            "0,5000",
            "--to",
            "9000,5000",
        ],
    );
    history("undo-workspace");
    assert!(
        list(
            list(&run("list-grids", &[]), "grids")[0].get("v").unwrap(),
            "lines"
        )
        .is_empty()
    );
    history("redo-workspace");
    assert_eq!(drawn.get("grids"), run("list-grids", &[]).get("grids"));
    run(
        "reshape-grid-line",
        &[
            "--id",
            GRID,
            "--axis-id",
            V,
            "--from",
            "0,7000",
            "--to",
            "9000,7000",
        ],
    );
    history("undo-workspace");
    assert_eq!(drawn.get("grids"), run("list-grids", &[]).get("grids"));
}
