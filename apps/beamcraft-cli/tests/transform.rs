// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and beams, MPL-2.0. Expected values are closed forms.

//! T03 end to end: rotate and mirror turn one element in place through `update-element`, the
//! arrays create independent copies in one revision, and every operation is exactly one undo
//! step. Links follow the copies (the source's level) and the quantities are rebuilt where the
//! shape is re-encoded.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const USER: &str = "01890a5d-ac96-774b-b9cc-aaaaaaaaaaaa";
const QUARTER: &str = "1.5707963267948966";

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("t03-tests"),
        }
    }
    fn dir(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn run(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.run(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }
    fn code(&self, args: &[&str]) -> String {
        let (code, body) = self.run(args);
        assert_ne!(code, 0, "{args:?}: {body:?}");
        text(&body, "code")
    }
    /// A published store with one plate and a forked workspace draft over it.
    fn forked(&self, name: &str) -> (String, String, String) {
        let published = self.dir(name);
        std::fs::create_dir_all(&published).unwrap();
        let created = self.ok(&[
            "create-plate",
            "--store",
            &published,
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--translation",
            "1000,0,0",
        ]);
        let id = text(at(&created, &["plate"]), "element_id");
        let dir = self.dir(&format!("{name}-ws"));
        self.ok(&[
            "create-workspace",
            "--store",
            &published,
            "--workspace-dir",
            &dir,
            "--user-id",
            USER,
        ]);
        (published, dir, id)
    }
    fn edit(&self, dir: &str, args: &[&str]) -> Json {
        let mut all = vec![args[0], "--store", dir];
        all.extend_from_slice(&args[1..]);
        self.ok(&all)
    }
    fn undo(&self, dir: &str) -> Json {
        self.ok(&["undo-workspace", "--workspace-dir", dir])
    }
    fn plates(&self, dir: &str) -> Vec<Json> {
        at(&self.edit(dir, &["inspect-elements"]), &["plates"])
            .array()
            .unwrap()
            .to_vec()
    }
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

fn find<'a>(elements: &'a [Json], id: &str) -> &'a Json {
    elements
        .iter()
        .find(|e| text(e, "element_id") == id)
        .unwrap()
}

fn translation(element: &Json) -> Vec<f64> {
    at(element, &["placement", "translation_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}

fn orientation(element: &Json) -> Vec<f64> {
    at(element, &["placement", "orientation_xyzw"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}

#[test]
fn a_rotation_is_one_placement_step_and_one_undo() {
    let f = Fixture::new();
    let (_published, dir, id) = f.forked("rotate");
    // A quarter turn about Z through the origin turns the (1000, 0, 0) placement onto +Y and
    // composes with the identity orientation.
    let updated = f.edit(
        &dir,
        &[
            "update-element",
            "--id",
            &id,
            "--rotate-about",
            "0,0,0",
            "--rotate-axis",
            "0,0,1",
            "--rotate-angle",
            QUARTER,
        ],
    );
    assert_eq!(text(&updated, "outcome"), "element_updated");
    let effect = at(&updated, &["effect"]);
    assert!(at(effect, &["rotate"]).boolean().unwrap());
    assert!(at(effect, &["placement"]).boolean().unwrap());
    let element = at(&updated, &["element"]);
    let moved = translation(element);
    assert!(moved[0].abs() < 1e-6, "{moved:?}");
    assert!((moved[1] - 1000.0).abs() < 1e-6, "{moved:?}");
    let turned = orientation(element);
    assert!((turned[2] - 0.5_f64.sqrt()).abs() < 1e-6, "{turned:?}");
    // The same op id recomputes the same absolute placement from the head: a replay.
    let replay = f.edit(
        &dir,
        &[
            "update-element",
            "--id",
            &id,
            "--rotate-about",
            "0,0,0",
            "--rotate-axis",
            "0,0,1",
            "--rotate-angle",
            QUARTER,
            "--op-id",
            &text(&updated, "op_id"),
        ],
    );
    assert_eq!(text(&replay, "outcome"), "element_update_replayed");
    // One undo step restores the placement.
    f.undo(&dir);
    let plates = f.plates(&dir);
    let restored = find(&plates, &id);
    assert_eq!(translation(restored)[0], 1000.0);
    assert!(translation(restored)[1].abs() < 1e-9);
    // Absolute placement fields and a rotate do not mix; neither do rotate and mirror.
    assert_eq!(
        f.code(&[
            "update-element",
            "--store",
            &dir,
            "--id",
            &id,
            "--rotate-about",
            "0,0,0",
            "--rotate-axis",
            "0,0,1",
            "--rotate-angle",
            QUARTER,
            "--translation",
            "1,2,3",
        ]),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn a_plate_mirror_reencodes_the_side_and_the_cuts_and_rebuilds() {
    let f = Fixture::new();
    let (published, dir, id) = f.forked("mirror-plate");
    // A cut records the geometry the mirror must reflect.
    f.edit(
        &dir,
        &[
            "cut-by-plane",
            "--id",
            &id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000a1",
            "--point",
            "250,0,5",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    let mirrored = f.edit(
        &dir,
        &[
            "update-element",
            "--id",
            &id,
            "--mirror-point",
            "0,0,0",
            "--mirror-normal",
            "1,0,0",
        ],
    );
    assert_eq!(text(&mirrored, "outcome"), "element_updated");
    let element = at(&mirrored, &["element"]);
    // The (1000, 0, 0) placement mirrors onto (-1000, 0, 0) about x = 0.
    let moved = translation(element);
    assert!((moved[0] + 1000.0).abs() < 1e-6, "{moved:?}");
    // The body is rebuilt (a shape effect) and the cut survives mirrored: same plane offset
    // through the element-local reflection, the kept side carried over.
    assert!(at(&mirrored, &["effect", "shape"]).boolean().unwrap());
    let cuts = at(element, &["cuts"]).array().unwrap();
    assert_eq!(cuts.len(), 1);
    f.undo(&dir);
    let plates = f.plates(&dir);
    let restored = find(&plates, &id);
    assert!((translation(restored)[0] - 1000.0).abs() < 1e-9);
    // The published (base) element was never written by the draft edits.
    let base = f.ok(&["inspect-elements", "--store", &published]);
    let base_plates = at(&base, &["plates"]).array().unwrap().to_vec();
    let base_plate = find(&base_plates, &id);
    assert!((translation(base_plate)[0] - 1000.0).abs() < 1e-9);
}

#[test]
fn a_member_mirror_reflects_the_path_and_refuses_a_polybeam() {
    let f = Fixture::new();
    let published = f.dir("mirror-member");
    std::fs::create_dir_all(&published).unwrap();
    let profile = text(
        at(
            &f.ok(&[
                "define-profile",
                "--store",
                &published,
                "--width",
                "200",
                "--height",
                "300",
            ]),
            &["definition"],
        ),
        "digest",
    );
    let created = f.ok(&[
        "create-member",
        "--store",
        &published,
        "--profile-digest",
        &profile,
        "--start",
        "0,0,0",
        "--end",
        "2000,0,0",
        "--kind",
        "beam",
    ]);
    let id = text(at(&created, &["member"]), "element_id");
    let dir = f.dir("mirror-member-ws");
    f.ok(&[
        "create-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &dir,
        "--user-id",
        USER,
    ]);
    let mirrored = f.edit(
        &dir,
        &[
            "update-element",
            "--id",
            &id,
            "--mirror-point",
            "0,0,0",
            "--mirror-normal",
            "0,1,0",
        ],
    );
    let element = at(&mirrored, &["element"]);
    // The path reflects across y = 0: with the axis on the plane it keeps its points, and the
    // placement orientation carries the mirror.
    let points = at(element, &["axis_points_mm"]).array().unwrap();
    assert_eq!(points.len(), 2);
    assert!((num(element, &["axis_length_mm"]) - 2000.0).abs() < 1e-6);
    // A polybeam refuses: its mirrored path is not expressible.
    let created = f.edit(
        &dir,
        &[
            "create-polybeam",
            "--profile-digest",
            &profile,
            "--points",
            "0,0,0;1000,0,0;1000,1000,0",
            "--kind",
            "beam",
        ],
    );
    let bent = text(at(&created, &["member"]), "element_id");
    assert_eq!(
        f.code(&[
            "update-element",
            "--store",
            &dir,
            "--id",
            &bent,
            "--mirror-point",
            "0,0,0",
            "--mirror-normal",
            "1,0,0",
        ]),
        "UNSUPPORTED_GEOMETRY"
    );
}

#[test]
fn a_drilled_member_refuses_the_copy_the_array_and_the_mirror() {
    let f = Fixture::new();
    let published = f.dir("drilled");
    std::fs::create_dir_all(&published).unwrap();
    let profile = text(
        at(
            &f.ok(&[
                "define-profile",
                "--store",
                &published,
                "--width",
                "200",
                "--height",
                "300",
            ]),
            &["definition"],
        ),
        "digest",
    );
    let created = f.ok(&[
        "create-member",
        "--store",
        &published,
        "--profile-digest",
        &profile,
        "--start",
        "0,0,0",
        "--end",
        "2000,0,0",
        "--kind",
        "beam",
    ]);
    let id = text(at(&created, &["member"]), "element_id");
    f.ok(&[
        "add-hole",
        "--store",
        &published,
        "--id",
        &id,
        "--operation-id",
        "019a7bf0-0000-7000-8000-0000000000d1",
        "--axis-point",
        "0,0,1000",
        "--axis-direction",
        "1,0,0",
        "--diameter",
        "50",
    ]);
    let dir = f.dir("drilled-ws");
    f.ok(&[
        "create-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &dir,
        "--user-id",
        USER,
    ]);
    // The hole records have no copy, array or mirror authoring yet: all three refuse before
    // anything is written instead of silently dropping the holes.
    assert_eq!(
        f.code(&["copy-element", "--store", &dir, "--id", &id]),
        "DEPENDENCY_UNAVAILABLE"
    );
    assert_eq!(
        f.code(&[
            "array-linear",
            "--store",
            &dir,
            "--id",
            &id,
            "--count",
            "2",
            "--direction",
            "1,0,0",
            "--spacing",
            "3000",
        ]),
        "DEPENDENCY_UNAVAILABLE"
    );
    assert_eq!(
        f.code(&[
            "update-element",
            "--store",
            &dir,
            "--id",
            &id,
            "--mirror-point",
            "0,0,0",
            "--mirror-normal",
            "1,0,0",
        ]),
        "UNSUPPORTED_GEOMETRY"
    );
}

#[test]
fn a_linear_array_is_one_revision_and_one_undo_step() {
    let f = Fixture::new();
    let published = f.dir("array");
    std::fs::create_dir_all(&published).unwrap();
    let levels = f.ok(&[
        "create-level",
        "--store",
        &published,
        "--name",
        "L1",
        "--elevation",
        "4000",
    ]);
    let views = at(&levels, &["levels"]).array().unwrap().to_vec();
    let level = text(&views[0], "id");
    let created = f.ok(&[
        "create-plate",
        "--store",
        &published,
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
    ]);
    let id = text(at(&created, &["plate"]), "element_id");
    // The level assignment (D2) is an update, and the array's copies inherit it.
    f.ok(&[
        "update-element",
        "--store",
        &published,
        "--id",
        &id,
        "--level",
        &level,
    ]);
    let dir = f.dir("array-ws");
    f.ok(&[
        "create-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &dir,
        "--user-id",
        USER,
    ]);
    let arrayed = f.edit(
        &dir,
        &[
            "array-linear",
            "--id",
            &id,
            "--count",
            "3",
            "--direction",
            "1,0,0",
            "--spacing",
            "1500",
        ],
    );
    assert_eq!(text(&arrayed, "outcome"), "element_arrayed");
    // Three copies at the stepped placements.
    let elements = at(&arrayed, &["elements"]).array().unwrap();
    assert_eq!(elements.len(), 3);
    for (index, element) in elements.iter().enumerate() {
        let moved = translation(element);
        let expected = 1500.0 * (index as f64 + 1.0);
        assert!((moved[0] - expected).abs() < 1e-6, "{moved:?}");
        // The copies share the source's body and quantities (1000 x 500 x 10 mm).
        assert_eq!(num(element, &["quantities", "volume", "mm3"]), 5_000_000.0);
    }
    // The level link follows every copy.
    let levels = f.edit(&dir, &["levels"]);
    let views = at(&levels, &["levels"]).array().unwrap();
    let assigned = at(&views[0], &["elements"]).array().unwrap();
    assert_eq!(
        assigned.len(),
        4,
        "the source and three copies on the level"
    );
    // One undo step removes every copy.
    f.undo(&dir);
    let plates = f.plates(&dir);
    assert_eq!(plates.len(), 1);
    assert_eq!(text(&plates[0], "element_id"), id);
}

#[test]
fn a_polar_array_rotates_each_copy_about_the_axis() {
    let f = Fixture::new();
    let (_published, dir, id) = f.forked("polar");
    let arrayed = f.edit(
        &dir,
        &[
            "array-polar",
            "--id",
            &id,
            "--count",
            "4",
            "--point",
            "0,0,0",
            "--axis",
            "0,0,1",
            "--angle",
            QUARTER,
        ],
    );
    let elements = at(&arrayed, &["elements"]).array().unwrap();
    assert_eq!(elements.len(), 4);
    // The source sits at (1000, 0, 0): the copies march a quarter turn each, and the fourth is
    // a full turn back at the start.
    let expected = [
        ([0.0, 1000.0], 1.0),
        ([-1000.0, 0.0], 2.0),
        ([0.0, -1000.0], 3.0),
        ([1000.0, 0.0], 4.0),
    ];
    for (element, (place, _)) in elements.iter().zip(expected) {
        let moved = translation(element);
        assert!((moved[0] - place[0]).abs() < 1e-6, "{moved:?}");
        assert!((moved[1] - place[1]).abs() < 1e-6, "{moved:?}");
    }
    // A zero angle and a zero direction refuse before anything is written.
    assert_eq!(
        f.code(&[
            "array-polar",
            "--store",
            &dir,
            "--id",
            &id,
            "--count",
            "2",
            "--point",
            "0,0,0",
            "--axis",
            "0,0,1",
            "--angle",
            "0",
        ]),
        "LIMIT_EXCEEDED"
    );
    assert_eq!(
        f.code(&[
            "array-linear",
            "--store",
            &dir,
            "--id",
            &id,
            "--count",
            "2",
            "--direction",
            "0,0,0",
            "--spacing",
            "100",
        ]),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn two_point_moves_and_copies_are_the_same_operations_as_vectors() {
    let f = Fixture::new();
    let (_published, dir, id) = f.forked("twopoint");
    // Move by two points equals the equivalent absolute translation.
    let moved = f.edit(
        &dir,
        &[
            "update-element",
            "--id",
            &id,
            "--move-from",
            "0,0,0",
            "--move-to",
            "100,200,300",
        ],
    );
    assert_eq!(
        translation(at(&moved, &["element"])),
        vec![1100.0, 200.0, 300.0]
    );
    // The two-point form is exclusive with the explicit vector form.
    assert_eq!(
        f.code(&[
            "update-element",
            "--store",
            &dir,
            "--id",
            &id,
            "--move-from",
            "0,0,0",
            "--move-to",
            "1,0,0",
            "--translation",
            "5,0,0",
        ]),
        "INVALID_ARGUMENT"
    );
    // Copy by two points offsets the copy by the point difference.
    let copied = f.edit(
        &dir,
        &[
            "copy-element",
            "--id",
            &id,
            "--copy-from",
            "0,0,0",
            "--copy-to",
            "0,0,500",
        ],
    );
    assert_eq!(text(&copied, "outcome"), "element_copied");
    let copy_translation = translation(at(&copied, &["element"]));
    assert_eq!(copy_translation, vec![1100.0, 200.0, 800.0]);
    assert_eq!(
        f.code(&[
            "copy-element",
            "--store",
            &dir,
            "--id",
            &id,
            "--copy-from",
            "0,0,0",
            "--copy-to",
            "1,0,0",
            "--offset",
            "0,0,1",
        ]),
        "INVALID_ARGUMENT"
    );
}
