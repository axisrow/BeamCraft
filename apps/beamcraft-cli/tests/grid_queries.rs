// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! T01.4 grid query and placement regression scenarios.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
const G: &str = "01990000-0000-7000-8000-000000003001";
const P: &str = "01990000-0000-7000-8000-000000003002";
const OP: &str = "01990000-0000-7000-8000-000000003003";
fn ok(f: &support::StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().into()
}
fn list<'a>(j: &'a Json, k: &str) -> &'a [Json] {
    j.get(k).unwrap().array().unwrap()
}
fn create(f: &support::StoreFixture) {
    ok(
        f,
        "create-grid",
        &[
            "--name",
            "G",
            "--new-id",
            G,
            "--u-expression",
            "0 2*6000",
            "--v-expression",
            "0 7500 4500",
        ],
    );
}
fn reject(f: &support::StoreFixture, c: &str, a: &[&str]) {
    let before = ok(f, "open-revision", &[]);
    let (code, j) = f.run(c, a);
    assert_eq!(code, 2, "{j:?}");
    assert_eq!(before, ok(f, "open-revision", &[]));
}
#[test]
fn read_queries_and_snaps_follow_contract_without_publication() {
    let f = support::StoreFixture::new("t01-queries");
    create(&f);
    let before = ok(&f, "open-revision", &[]);
    let p = ok(
        &f,
        "resolve-grid-point",
        &["--grid", "G", "--labels", "B,3"],
    );
    assert_eq!(
        list(&p, "point_world_mm")
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [12000., 7500.]
    );
    let plane = ok(&f, "resolve-grid-plane", &["--grid", G, "--label", "3"]);
    assert_eq!(list(&plane, "normal_world")[0].float().unwrap().abs(), 1.);
    let s = ok(
        &f,
        "snap",
        &[
            "--point",
            "12003,7502,3000",
            "--radius",
            "10",
            "--kinds",
            "grid_intersection,grid_end,grid_line",
        ],
    );
    let hits = list(&s, "candidates");
    assert_eq!(hits.len(), 3);
    assert_eq!(text(&hits[0], "kind"), "grid_line");
    assert_eq!(text(&hits[2], "kind"), "grid_intersection");
    assert_eq!(list(&hits[2], "point_world_mm")[2].float().unwrap(), 3000.);
    let empty = ok(
        &f,
        "snap",
        &[
            "--point",
            "12003,7502,3000",
            "--radius",
            "10",
            "--kinds",
            "grid_line",
            "--grids",
            "",
        ],
    );
    assert!(list(&empty, "candidates").is_empty());
    assert_eq!(before, ok(&f, "open-revision", &[]));
    reject(&f, "resolve-grid-point", &["--grid", G, "--labels", "1,2"]);
    reject(&f, "resolve-grid-point", &["--grid", G, "--labels", "?,B"]);
}
#[test]
fn plate_grid_placement_replays_after_grid_moves_and_deletion() {
    let f = support::StoreFixture::new("t01-place");
    create(&f);
    let a = [
        "--width",
        "100",
        "--length",
        "200",
        "--thickness",
        "10",
        "--origin-at-grid",
        "G:B,3",
        "--z",
        "3000",
        "--new-id",
        P,
        "--op-id",
        OP,
    ];
    ok(&f, "create-plate", &a);
    let before = ok(&f, "inspect-elements", &[]);
    let plates = list(&before, "plates").to_vec();
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            G,
            "--origin",
            "500,600",
            "--u-expression",
            "0 6500 5500",
            "--u-labels",
            "1 2 3",
        ],
    );
    let replay = ok(&f, "create-plate", &a);
    assert!(text(&replay, "outcome").contains("replayed"));
    assert_eq!(list(&ok(&f, "inspect-elements", &[]), "plates"), plates);
    ok(&f, "delete-element", &["--id", G]);
    ok(&f, "create-plate", &a);
}
#[test]
fn placement_height_errors_and_level_offsets_are_explicit() {
    let f = support::StoreFixture::new("t01-height");
    create(&f);
    reject(
        &f,
        "create-plate",
        &[
            "--width",
            "10",
            "--length",
            "20",
            "--thickness",
            "1",
            "--origin-at-grid",
            "G:B,3",
        ],
    );
    reject(
        &f,
        "create-plate",
        &[
            "--width",
            "10",
            "--length",
            "20",
            "--thickness",
            "1",
            "--origin-at-grid",
            "G:B,3",
            "--z",
            "10",
            "--translation",
            "1,2,3",
        ],
    );
    let level = ok(&f, "create-level", &["--name", "L", "--elevation", "3000"]);
    let id = text(&level, "element_id");
    let profile = ok(&f, "define-profile", &["--width", "100", "--height", "50"]);
    let digest = text(profile.get("definition").unwrap(), "digest");
    ok(
        &f,
        "create-member",
        &[
            "--start-at-grid",
            "G:B,2",
            "--end-at-grid",
            "G:B,3",
            "--level",
            &id,
            "--offset",
            "500",
            "--profile-digest",
            &digest,
        ],
    );
    let j = ok(&f, "inspect-elements", &[]);
    let member = &list(&j, "members")[0];
    // Independent query of the accepted member's semantic axis.
    let member_id = text(member, "element_id");
    let snaps = ok(
        &f,
        "snap",
        &[
            "--point",
            "6000,7500,3500",
            "--radius",
            "0",
            "--kinds",
            "axis_end",
        ],
    );
    assert!(
        list(&snaps, "candidates")
            .iter()
            .any(|c| text(c, "element_id") == member_id)
    );
}

#[test]
fn a_grid_placed_plate_is_one_workspace_undo_redo_unit() {
    let f = support::StoreFixture::new("t01-placement-undo");
    create(&f);
    let draft = f.root.join("draft");
    let path = draft.to_str().unwrap();
    ok(&f, "create-workspace", &["--workspace-dir", path]);
    let run = |c, a: &[&str]| {
        let (code, j) = support::StoreFixture::run_in(path, c, a);
        assert_eq!(code, 0, "{c}: {j:?}");
        j
    };
    run(
        "create-plate",
        &[
            "--width",
            "100",
            "--length",
            "200",
            "--thickness",
            "10",
            "--origin-at-grid",
            "G:B,3",
            "--z",
            "1000",
        ],
    );
    let created = run("inspect-elements", &[]);
    let step = |c| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([c, "--workspace-dir", path])
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
    };
    step("undo-workspace");
    assert!(list(&run("inspect-elements", &[]), "plates").is_empty());
    step("redo-workspace");
    assert_eq!(
        list(&run("inspect-elements", &[]), "plates"),
        list(&created, "plates")
    );
}
