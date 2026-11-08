// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.8 magnetic grid bindings through the real binary (contract §7.5): a bound member
//! follows the grid frame, its stored truth stays local, unbinding materializes the world
//! axis, and a grid with followers refuses deletion.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;
use beamcraft_json::Json;
use support::text;

const GRID: &str = "01990000-0000-7000-8000-000000000101";
const GRID2: &str = "01990000-0000-7000-8000-000000000109";
const MEMBER: &str = "01990000-0000-7000-8000-000000000102";

fn ok(f: &support::StoreFixture, command: &str, args: &[&str]) -> Json {
    let (code, j) = f.run(command, args);
    assert_eq!(code, 0, "{command}: {j:?}");
    j
}

fn rejected(f: &support::StoreFixture, command: &str, args: &[&str], expected: &str) {
    let (code, j) = f.run(command, args);
    assert_eq!(code, 2, "{command}: {j:?}");
    assert_eq!(text(&j, "code"), expected, "{command}: {j:?}");
}

fn axis_points(j: &Json) -> Vec<[f64; 3]> {
    let member = j.get("members").unwrap().array().unwrap()[0].clone();
    let axis = member.get("axis_points_mm").unwrap().array().unwrap();
    axis.iter()
        .map(|p| {
            let values = p.array().unwrap();
            [
                values[0].float().unwrap(),
                values[1].float().unwrap(),
                values[2].float().unwrap(),
            ]
        })
        .collect()
}

/// A rotated frame's rigid roundtrip carries double-precision residuals far below any
/// manufacturing meaning; the assertions stay exact for unrotated frames.
fn near(actual: [f64; 3], expected: [f64; 3]) {
    for (a, e) in actual.iter().zip(&expected) {
        assert!((a - e).abs() < 1e-6, "expected {e} within 1e-6 mm, got {a}");
    }
}

fn authored_layer(f: &support::StoreFixture, reply: &Json) -> String {
    let layers = reply
        .get("revision")
        .unwrap()
        .get("manifest")
        .unwrap()
        .get("layers")
        .unwrap()
        .array()
        .unwrap();
    let digest = layers
        .iter()
        .find(|l| text(l, "name") == "authored.usda")
        .map(|l| text(l, "digest").to_string())
        .unwrap();
    let path = f
        .root
        .join("store")
        .join("blobs")
        .join(&digest[..2])
        .join(&digest);
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

fn fixture() -> support::StoreFixture {
    let f = support::StoreFixture::new("t018-magnetic");
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "Main",
            "--new-id",
            GRID,
            "--u-expression",
            "0 6000",
            "--v-expression",
            "0 6000",
            "--op-id",
            "01990000-0000-7000-8000-000000000201",
        ],
    );
    let profile = text(
        ok(&f, "define-profile", &["--width", "200", "--height", "300"])
            .get("definition")
            .unwrap(),
        "digest",
    );
    let steel = text(
        ok(&f, "define-material", &["--density", "7850"])
            .get("definition")
            .unwrap(),
        "digest",
    );
    ok(
        &f,
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "1000,1000,0",
            "--end",
            "2000,1000,0",
            "--profile-digest",
            &profile,
            "--material-digest",
            &steel,
            "--name",
            "B1",
            "--new-id",
            MEMBER,
            "--op-id",
            "01990000-0000-7000-8000-000000000202",
        ],
    );
    f
}

/// The stored member truth while bound is the follow pair and no world axis; the world
/// geometry of the derived picture follows the frame exactly (GR-P17).
#[test]
fn a_bound_member_follows_the_frame_and_unbinding_materializes_the_world_axis() {
    let f = fixture();
    let bind_reply = ok(
        &f,
        "bind-grid",
        &[
            "--id",
            MEMBER,
            "--grid",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000203",
        ],
    );
    let bound = authored_layer(&f, &bind_reply);
    assert!(bound.contains("bim:followsGrid"), "{bound}");
    assert!(bound.contains("steel:grid:localAxisMm"), "{bound}");

    // Move the grid frame only: origin +1000,+2000 and rotate the frame by 90 degrees.
    // The local points (1000,1000) and (2000,1000) map to world (0,3000) and (0,4000).
    let move_reply = ok(
        &f,
        "update-grid",
        &[
            "--id",
            GRID,
            "--origin",
            "1000,2000",
            "--rotation",
            "90",
            "--op-id",
            "01990000-0000-7000-8000-000000000204",
        ],
    );
    let still_local = authored_layer(&f, &move_reply);
    assert!(still_local.contains("steel:grid:localAxisMm"));
    let j = ok(&f, "inspect-elements", &["--element", MEMBER]);
    let world = axis_points(&j);
    assert_eq!(world.len(), 2);
    near(world[0], [0.0, 3000.0, 0.0]);
    near(world[1], [0.0, 4000.0, 0.0]);

    // Unbinding materializes the derived world axis as the plain storage truth.
    let unbind_reply = ok(
        &f,
        "unbind-grid",
        &[
            "--id",
            MEMBER,
            "--op-id",
            "01990000-0000-7000-8000-000000000205",
        ],
    );
    let released = authored_layer(&f, &unbind_reply);
    assert!(released.contains("bim:axisPointsMm"));
    // The binding storage is gone: the local axis attr is erased outright, and the follow
    // relationship spec stays as an explicit empty list (the PartOf detach pattern).
    assert!(!released.contains("steel:grid:localAxisMm"));
    let j = ok(&f, "inspect-elements", &["--element", MEMBER]);
    assert_eq!(axis_points(&j), world);
}

/// A grid with followers refuses deletion (GR-N21); releasing the follower unblocks it
/// (GR-N23); releasing an unbound member and binding to an unknown grid are refused.
#[test]
fn a_grid_with_followers_refuses_deletion_until_the_binding_is_released() {
    let f = fixture();
    ok(
        &f,
        "bind-grid",
        &[
            "--id",
            MEMBER,
            "--grid",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000203",
        ],
    );
    rejected(
        &f,
        "delete-element",
        &[
            "--id",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000206",
        ],
        "DEPENDENCY_CONFLICT",
    );
    // The member keeps its derived geometry after the refused delete.
    let j = ok(&f, "inspect-elements", &["--element", MEMBER]);
    assert_eq!(axis_points(&j).len(), 2);
    ok(
        &f,
        "unbind-grid",
        &[
            "--id",
            MEMBER,
            "--op-id",
            "01990000-0000-7000-8000-000000000205",
        ],
    );
    let (code, _) = f.run(
        "delete-element",
        &[
            "--id",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000207",
        ],
    );
    assert_eq!(code, 0);
    let j = ok(&f, "inspect-elements", &["--element", MEMBER]);
    assert_eq!(axis_points(&j).len(), 2);

    // An unbound member cannot be released; an unknown grid cannot be followed.
    rejected(
        &f,
        "unbind-grid",
        &[
            "--id",
            MEMBER,
            "--op-id",
            "01990000-0000-7000-8000-000000000208",
        ],
        "INVALID_ARGUMENT",
    );
    rejected(
        &f,
        "bind-grid",
        &[
            "--id",
            MEMBER,
            "--grid",
            GRID2,
            "--op-id",
            "01990000-0000-7000-8000-000000000209",
        ],
        "NOT_FOUND",
    );
}

/// Rebinding is allowed: the stored local points are recomputed from the current world axis
/// against the newly named grid (contract §7.5, bind-grid).
#[test]
fn rebinding_names_a_new_frame_and_recomputes_the_local_axis() {
    let f = fixture();
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "Other",
            "--new-id",
            GRID2,
            "--u-expression",
            "0 3000",
            "--v-expression",
            "0 3000",
            "--op-id",
            "01990000-0000-7000-8000-000000000210",
        ],
    );
    ok(
        &f,
        "bind-grid",
        &[
            "--id",
            MEMBER,
            "--grid",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000203",
        ],
    );
    ok(
        &f,
        "bind-grid",
        &[
            "--id",
            MEMBER,
            "--grid",
            GRID2,
            "--op-id",
            "01990000-0000-7000-8000-000000000211",
        ],
    );
    // The world geometry is unchanged by a rebind to an identical frame; the record now
    // names GRID2, and deleting GRID is unblocked.
    let j = ok(&f, "inspect-elements", &["--element", MEMBER]);
    assert_eq!(axis_points(&j)[0], [1000.0, 1000.0, 0.0]);
    let (code, _) = f.run(
        "delete-element",
        &[
            "--id",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000212",
        ],
    );
    assert_eq!(code, 0);
}

/// GR-N22: a shape rewrite of a bound member refuses with `DEPENDENCY_CONFLICT` and leaves
/// the geometry unchanged; the same edit passes once the binding is released.
#[test]
fn a_shape_rewrite_of_a_bound_member_refuses_until_the_binding_is_released() {
    let f = fixture();
    ok(
        &f,
        "bind-grid",
        &[
            "--id",
            MEMBER,
            "--grid",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000203",
        ],
    );
    rejected(
        &f,
        "update-element",
        &[
            "--id",
            MEMBER,
            "--axis-point",
            "1500,1000,0;2500,1000,0",
            "--op-id",
            "01990000-0000-7000-8000-000000000213",
        ],
        "DEPENDENCY_CONFLICT",
    );
    let j = ok(&f, "inspect-elements", &["--element", MEMBER]);
    assert_eq!(axis_points(&j)[0], [1000.0, 1000.0, 0.0]);
    assert_eq!(axis_points(&j)[1], [2000.0, 1000.0, 0.0]);
    ok(
        &f,
        "unbind-grid",
        &[
            "--id",
            MEMBER,
            "--op-id",
            "01990000-0000-7000-8000-000000000205",
        ],
    );
    let (code, _) = f.run(
        "update-element",
        &[
            "--id",
            MEMBER,
            "--axis-point",
            "1500,1000,0;2500,1000,0",
            "--op-id",
            "01990000-0000-7000-8000-000000000214",
        ],
    );
    assert_eq!(code, 0);
    let j = ok(&f, "inspect-elements", &["--element", MEMBER]);
    assert_eq!(axis_points(&j)[0], [1500.0, 1000.0, 0.0]);
    assert_eq!(axis_points(&j)[1], [2500.0, 1000.0, 0.0]);
}

/// The store reopen path keeps the binding: a fresh process derives the same world geometry.
#[test]
fn the_binding_survives_a_reopen_and_the_delete_guard_stays_armed() {
    let f = fixture();
    ok(
        &f,
        "bind-grid",
        &[
            "--id",
            MEMBER,
            "--grid",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000203",
        ],
    );
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            GRID,
            "--origin",
            "1000,2000",
            "--rotation",
            "90",
            "--op-id",
            "01990000-0000-7000-8000-000000000204",
        ],
    );
    // Every command in this suite already runs in a fresh process (support::run_in); the
    // reopen happens on every call, so a refused delete after the move proves the index
    // still sees the binding relationship from the reopened store.
    rejected(
        &f,
        "delete-element",
        &[
            "--id",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000206",
        ],
        "DEPENDENCY_CONFLICT",
    );
}

const PLACED: &str = "01990000-0000-7000-8000-000000000110";

/// The world axis of the first member of an inspection: the element-local axis moved by the
/// member's placement translation (the placed members here carry no rotation).
fn placed_world(j: &Json) -> Vec<[f64; 3]> {
    let member = j.get("members").unwrap().array().unwrap()[0].clone();
    let placement = member.get("placement").unwrap();
    let q: Vec<f64> = placement
        .get("orientation_xyzw")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    assert_eq!(q, vec![0.0, 0.0, 0.0, 1.0]);
    let t: Vec<f64> = placement
        .get("translation_mm")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    axis_points(j)
        .into_iter()
        .map(|p| [p[0] + t[0], p[1] + t[1], p[2] + t[2]])
        .collect()
}

/// Issue 549: a member with a placement offset follows a grid rotation onto exactly the
/// position of an unbound twin transformed by hand; the inspected axis plus the placement
/// is that position, the release keeps it, and a placement edit of a bound member refuses
/// like a shape edit (GR-N22).
#[test]
fn a_placed_bound_member_follows_the_frame_with_its_placement_applied_once() {
    let f = fixture();
    let profile = text(
        ok(&f, "define-profile", &["--width", "200", "--height", "300"])
            .get("definition")
            .unwrap(),
        "digest",
    );
    let steel = text(
        ok(&f, "define-material", &["--density", "7850"])
            .get("definition")
            .unwrap(),
        "digest",
    );
    ok(
        &f,
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "8000,0,0",
            "--translation",
            "3000,2000,0",
            "--profile-digest",
            &profile,
            "--material-digest",
            &steel,
            "--name",
            "Moved",
            "--new-id",
            PLACED,
            "--op-id",
            "01990000-0000-7000-8000-000000000220",
        ],
    );
    let before = ok(&f, "inspect-elements", &["--element", PLACED]);
    let world_before = placed_world(&before);
    near(world_before[0], [3000.0, 2000.0, 0.0]);
    ok(
        &f,
        "bind-grid",
        &[
            "--id",
            PLACED,
            "--grid",
            GRID,
            "--op-id",
            "01990000-0000-7000-8000-000000000221",
        ],
    );
    // Binding alone must not move the member.
    let bound = ok(&f, "inspect-elements", &["--element", PLACED]);
    near(placed_world(&bound)[0], [3000.0, 2000.0, 0.0]);
    near(placed_world(&bound)[1], [11000.0, 2000.0, 0.0]);
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            GRID,
            "--origin",
            "2000,1000",
            "--op-id",
            "01990000-0000-7000-8000-000000000222",
        ],
    );
    ok(
        &f,
        "update-grid",
        &[
            "--id",
            GRID,
            "--rotation",
            "90",
            "--op-id",
            "01990000-0000-7000-8000-000000000223",
        ],
    );
    // The unbound twin moved by (2000,1000) and turned 90 degrees about (2000,1000) lands at
    // (0,4000) -> (0,12000).
    let turned = ok(&f, "inspect-elements", &["--element", PLACED]);
    let world = placed_world(&turned);
    near(world[0], [0.0, 4000.0, 0.0]);
    near(world[1], [0.0, 12000.0, 0.0]);
    // A placement edit of a bound member is a hand edit: it refuses with nothing written.
    rejected(
        &f,
        "update-element",
        &[
            "--id",
            PLACED,
            "--translation",
            "0,0,0",
            "--op-id",
            "01990000-0000-7000-8000-000000000224",
        ],
        "DEPENDENCY_CONFLICT",
    );
    ok(
        &f,
        "unbind-grid",
        &[
            "--id",
            PLACED,
            "--op-id",
            "01990000-0000-7000-8000-000000000225",
        ],
    );
    let released = ok(&f, "inspect-elements", &["--element", PLACED]);
    let world = placed_world(&released);
    near(world[0], [0.0, 4000.0, 0.0]);
    near(world[1], [0.0, 12000.0, 0.0]);
}
