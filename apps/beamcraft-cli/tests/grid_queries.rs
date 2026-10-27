// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! T01.4 grid query and placement regression scenarios.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
const G: &str = "01990000-0000-7000-8000-000000003001";
const P: &str = "01990000-0000-7000-8000-000000003002";
const OP: &str = "01990000-0000-7000-8000-000000003003";
const M: &str = "01990000-0000-7000-8000-000000003005";
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
    // GR-P07: the candidates in contract order - line B, line 3, then the B,3 crossing - with
    // plan distances 2, 3 and sqrt(13), no grid_end, and every hit at the query height.
    let hits = list(&s, "candidates");
    assert_eq!(hits.len(), 3);
    let coords = |h: &Json| -> Vec<f64> {
        list(h, "point_world_mm")
            .iter()
            .map(|v| v.float().unwrap())
            .collect()
    };
    assert_eq!(coords(&hits[0]), [12003., 7500., 3000.]);
    assert_eq!(coords(&hits[1]), [12000., 7502., 3000.]);
    assert_eq!(coords(&hits[2]), [12000., 7500., 3000.]);
    for (hit, kind) in hits
        .iter()
        .zip(["grid_line", "grid_line", "grid_intersection"])
    {
        assert_eq!(text(hit, "kind"), kind);
    }
    let d = |h: &Json| h.get("distance_mm").unwrap().float().unwrap();
    assert_eq!(d(&hits[0]), 2.);
    assert_eq!(d(&hits[1]), 3.);
    assert!((d(&hits[2]) - 13f64.sqrt()).abs() < 1e-9);
    let kept = ok(
        &f,
        "snap",
        &[
            "--point",
            "12003,7502,3000",
            "--radius",
            "10",
            "--kinds",
            "grid_intersection,grid_end,grid_line",
            "--grids",
            G,
        ],
    );
    assert_eq!(list(&kept, "candidates"), hits);
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

#[test]
fn numeric_member_input_can_use_a_grid_start_without_losing_relative_end_semantics() {
    let f = support::StoreFixture::new("t04-grid-input");
    create(&f);
    let profile = ok(&f, "define-profile", &["--width", "20", "--height", "30"]);
    let digest = text(profile.get("definition").unwrap(), "digest");
    let address = format!("{G}:B,3");
    let made = ok(
        &f,
        "create-member",
        &[
            "--start-at-grid",
            &address,
            "--z",
            "3000",
            "--end",
            "@500,100,20",
            "--axis-lock",
            "x",
            "--profile-digest",
            &digest,
        ],
    );
    let points = list(made.get("member").unwrap(), "axis_points_mm");
    for (p, expected) in points
        .iter()
        .zip([[12000., 7500., 3000.], [12500., 7500., 3000.]])
    {
        assert_eq!(
            p.array()
                .unwrap()
                .iter()
                .map(|v| v.float().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
    }
}

/// GR-N08 through the CLI: a drawn pair whose infinite lines cross outside both stored segments
/// has no intersection point and the refusal carries the contract reason.
#[test]
fn a_non_crossing_drawn_pair_refuses_with_the_contract_reason() {
    let f = support::StoreFixture::new("t01-noncrossing");
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "D",
            "--new-id",
            G,
            "--u-expression",
            "0",
            "--u-labels",
            "U",
            "--v-from",
            "20000,0",
            "--v-to",
            "21000,1000",
            "--v-label",
            "V",
        ],
    );
    let before = ok(&f, "open-revision", &[]);
    let (code, j) = f.run("resolve-grid-point", &["--grid", G, "--labels", "U,V"]);
    assert_eq!(code, 2, "{j:?}");
    assert_eq!(text(&j, "code"), "INVALID_ARGUMENT");
    assert_eq!(text(&j, "message"), "GRID_INTERSECTION_UNDEFINED");
    assert_eq!(before, ok(&f, "open-revision", &[]));
}

/// GR-P08 end to end: a member placed on two crossings at a level plus offset keeps its axis,
/// mesh digest and mass report when the grid family is later edited, a retry of the stored
/// operation replays the original coordinates, and a fresh placement resolves anew.
#[test]
fn a_member_placed_on_a_grid_holds_coordinates_when_the_grid_moves() {
    let f = support::StoreFixture::new("t01-member-grid");
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "G",
            "--new-id",
            G,
            "--u-expression",
            "0 3*6000",
            "--v-expression",
            "0 7500 4500",
        ],
    );
    let level = ok(&f, "create-level", &["--name", "L", "--elevation", "3000"]);
    let level_id = text(&level, "element_id");
    let profile = ok(&f, "define-profile", &["--width", "100", "--height", "50"]);
    let digest = text(profile.get("definition").unwrap(), "digest");
    let args = [
        "--start-at-grid",
        "G:B,3",
        "--end-at-grid",
        "G:B,4",
        "--level",
        &level_id,
        "--offset",
        "500",
        "--profile-digest",
        &digest,
        "--new-id",
        M,
        "--op-id",
        OP,
    ];
    ok(&f, "create-member", &args);
    // The contract tolerance for coordinates is 1e-6 mm.
    let close = |got: &[f64], want: &[f64]| got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-6);
    let coords = |j: &Json, id: &str| -> Vec<f64> {
        list(j, "members")
            .iter()
            .find(|m| text(m, "element_id") == id)
            .unwrap()
            .get("axis_points_mm")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .flat_map(|p| p.array().unwrap())
            .map(|v| v.float().unwrap())
            .collect()
    };
    let placed = ok(&f, "inspect-elements", &[]);
    assert!(close(
        &coords(&placed, M),
        &[12000., 7500., 3500., 18000., 7500., 3500.]
    ));
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            G,
            "--u-expression",
            "0 3*6500",
            "--u-labels",
            "1 2 3 4",
        ],
    );
    let after = ok(&f, "inspect-elements", &[]);
    assert_eq!(list(&after, "members"), list(&placed, "members"));
    // The stored operation holds plain coordinates, so the retry replays it even though the
    // named crossings have moved.
    let replay = ok(&f, "create-member", &args);
    assert_eq!(text(&replay, "outcome"), "member_replayed");
    let replayed = ok(&f, "inspect-elements", &[]);
    assert_eq!(list(&replayed, "members"), list(&after, "members"));
    // A fresh operation resolves the same addresses against the edited family.
    ok(
        &f,
        "create-member",
        &[
            "--start-at-grid",
            "G:B,3",
            "--end-at-grid",
            "G:B,4",
            "--level",
            &level_id,
            "--offset",
            "500",
            "--profile-digest",
            &digest,
            "--new-id",
            "01990000-0000-7000-8000-000000003007",
            "--op-id",
            "01990000-0000-7000-8000-000000003008",
        ],
    );
    let moved = ok(&f, "inspect-elements", &[]);
    assert!(close(
        &coords(&moved, "01990000-0000-7000-8000-000000003007"),
        &[13000., 7500., 3500., 19500., 7500., 3500.]
    ));
}
