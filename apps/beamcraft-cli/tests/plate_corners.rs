// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected areas are the closed forms of the F04
// plate amendment's acceptance cases (a cut triangle, a fillet's complement of a square), never
// production math.

//! A12.2 end to end through the real `beamcraft` binary: `create-plate --corners` authors
//! chamfers and roundings as parameters, the amendment's area cases hold on the saved revision,
//! the negative cases change nothing, and the treatments survive copy, undo, redo and a fresh
//! process reopen exactly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use beamcraft_json::Json;

const USER: &str = "01890a5d-ac96-774b-b9cc-cccccccccccc";

fn ok(f: &support::StoreFixture, command: &str, extra: &[&str]) -> Json {
    let (code, body) = f.run(command, extra);
    assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
    body
}

fn text(j: &Json, key: &str) -> String {
    j.get(key).unwrap().string().unwrap().to_owned()
}

fn at<'a>(j: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(j, |j, k| j.get(k).unwrap())
}

fn num(j: &Json, path: &[&str]) -> f64 {
    at(j, path).float().unwrap()
}

fn list<'a>(j: &'a Json, key: &str) -> &'a [Json] {
    j.get(key).unwrap().array().unwrap()
}

fn plates(f: &support::StoreFixture) -> Vec<Json> {
    list(&ok(f, "inspect-elements", &[]), "plates").to_vec()
}

fn by_id<'a>(plates: &'a [Json], id: &str) -> &'a Json {
    plates
        .iter()
        .find(|p| text(p, "element_id") == id)
        .unwrap_or_else(|| panic!("plate {id} is not listed"))
}

/// A treated 1000 x 1000 x 10 plate: the volume and the corners the inspection reports.
/// An empty `corners` authors no option.
fn create_treated(f: &support::StoreFixture, name: &str, corners: &str) -> Json {
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

/// The command must fail and the store must read exactly as before.
fn refused(f: &support::StoreFixture, command: &str, extra: &[&str]) {
    let before = ok(f, "open-revision", &[]);
    let (code, body) = f.run(command, extra);
    assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
    assert_eq!(
        before,
        ok(f, "open-revision", &[]),
        "{command} changed the store"
    );
}

/// The 1000 x 1000 square's arguments plus one `--corners` option.
fn with_corners<'a>(square: &[&'a str], corners: &'a str) -> Vec<&'a str> {
    let mut all = square.to_vec();
    all.extend_from_slice(&["--corners", corners]);
    all
}

#[test]
fn the_amendment_area_cases_hold_on_the_saved_revision() {
    let f = support::StoreFixture::new("a12-2-areas");

    // Case 2: one chamfer d1 = d2 = 100 cuts a 5,000 mm2 triangle; a single distance gives the
    // same symmetric cut, down to the same mesh bytes.
    let one = create_treated(&f, "One value", "2=chamfer:100");
    let two = create_treated(&f, "Two values", "2=chamfer:100,100");
    for created in [&one, &two] {
        let listed = plates(&f);
        let plate = by_id(&listed, &text(at(created, &["plate"]), "element_id"));
        assert!(
            (num(plate, &["quantities", "volume", "mm3"]) - 9_950_000.0).abs() <= 1e-6,
            "{}",
            num(plate, &["quantities", "volume", "mm3"])
        );
        // A treated origin rectangle is no longer the box: it is a contour.
        assert_eq!(text(plate, "outline"), "contour");
        assert_eq!(at(plate, &["arc_deviation_mm"]), &Json::Null);
    }
    let one_id = text(at(&one, &["plate"]), "element_id");
    let two_id = text(at(&two, &["plate"]), "element_id");
    assert_eq!(
        text(by_id(&plates(&f), &one_id), "mesh_digest"),
        text(by_id(&plates(&f), &two_id), "mesh_digest")
    );

    // Case 3: d1 = 100 along the previous edge and d2 = 50 remove 2,500 mm2.
    let three = create_treated(&f, "Asymmetric", "1=chamfer:100,50");
    let listed = plates(&f);
    let plate = by_id(&listed, &text(at(&three, &["plate"]), "element_id"));
    assert!(
        (num(plate, &["quantities", "volume", "mm3"]) - 9_975_000.0).abs() <= 1e-6,
        "{}",
        num(plate, &["quantities", "volume", "mm3"])
    );

    // Case 4: one rounding of radius 100 stays within the reported deviation of the analytic
    // value, and the deviation itself is at most 0.05 mm.
    let four = create_treated(&f, "Rounded", "2=round:100");
    let rounded_id = text(at(&four, &["plate"]), "element_id");
    let listed = plates(&f);
    let plate = by_id(&listed, &rounded_id);
    let analytic = 10.0 * (1_000_000.0 - (1.0 - core::f64::consts::FRAC_PI_4) * 10_000.0);
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!(
        (volume - analytic).abs() <= 100.0,
        "faceted {volume} vs analytic {analytic}"
    );
    let deviation = num(plate, &["arc_deviation_mm"]);
    assert!(deviation > 0.0 && deviation <= 0.05, "{deviation}");

    // The authored parameters read back through a fresh process, vertex and treatment intact.
    let stored_listed = plates(&f);
    let stored = by_id(&stored_listed, &rounded_id);
    let corners = list(stored, "corners");
    assert_eq!(corners.len(), 1);
    assert_eq!(corners[0].get("vertex").unwrap().float().unwrap(), 2.0);
    assert_eq!(text(&corners[0], "corner"), "round");
    assert_eq!(
        corners[0]
            .get("values_mm")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [100.0]
    );
    // A mixed plate reports both entries in authored order.
    let mixed = create_treated(&f, "Mixed", "0=chamfer:100;3=round:50");
    let mixed_listed = plates(&f);
    let plate = by_id(&mixed_listed, &text(at(&mixed, &["plate"]), "element_id"));
    let corners = list(plate, "corners");
    assert_eq!(corners.len(), 2);
    assert_eq!(text(&corners[0], "corner"), "chamfer");
    assert_eq!(
        corners[0]
            .get("values_mm")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [100.0, 100.0]
    );
    assert_eq!(text(&corners[1], "corner"), "round");
}

#[test]
fn the_negative_cases_change_nothing() {
    let f = support::StoreFixture::new("a12-2-negative");
    let square = ["--width", "1000", "--length", "1000"];
    // One valid plate so the store has a head revision to compare against.
    create_treated(&f, "Sharp", "");

    // Case 1: zero, negative, non-finite and over the cap.
    for corners in [
        "0=round:0",
        "0=round:-5",
        "0=round:nan",
        "0=round:2000000000",
    ] {
        refused(&f, "create-plate", &with_corners(&square, corners));
    }
    // A vertex that does not exist, and one named twice.
    refused(&f, "create-plate", &with_corners(&square, "9=chamfer:100"));
    refused(
        &f,
        "create-plate",
        &with_corners(&square, "0=round:10;0=chamfer:10"),
    );
    // Case 2: the consumed lengths of one edge reach the edge length.
    refused(
        &f,
        "create-plate",
        &with_corners(&square, "0=chamfer:500,500;1=chamfer:500,500"),
    );
    // Case 3: a treatment on a collinear vertex.
    refused(
        &f,
        "create-plate",
        &[
            "--contour",
            "0,0;500,0;1000,0;1000,500;0,500",
            "--corners",
            "1=round:50",
        ],
    );
    // Malformed syntax is refused at the option level.
    refused(&f, "create-plate", &with_corners(&square, "0=bogus:5"));
    refused(&f, "create-plate", &with_corners(&square, "2=round"));
    // The refusals left the store with exactly the one sharp plate.
    assert_eq!(plates(&f).len(), 1);
}

#[test]
fn treatments_survive_copy_undo_and_reopen_exactly() {
    let f = support::StoreFixture::new("a12-2-persistence");
    let created = create_treated(&f, "Source", "0=chamfer:100;3=round:50");
    let source = text(at(&created, &["plate"]), "element_id");
    let before = by_id(&plates(&f), &source).clone();

    // A copy carries the treatments: the same corners, mesh bytes and quantities.
    let copied = ok(
        &f,
        "copy-element",
        &["--id", &source, "--name", "Copy", "--offset", "2000,0,0"],
    );
    let copy_id = text(at(&copied, &["element"]), "element_id");
    let after = plates(&f);
    let copy = by_id(&after, &copy_id);
    assert_eq!(copy.get("corners"), before.get("corners"));
    assert_eq!(text(copy, "mesh_digest"), text(&before, "mesh_digest"));
    assert_eq!(
        num(copy, &["quantities", "volume", "mm3"]),
        num(&before, &["quantities", "volume", "mm3"])
    );

    // Numbering follows the geometry: the treated outlines match, so one group holds both.
    let preview = ok(&f, "preview-numbering", &[]);
    let groups = list(&preview, "groups");
    assert_eq!(groups.len(), 1, "{preview:?}");
    let members: Vec<String> = groups[0]
        .get("members")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|m| m.string().unwrap().to_owned())
        .collect();
    assert_eq!(members, [source.clone(), copy_id]);

    // One undo unit in a workspace: a treated create undoes whole, redo restores it byte for
    // byte. Workspace history commands take no --store; the workspace directory says it all.
    let dir = f.root.join("ws").to_str().unwrap().to_owned();
    ok(
        &f,
        "create-workspace",
        &["--workspace-dir", &dir, "--user-id", USER],
    );
    let draft = create_treated_in(&dir, "Draft plate", "2=round:100");
    let draft_id = text(at(&draft, &["plate"]), "element_id");
    let draft_before = by_id(&plates_in(&dir), &draft_id).clone();
    ok_raw(&["undo-workspace", "--workspace-dir", &dir]);
    assert!(
        plates_in(&dir)
            .iter()
            .all(|p| text(p, "element_id") != draft_id)
    );
    ok_raw(&["redo-workspace", "--workspace-dir", &dir]);
    let after_redo = plates_in(&dir);
    let restored = by_id(&after_redo, &draft_id);
    assert_eq!(restored.get("corners"), draft_before.get("corners"));
    assert_eq!(
        text(restored, "mesh_digest"),
        text(&draft_before, "mesh_digest")
    );
    assert_eq!(
        num(restored, &["quantities", "volume", "mm3"]),
        num(&draft_before, &["quantities", "volume", "mm3"])
    );
}

/// A workspace-scoped command: the store argument is the workspace directory, not the fixture's.
fn ok_in(store: &str, command: &str, extra: &[&str]) -> Json {
    let (code, body) = support::StoreFixture::run_in(store, command, extra);
    assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
    body
}

/// A command with its full argument list, for the workspace history commands that take no
/// `--store` at all.
fn ok_raw(args: &[&str]) -> Json {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .output()
        .unwrap();
    let body = beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert_eq!(output.status.code().unwrap(), 0, "{args:?}: {body:?}");
    body
}

fn create_treated_in(store: &str, name: &str, corners: &str) -> Json {
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
