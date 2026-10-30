// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected volumes are the closed forms of the
// F04 plate amendment's contour-editing acceptance cases, never production math.

//! A12.3 end to end through the real `beamcraft` binary: the five contour-edit commands move,
//! insert and delete vertices and set one corner, each as one shape-changing revision that
//! replays the holes and cuts, refuses invalid geometry before publication and keeps the
//! treatments, placement and identities of the plate.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support::{StoreFixture, at, list, text};

use beamcraft_json::Json;

const USER: &str = "01890a5d-ac96-774b-b9cc-cccccccccccc";

fn ok(f: &StoreFixture, command: &str, extra: &[&str]) -> Json {
    let (code, body) = f.run(command, extra);
    assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
    body
}

fn num(j: &Json, path: &[&str]) -> f64 {
    at(j, path).float().unwrap()
}

fn plates(f: &StoreFixture) -> Vec<Json> {
    list(&ok(f, "inspect-elements", &[]), "plates").to_vec()
}

fn by_id<'a>(plates: &'a [Json], id: &str) -> &'a Json {
    plates
        .iter()
        .find(|p| text(p, "element_id") == id)
        .unwrap_or_else(|| panic!("plate {id} is not listed"))
}

/// A 1000 x 1000 x 10 plate with optional corner treatments.
fn create_plate(f: &StoreFixture, name: &str, corners: &str) -> Json {
    let mut extra: Vec<&str> = vec![
        "--width",
        "1000",
        "--length",
        "1000",
        "--thickness",
        "10",
        "--name",
        name,
    ];
    if !corners.is_empty() {
        extra.extend(["--corners", corners]);
    }
    ok(f, "create-plate", &extra)
}

fn element_id(created: &Json) -> String {
    text(at(created, &["plate"]), "element_id")
}

/// The command must fail and the store must read exactly as before.
fn refused(f: &StoreFixture, command: &str, extra: &[&str]) {
    let before = ok(f, "open-revision", &[]);
    let (code, body) = f.run(command, extra);
    assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
    assert_eq!(
        before,
        ok(f, "open-revision", &[]),
        "{command} changed the store"
    );
}

#[test]
fn delete_insert_and_edge_move_shape_the_contour_as_the_amendment_cases_say() {
    let f = StoreFixture::new("a12-3-shape-cases");
    // Case 6: a delete on a quadrilateral gives a triangle (half the square), an insert then a
    // move gives a pentagon, and an edge move changes exactly its two end vertices.
    let square = create_plate(&f, "Square", "");
    let id = element_id(&square);
    let deleted = ok(
        &f,
        "delete-plate-vertex",
        &[
            "--id",
            &id,
            "--vertex",
            "0",
            "--op-id",
            "019a7bf0-0000-7000-8000-000000ce1201",
        ],
    );
    assert_eq!(text(&deleted, "outcome"), "plate_vertex_deleted");
    assert!(at(&deleted, &["effect", "shape"]) == &Json::Bool(true));
    assert_eq!(
        num(by_id(&plates(&f), &id), &["quantities", "volume", "mm3"]),
        5_000_000.
    );

    // The insert splits the top edge with a collinear sharp vertex and keeps the body; the
    // outward move adds the 1000 x 500 / 2 corner triangle.
    let inserted = ok(
        &f,
        "insert-plate-vertex",
        &["--id", &id, "--edge", "1", "--point", "500,1000"],
    );
    assert_eq!(text(&inserted, "outcome"), "plate_vertex_inserted");
    assert_eq!(
        num(by_id(&plates(&f), &id), &["quantities", "volume", "mm3"]),
        5_000_000.
    );
    let moved = ok(
        &f,
        "move-plate-vertex",
        &["--id", &id, "--vertex", "2", "--to", "500,1500"],
    );
    assert_eq!(text(&moved, "outcome"), "plate_vertex_moved");
    assert_eq!(
        num(by_id(&plates(&f), &id), &["quantities", "volume", "mm3"]),
        7_500_000.
    );

    // The edge move on a fresh treated square: edge 3 joins the wrapped pair (vertex 3 and
    // vertex 0), so the plate grows 200 mm to the left and the treated corner stays cut.
    let edged = create_plate(&f, "Edge", "2=chamfer:100");
    let edge_id = element_id(&edged);
    ok(
        &f,
        "move-plate-edge",
        &["--id", &edge_id, "--edge", "3", "--delta", "-200,0"],
    );
    let after = plates(&f);
    assert_eq!(
        num(by_id(&after, &edge_id), &["quantities", "volume", "mm3"]),
        11_950_000.
    );
    // The treatment survived the edge move on its vertex.
    let corners = list(by_id(&after, &edge_id), "corners");
    assert_eq!(corners.len(), 1);
    assert_eq!(corners[0].get("vertex").unwrap().float().unwrap(), 2.0);
}

#[test]
fn a_vertex_move_takes_its_treatment_along_and_a_set_corner_edits_one_vertex() {
    let f = StoreFixture::new("a12-3-treatments");
    let created = create_plate(&f, "Treated", "2=chamfer:100");
    let id = element_id(&created);
    // Move a sharp vertex: the chamfer stays on vertex 2 with its values, and the cut is
    // replayed on the new contour (1,250,000 mm2 less the 5,000 mm2 triangle).
    ok(
        &f,
        "move-plate-vertex",
        &["--id", &id, "--vertex", "0", "--to", "-500,0"],
    );
    let listed = plates(&f);
    let plate = by_id(&listed, &id);
    assert_eq!(num(plate, &["quantities", "volume", "mm3"]), 12_450_000.);
    let corners = list(plate, "corners");
    assert_eq!(corners.len(), 1);
    assert_eq!(corners[0].get("vertex").unwrap().float().unwrap(), 2.0);

    // Set one more corner on the moved vertex 0 and clear the first one.
    ok(
        &f,
        "set-plate-corner",
        &["--id", &id, "--vertex", "0", "--corner", "round:50"],
    );
    let listed = plates(&f);
    let corners = list(by_id(&listed, &id), "corners");
    assert_eq!(corners.len(), 2);
    assert_eq!(text(&corners[0], "corner"), "round");
    ok(
        &f,
        "set-plate-corner",
        &["--id", &id, "--vertex", "2", "--corner", "sharp"],
    );
    let listed = plates(&f);
    let corners = list(by_id(&listed, &id), "corners");
    assert_eq!(corners.len(), 1);
    assert_eq!(corners[0].get("vertex").unwrap().float().unwrap(), 0.0);
    // An immediate retry through the same op id replays and writes nothing.
    let args = [
        "--id",
        &id,
        "--vertex",
        "3",
        "--corner",
        "chamfer:60",
        "--op-id",
        "019a7bf0-0000-7000-8000-000000ce1301",
    ];
    ok(&f, "set-plate-corner", &args);
    let head = ok(&f, "open-revision", &[]);
    let replay = ok(&f, "set-plate-corner", &args);
    assert_eq!(text(&replay, "outcome"), "contour_edit_replayed");
    assert_eq!(ok(&f, "open-revision", &[]), head);
}

#[test]
fn a_delete_removes_the_treatment_and_an_insert_shifts_the_rest() {
    let f = StoreFixture::new("a12-3-delete-shift");
    let created = create_plate(&f, "Two", "1=round:80;3=chamfer:60");
    let id = element_id(&created);
    // Deleting vertex 1 removes its rounding; the chamfer shifts from 3 to 2.
    ok(&f, "delete-plate-vertex", &["--id", &id, "--vertex", "1"]);
    let listed = plates(&f);
    let corners = list(by_id(&listed, &id), "corners");
    assert_eq!(corners.len(), 1);
    assert_eq!(corners[0].get("vertex").unwrap().float().unwrap(), 2.0);
    assert_eq!(text(&corners[0], "corner"), "chamfer");
    // The inserted vertex is sharp and the treatment shifts past it; the point splits the
    // diagonal edge, so the chamfered corner keeps turning.
    ok(
        &f,
        "insert-plate-vertex",
        &["--id", &id, "--edge", "0", "--point", "400,400"],
    );
    let listed = plates(&f);
    let corners = list(by_id(&listed, &id), "corners");
    assert_eq!(corners.len(), 1);
    assert_eq!(corners[0].get("vertex").unwrap().float().unwrap(), 3.0);
}

#[test]
fn the_negative_cases_change_nothing() {
    let f = StoreFixture::new("a12-3-negative");
    let created = create_plate(&f, "Sharp", "");
    let id = element_id(&created);

    // A triangle is the floor: the first delete works, the next is refused.
    ok(&f, "delete-plate-vertex", &["--id", &id, "--vertex", "0"]);
    refused(&f, "delete-plate-vertex", &["--id", &id, "--vertex", "0"]);
    // Out-of-range and malformed indices.
    refused(&f, "delete-plate-vertex", &["--id", &id, "--vertex", "9"]);
    refused(
        &f,
        "move-plate-edge",
        &["--id", &id, "--edge", "4", "--delta", "1,0"],
    );
    refused(
        &f,
        "move-plate-vertex",
        &["--id", &id, "--vertex", "x", "--to", "1,0"],
    );
    // Non-finite and over-cap points.
    refused(
        &f,
        "move-plate-vertex",
        &["--id", &id, "--vertex", "0", "--to", "nan,0"],
    );
    refused(
        &f,
        "move-plate-vertex",
        &["--id", &id, "--vertex", "0", "--to", "2000000000,0"],
    );
    // Negative case 5: a vertex moved so the contour crosses itself (a bowtie).
    let bowtie = create_plate(&f, "Bowtie", "");
    refused(
        &f,
        "move-plate-vertex",
        &[
            "--id",
            &element_id(&bowtie),
            "--vertex",
            "1",
            "--to",
            "-500,500",
        ],
    );
    // Negative case 6: an edge move that leaves a hole outside the plate.
    let holed = ok(
        &f,
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "1000",
            "--thickness",
            "10",
            "--name",
            "Holed",
            "--hole-rect",
            "700,400,200,200",
        ],
    );
    refused(
        &f,
        "move-plate-edge",
        &[
            "--id",
            &element_id(&holed),
            "--edge",
            "1",
            "--delta",
            "-500,0",
        ],
    );
    // A treatment on a collinear vertex (negative 3), through set-plate-corner.
    let collinear = ok(
        &f,
        "create-plate",
        &[
            "--contour",
            "0,0;500,0;1000,0;1000,500;0,500",
            "--thickness",
            "10",
            "--name",
            "Collinear",
        ],
    );
    refused(
        &f,
        "set-plate-corner",
        &[
            "--id",
            &element_id(&collinear),
            "--vertex",
            "1",
            "--corner",
            "round:50",
        ],
    );
    // Malformed treatment text and an unknown target.
    refused(
        &f,
        "set-plate-corner",
        &["--id", &id, "--vertex", "0", "--corner", "bogus:5"],
    );
    refused(
        &f,
        "set-plate-corner",
        &["--id", &id, "--vertex", "0", "--corner", "round"],
    );
    refused(
        &f,
        "move-plate-vertex",
        &[
            "--id",
            "019a7bf0-0000-7000-8000-000000ce1302",
            "--vertex",
            "0",
            "--to",
            "1,0",
        ],
    );
    // A member has no contour.
    ok(
        &f,
        "define-profile",
        &["--width", "100", "--height", "200", "--name", "M"],
    );
    let member = ok(
        &f,
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile",
            "M",
            "--name",
            "Beam",
        ],
    );
    refused(
        &f,
        "move-plate-vertex",
        &[
            "--id",
            &text(at(&member, &["member"]), "element_id"),
            "--vertex",
            "0",
            "--to",
            "1,0",
        ],
    );
    // The refusals left exactly the created plates.
    assert_eq!(plates(&f).len(), 4);
}

#[test]
fn an_edit_replays_the_linked_group_atomically_and_reuses_its_op_id() {
    let f = StoreFixture::new("a12-3-linked");
    let created = create_plate(&f, "Source", "0=round:100");
    let id = element_id(&created);
    let instance = ok(&f, "copy-instance", &["--id", &id, "--offset", "2000,0,0"]);
    let instance_id = text(at(&instance, &["element"]), "element_id");
    let op = "019a7bf0-0000-7000-8000-000000ce1401";
    let moved = ok(
        &f,
        "move-plate-vertex",
        &[
            "--id",
            &id,
            "--vertex",
            "2",
            "--to",
            "1500,1500",
            "--op-id",
            op,
        ],
    );
    assert!(at(&moved, &["effect", "shape"]) == &Json::Bool(true));
    let after = plates(&f);
    let definition = by_id(&after, &id);
    let linked = by_id(&after, &instance_id);
    assert_eq!(definition.get("corners"), linked.get("corners"));
    assert_eq!(definition.get("mesh_digest"), linked.get("mesh_digest"));
    // An immediate retry replays and writes nothing.
    let head = ok(&f, "open-revision", &[]);
    let replay = ok(
        &f,
        "move-plate-vertex",
        &[
            "--id",
            &id,
            "--vertex",
            "2",
            "--to",
            "1500,1500",
            "--op-id",
            op,
        ],
    );
    assert_eq!(text(&replay, "outcome"), "contour_edit_replayed");
    assert_eq!(ok(&f, "open-revision", &[]), head);
    // The same operation id with different content is a reuse, not a second mutation.
    let (code, error) = f.run(
        "insert-plate-vertex",
        &[
            "--id", &id, "--edge", "0", "--point", "500,1000", "--op-id", op,
        ],
    );
    assert_eq!(code, 2, "{error:?}");
    assert_eq!(text(&error, "code"), "OP_ID_REUSE");
    assert_eq!(ok(&f, "open-revision", &[]), head);
}

#[test]
fn each_edit_is_one_undo_unit_and_survives_redo() {
    let f = StoreFixture::new("a12-3-undo");
    create_plate(&f, "base", "");
    let dir = f.root.join("ws").to_str().unwrap().to_owned();
    ok(
        &f,
        "create-workspace",
        &["--workspace-dir", &dir, "--user-id", USER],
    );
    let created = create_plate_in(&dir, "draft", "0=chamfer:80");
    let id = element_id(&created);
    let before = by_id(&plates_in(&dir), &id).clone();
    let edited = ok_in(
        &dir,
        "move-plate-vertex",
        &["--id", &id, "--vertex", "1", "--to", "1500,0"],
    );
    let expected = at(&edited, &["element"]).clone();
    ok_raw(&["undo-workspace", "--workspace-dir", &dir]);
    let listed = plates_in(&dir);
    let undone = by_id(&listed, &id);
    assert_eq!(undone.get("mesh_digest"), before.get("mesh_digest"));
    assert_eq!(undone.get("corners"), before.get("corners"));
    ok_raw(&["redo-workspace", "--workspace-dir", &dir]);
    let listed = plates_in(&dir);
    let redone = by_id(&listed, &id);
    assert_eq!(redone.get("corners"), expected.get("corners"));
    assert_eq!(
        num(redone, &["quantities", "volume", "mm3"]),
        num(&expected, &["quantities", "volume", "mm3"])
    );
    // A corner set undoes as its own unit, back to the moved state.
    ok_in(
        &dir,
        "set-plate-corner",
        &["--id", &id, "--vertex", "3", "--corner", "round:40"],
    );
    ok_raw(&["undo-workspace", "--workspace-dir", &dir]);
    let listed = plates_in(&dir);
    let undone = by_id(&listed, &id);
    assert_eq!(undone.get("corners"), expected.get("corners"));
}

/// Workspace-scoped helpers, as in `plate_corners`.
fn ok_in(store: &str, command: &str, extra: &[&str]) -> Json {
    let (code, body) = StoreFixture::run_in(store, command, extra);
    assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
    body
}

fn ok_raw(args: &[&str]) -> Json {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .output()
        .unwrap();
    let body = beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(output.status.code().unwrap(), 0, "{args:?}: {body:?}");
    body
}

fn create_plate_in(store: &str, name: &str, corners: &str) -> Json {
    ok_in(
        store,
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "1000",
            "--thickness",
            "10",
            "--name",
            name,
            "--corners",
            corners,
        ],
    )
}

fn plates_in(store: &str) -> Vec<Json> {
    list(&ok_in(store, "inspect-elements", &[]), "plates").to_vec()
}
