// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic contours, MPL-2.0. Expected volumes and masses are closed forms
// (polygon area * thickness, volume * density), never production math.

//! A02.1 end to end through the real `beamcraft` binary: a plate with any simple counter-clockwise
//! contour in the plane `z = 0` is created, saved, reopened, edited, copied, numbered and exported
//! like a rectangular plate, and a wrong contour is refused with nothing authored.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use support::{at, text};

use beamcraft_json::Json;
use std::fs;
use std::path::Path;

/// 1000 x 600 with a 600 x 300 notch at the top right: area 600_000 - 180_000 = 420_000 mm2.
const L_SHAPE: &str = "0,0;1000,0;1000,300;400,300;400,600;0,600";
/// The same L mirrored about the diagonal: same bounding box, another outline.
const L_MIRRORED: &str = "0,0;1000,0;1000,600;600,600;600,300;0,300";
const L_AREA_MM2: f64 = 420_000.0;

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a02-tests"),
        }
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn raw(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.raw(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let (code, body) = self.raw(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        assert_eq!(text(&body, "outcome"), "rejected");
        text(&body, "code")
    }
    fn plates(&self) -> Vec<Json> {
        let inspected = self.ok("inspect-elements", &[]);
        at(&inspected, &["plates"]).array().unwrap().to_vec()
    }
}
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

#[test]
fn a_valid_near_planar_plate_is_saved_without_partial_geometry_and_replays() {
    let f = Fixture::new();
    let args = [
        "--contour",
        "999000000,0,0;999393216,0,0.75;999393216,524288,0;999000000,524288,0.75",
        "--thickness",
        "10",
        "--translation",
        "0,1000000000,0",
        "--new-id",
        "019a7bf0-0000-7000-8000-000000000005",
        "--op-id",
        "019a7bf0-0000-7000-8000-000000000004",
    ];
    let created = f.ok("create-plate", &args);
    let plate = at(&created, &["plate"]);
    assert_eq!(text(plate, "validity"), "valid");
    assert_eq!(text(plate, "geometry_status"), "unsupported");
    assert_eq!(text(plate, "geometry_code"), "UNSUPPORTED_GEOMETRY");
    for key in ["mesh_digest", "shape_key", "world_bounds_mm"] {
        assert_eq!(plate.get(key).unwrap(), &Json::Null);
    }
    for (quantity, unit) in [("volume", "mm3"), ("mass", "kg")] {
        assert_eq!(at(plate, &["quantities", quantity, unit]), &Json::Null);
        assert_eq!(
            text(at(plate, &["quantities", quantity]), "status"),
            "unsupported"
        );
    }
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(num(&inspected, &["plate_count"]), 1.);
    assert_eq!(text(at(&inspected, &["snapshot"]), "pin_generation"), "1");
    let reopened = &at(&inspected, &["plates"]).array().unwrap()[0];
    for key in [
        "contour_points_mm",
        "placement",
        "geometry_status",
        "quantities",
        "mesh_digest",
    ] {
        assert_eq!(reopened.get(key).unwrap(), plate.get(key).unwrap());
    }
    assert!(
        at(&inspected, &["snapshot", "meshes"])
            .array()
            .unwrap()
            .is_empty()
    );
    let entries = at(&inspected, &["snapshot", "elements"]).array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].get("mesh_digest").unwrap(), &Json::Null);
    let before = f.ok("open-revision", &[]);
    let replay = f.ok("create-plate", &args);
    assert_eq!(text(&replay, "outcome"), "plate_replayed");
    assert_eq!(f.ok("open-revision", &[]), before);
    assert_eq!(
        f.rejected("export-view", &["--out", &f.path("unsupported.bundle")]),
        "DEPENDENCY_UNAVAILABLE"
    );
    assert!(!Path::new(&f.path("unsupported.bundle")).exists());
    assert_eq!(
        f.rejected("preview-numbering", &[]),
        "DEPENDENCY_UNAVAILABLE"
    );
    assert_eq!(
        f.rejected("export-bom", &["--out", &f.path("unsupported.csv")]),
        "DEPENDENCY_UNAVAILABLE"
    );
    assert!(!Path::new(&f.path("unsupported.csv")).exists());
    assert_eq!(
        f.rejected("export-ifc", &["--out", &f.path("unsupported.ifc")]),
        "DEPENDENCY_UNAVAILABLE"
    );
    assert!(!Path::new(&f.path("unsupported.ifc")).exists());
    f.ok(
        "create-plate",
        &["--width", "1000", "--length", "500", "--thickness", "10"],
    );
    let mixed = f.ok("inspect-elements", &[]);
    assert_eq!(num(&mixed, &["plate_count"]), 2.);
    assert_eq!(
        at(&mixed, &["snapshot", "meshes"]).array().unwrap().len(),
        1
    );
    f.ok(
        "delete-element",
        &["--id", "019a7bf0-0000-7000-8000-000000000005"],
    );
    assert_eq!(f.plates().len(), 1);
}

#[test]
fn a_rectangle_in_its_3d_body_frame_is_saved_and_reopened() {
    let f = Fixture::new();
    let created = f.ok(
        "create-plate",
        &[
            "--contour",
            "10,20,30;10,1020,30;-490,1020,30;-490,20,30",
            "--thickness",
            "10",
        ],
    );
    let plate = at(&created, &["plate"]);
    assert_eq!(num(plate, &["dimensions_mm", "width"]), 1000.);
    assert_eq!(num(plate, &["dimensions_mm", "length"]), 500.);
    assert_eq!(num(plate, &["quantities", "volume", "mm3"]), 5_000_000.);
    let reopened = f.plates();
    assert_eq!(reopened.len(), 1);
    assert_eq!(
        at(&reopened[0], &["contour_points_mm"]),
        at(plate, &["contour_points_mm"])
    );
    assert_eq!(
        text(&reopened[0], "mesh_digest"),
        text(plate, "mesh_digest")
    );
    let before = f.ok("open-revision", &[]);
    assert_eq!(
        f.rejected(
            "create-plate",
            &[
                "--contour",
                "0,0,0;1000,0,0;1000,500,20;0,500,0",
                "--thickness",
                "10"
            ]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.ok("open-revision", &[]), before);
    assert_eq!(
        f.rejected(
            "create-plate",
            &[
                "--contour",
                "999000000,0,0;999393216,0,0.875;999393216,524288,0;999000000,524288,0.875",
                "--thickness",
                "10"
            ]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.ok("open-revision", &[]), before);
    assert_eq!(
        f.rejected(
            "create-plate",
            &[
                "--contour",
                "10,20,30;10,1020,30;-490,1020,30;-490,20,30",
                "--thickness",
                "10",
                "--hole-rect",
                "0,0,1,1"
            ]
        ),
        "UNSUPPORTED_GEOMETRY"
    );
    assert_eq!(f.ok("open-revision", &[]), before);
}

#[test]
fn an_l_shaped_plate_is_saved_reopened_and_measured_by_closed_forms() {
    let f = Fixture::new();
    f.ok("define-material", &["--density", "7850", "--grade", "S355"]);
    let created = f.ok(
        "create-plate",
        &[
            "--contour",
            L_SHAPE,
            "--thickness",
            "10",
            "--material",
            "S355",
            "--name",
            "L plate",
        ],
    );
    assert_eq!(text(&created, "outcome"), "plate_created");
    let plate = at(&created, &["plate"]);
    assert_eq!(text(plate, "outline"), "contour");
    assert_eq!(num(plate, &["dimensions_mm", "width"]), 1000.0);
    assert_eq!(num(plate, &["dimensions_mm", "length"]), 600.0);
    let volume = L_AREA_MM2 * 10.0;
    assert_eq!(num(plate, &["quantities", "volume", "mm3"]), volume);
    // 4,200,000 mm3 = 0.0042 m3 of steel at 7850 kg/m3 = 32.97 kg.
    assert!((num(plate, &["quantities", "mass", "kg"]) - 32.97).abs() <= 1e-9);

    // A fresh process reads the committed revision back and re-evaluates the same plate.
    let plates = f.plates();
    assert_eq!(plates.len(), 1);
    let reopened = &plates[0];
    assert_eq!(text(reopened, "outline"), "contour");
    assert_eq!(
        at(reopened, &["contour_points_mm"]).array().unwrap().len(),
        6
    );
    assert_eq!(
        text(reopened, "mesh_digest"),
        text(plate, "mesh_digest"),
        "the re-evaluated mesh is the committed one"
    );
    assert_eq!(num(reopened, &["quantities", "volume", "mm3"]), volume);
    // 6 contour points are 12 vertices; 4 triangles per cap and 12 on the sides.
    assert_eq!(num(reopened, &["vertices"]), 12.0);
    assert_eq!(num(reopened, &["triangles"]), 20.0);
    f.ok("open-revision", &[]);
}

#[test]
fn the_same_contour_gives_the_same_mesh_and_a_rectangle_keeps_its_box() {
    let a = Fixture::new();
    let b = Fixture::new();
    for f in [&a, &b] {
        f.ok("create-plate", &["--contour", L_SHAPE, "--thickness", "10"]);
    }
    assert_eq!(
        text(&a.plates()[0], "mesh_digest"),
        text(&b.plates()[0], "mesh_digest")
    );
    // The origin rectangle is the same plate whichever way it is described: the G02 box.
    let by_size = Fixture::new();
    let by_contour = Fixture::new();
    by_size.ok(
        "create-plate",
        &["--width", "1000", "--length", "500", "--thickness", "10"],
    );
    by_contour.ok(
        "create-plate",
        &[
            "--contour",
            "0,0;1000,0;1000,500;0,500",
            "--thickness",
            "10",
        ],
    );
    let (x, y) = (&by_size.plates()[0], &by_contour.plates()[0]);
    assert_eq!(text(x, "outline"), "rectangle");
    assert_eq!(text(y, "outline"), "rectangle");
    assert_eq!(text(x, "mesh_digest"), text(y, "mesh_digest"));
    // A rectangle that does not start at the origin is a contour, evaluated as authored.
    let moved = Fixture::new();
    moved.ok(
        "create-plate",
        &[
            "--contour",
            "100,100;1100,100;1100,600;100,600",
            "--thickness",
            "10",
        ],
    );
    let plate = &moved.plates()[0];
    assert_eq!(text(plate, "outline"), "contour");
    assert_eq!(num(plate, &["quantities", "volume", "mm3"]), 5_000_000.0);
    assert_ne!(text(plate, "mesh_digest"), text(x, "mesh_digest"));
}

#[test]
fn a_contour_plate_takes_holes_edits_copies_and_deletes_like_any_plate() {
    let f = Fixture::new();
    let created = f.ok(
        "create-plate",
        &[
            "--contour",
            L_SHAPE,
            "--thickness",
            "10",
            "--hole-rect",
            "100,100,100,100",
        ],
    );
    let plate = at(&created, &["plate"]);
    let id = text(plate, "element_id");
    assert_eq!(num(plate, &["hole_count"]), 1.0);
    // The 100 x 100 hole removes 100,000 mm3 from 4,200,000 mm3.
    assert!((num(plate, &["quantities", "volume", "mm3"]) - 4_100_000.0).abs() <= 1e-3);
    // A hole through the notch leaves the plate, so nothing is created.
    assert_eq!(
        f.rejected(
            "create-plate",
            &[
                "--contour",
                L_SHAPE,
                "--thickness",
                "10",
                "--hole-rect",
                "350,250,100,150",
            ],
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.plates().len(), 1);

    // The thickness is edited without touching the outline.
    f.ok("update-element", &["--id", &id, "--thickness", "20"]);
    let edited = f.plates();
    assert_eq!(text(&edited[0], "outline"), "contour");
    assert!((num(&edited[0], &["quantities", "volume", "mm3"]) - 8_200_000.0).abs() <= 1e-3);
    // A move changes only the placement.
    f.ok(
        "update-element",
        &["--id", &id, "--translation", "5000,0,0"],
    );
    let moved = f.plates();
    assert_eq!(
        text(&moved[0], "mesh_digest"),
        text(&edited[0], "mesh_digest")
    );
    // A copy has the same body and an id of its own; deleting the source leaves it alone.
    f.ok(
        "copy-element",
        &["--id", &id, "--offset", "0,2000,0", "--name", "Copy"],
    );
    let both = f.plates();
    assert_eq!(both.len(), 2);
    assert_eq!(text(&both[0], "mesh_digest"), text(&both[1], "mesh_digest"));
    f.ok("delete-element", &["--id", &id]);
    let rest = f.plates();
    assert_eq!(rest.len(), 1);
    assert_eq!(text(&rest[0], "name"), "Copy");
}

#[test]
fn contour_plates_are_numbered_by_outline_and_exported_to_ifc() {
    let f = Fixture::new();
    for contour in [L_SHAPE, L_SHAPE, L_MIRRORED] {
        f.ok("create-plate", &["--contour", contour, "--thickness", "10"]);
    }
    f.ok(
        "create-plate",
        &["--width", "1000", "--length", "600", "--thickness", "10"],
    );
    // Same bounding box everywhere, three outlines: two identical Ls share a number.
    let preview = f.ok("preview-numbering", &[]);
    let groups = at(&preview, &["groups"]).array().unwrap();
    let mut sizes: Vec<f64> = groups.iter().map(|g| num(g, &["count"])).collect();
    sizes.sort_by(f64::total_cmp);
    assert_eq!(sizes, [1.0, 1.0, 2.0]);
    let descriptions: Vec<String> = groups.iter().map(|g| text(g, "description")).collect();
    assert_eq!(
        descriptions
            .iter()
            .filter(|d| d.contains("contour of 6 points"))
            .count(),
        2,
        "{descriptions:?}"
    );

    let out = f.path("model.ifc");
    let exported = f.ok("export-ifc", &["--out", &out]);
    assert_eq!(text(&exported, "outcome"), "exported");
    assert_eq!(num(&exported, &["counts", "exported"]), 4.0);
    assert_eq!(num(&exported, &["counts", "without_representation"]), 0.0);
    let step = fs::read_to_string(&out).unwrap();
    assert_eq!(step.matches("IFCPLATE(").count(), 4);
    assert!(step.contains("IFCTRIANGULATEDFACESET"));
}

#[test]
fn a_wrong_contour_is_refused_and_nothing_is_authored() {
    let f = Fixture::new();
    let refused =
        |contour: &str| f.rejected("create-plate", &["--contour", contour, "--thickness", "10"]);
    // Clockwise, self-crossing (F04-NEG-30), touching itself, doubling back, too few points.
    assert_eq!(
        refused("0,0;0,600;400,600;400,300;1000,300;1000,0"),
        "INVALID_ARGUMENT"
    );
    assert_eq!(refused("0,0;1000,500;0,500;800,0"), "INVALID_ARGUMENT");
    assert_eq!(
        refused("0,0;1000,0;1000,500;500,0;0,500"),
        "INVALID_ARGUMENT"
    );
    assert_eq!(refused("0,0;500,0;1000,0"), "INVALID_ARGUMENT");
    assert_eq!(refused("0,0;1000,0"), "INVALID_ARGUMENT");
    // Malformed text, a repeated point, an overflowing size, a thickness below the floor.
    assert_eq!(refused("0,0;1000,0;1000,x;0,500"), "INVALID_ARGUMENT");
    assert_eq!(refused("0,0;1000,0;1000"), "INVALID_ARGUMENT");
    assert_eq!(
        refused("0,0;1000,0;1000,0;1000,500;0,500"),
        "INVALID_ARGUMENT"
    );
    assert_eq!(refused("0,0;1e12,0;1e12,500;0,500"), "LIMIT_EXCEEDED");
    assert_eq!(refused("0,0;1000,0;1000,500;nan,500"), "NON_FINITE_NUMBER");
    assert_eq!(
        f.rejected("create-plate", &["--contour", L_SHAPE, "--thickness", "0"]),
        "INVALID_ARGUMENT"
    );
    // --contour replaces --width and --length.
    assert_eq!(
        f.rejected(
            "create-plate",
            &["--contour", L_SHAPE, "--width", "10", "--thickness", "10"]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "create-plate",
            &["--contour", L_SHAPE, "--length", "10", "--thickness", "10"]
        ),
        "INVALID_ARGUMENT"
    );
    // Neither a contour nor a size.
    assert_eq!(
        f.rejected("create-plate", &["--thickness", "10"]),
        "INVALID_ARGUMENT"
    );
    // Nothing was committed by any of the refusals.
    let (code, body) = f.raw("inspect-elements", &[]);
    assert!(
        code == 0 || text(&body, "outcome") == "rejected",
        "{body:?}"
    );
    if code == 0 {
        assert!(at(&body, &["plates"]).array().unwrap().is_empty());
    }
}

#[test]
fn holes_are_compared_relative_to_the_contour_not_to_the_element_frame() {
    let f = Fixture::new();
    // The same L, the second one 50 mm further along X.
    let shifted = "50,0;1050,0;1050,300;450,300;450,600;50,600";
    let hole_at = |x: &str| format!("{x},100,100,100");
    // Plate 1 and 2: the hole stays at x = 100 in the frame, so it is 100 mm and 50 mm from the
    // left edge of the two plates. Different parts.
    f.ok(
        "create-plate",
        &[
            "--contour",
            L_SHAPE,
            "--thickness",
            "10",
            "--hole-rect",
            &hole_at("100"),
        ],
    );
    f.ok(
        "create-plate",
        &[
            "--contour",
            shifted,
            "--thickness",
            "10",
            "--hole-rect",
            &hole_at("100"),
        ],
    );
    let preview = f.ok("preview-numbering", &[]);
    assert_eq!(at(&preview, &["groups"]).array().unwrap().len(), 2);
    // Plate 3: the hole moved by the same 50 mm, so it sits like the hole of plate 1. One group
    // with plate 1, and plate 2 stays alone.
    f.ok(
        "create-plate",
        &[
            "--contour",
            shifted,
            "--thickness",
            "10",
            "--hole-rect",
            &hole_at("150"),
        ],
    );
    let preview = f.ok("preview-numbering", &[]);
    let groups = at(&preview, &["groups"]).array().unwrap();
    let mut sizes: Vec<f64> = groups.iter().map(|g| num(g, &["count"])).collect();
    sizes.sort_by(f64::total_cmp);
    assert_eq!(sizes, [1.0, 2.0], "{preview:?}");
}
