// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected volumes are closed forms: the box minus
// the polygon areas, times the thickness. The saved layer text is read with plain string matching
// and no BeamCraft crate.

//! A04 through holes end to end through the real `beamcraft` binary: `create-plate
//! --hole-rect/--hole-round` authors the hole polygons in the plate, the kernel cuts them, and
//! fresh processes reopen and re-verify the result.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_json::Json;
use std::fs;

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a04-tests"),
        }
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
    }
    fn plate(&self, extra: &[&str]) -> (i32, Json) {
        let mut args = vec!["--width", "1000", "--length", "500", "--thickness", "10"];
        args.extend_from_slice(extra);
        self.run("create-plate", &args)
    }
    fn ok_plate(&self, extra: &[&str]) -> Json {
        let (code, body) = self.plate(extra);
        assert_eq!(code, 0, "{extra:?}: {body:?}");
        body
    }
    fn rejected(&self, extra: &[&str]) -> String {
        let (code, body) = self.plate(extra);
        assert_eq!(code, 2, "{extra:?}");
        text(&body, "code")
    }
    fn authored_text(&self, revision: &Json) -> String {
        let layers = at(revision, &["revision", "manifest", "layers"])
            .array()
            .unwrap();
        let layer = layers
            .iter()
            .find(|l| text(l, "name") == "authored.usda")
            .unwrap();
        let digest = text(layer, "digest");
        let path = self
            .shared
            .root
            .join("store")
            .join("blobs")
            .join(&digest[..2])
            .join(&digest);
        String::from_utf8(fs::read(path).unwrap()).unwrap()
    }
}
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

#[test]
fn rectangular_holes_give_the_exact_volume_and_mass_and_reopen_verified() {
    let f = Fixture::new();
    let steel = {
        let (_, reply) = f.run("define-material", &["--density", "7850"]);
        text(reply.get("definition").unwrap(), "digest")
    };
    let created = f.ok_plate(&[
        "--hole-rect",
        "100,100,100,100;400,200,50,50",
        "--material-digest",
        &steel,
    ]);
    let plate = at(&created, &["plate"]);
    assert_eq!(num(plate, &["hole_count"]), 2.0);
    // (1000 * 500 - 100 * 100 - 50 * 50) * 10 = 4,875,000 mm3 = 0.004875 m3; * 7850 = 38.26875 kg.
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!((volume - 4_875_000.0).abs() <= 1e-6, "{volume}");
    let mass = num(plate, &["quantities", "mass", "kg"]);
    assert!((mass - 38.268_75).abs() <= 1e-9, "{mass}");
    assert!(num(plate, &["triangles"]) > 12.0);

    let inspected = f.run("inspect-elements", &[]).1;
    let reopened = &at(&inspected, &["plates"]).array().unwrap()[0];
    assert_eq!(text(reopened, "mesh_digest"), text(plate, "mesh_digest"));
    assert_eq!(num(reopened, &["hole_count"]), 2.0);
    assert_eq!(
        num(reopened, &["quantities", "volume", "mm3"]),
        num(plate, &["quantities", "volume", "mm3"])
    );

    // An ordinary reader of the saved text sees the polygons as plain arrays.
    let authored = f.authored_text(&created);
    assert!(authored.contains("custom int[] steel:plate:holeVertexCounts = [4, 4]"));
    assert!(authored.contains("custom double3[] steel:plate:holePointsMm = [(100.0, 100.0, 0.0), (100.0, 200.0, 0.0), (200.0, 200.0, 0.0), (200.0, 100.0, 0.0), (400.0, 200.0, 0.0), (400.0, 250.0, 0.0), (450.0, 250.0, 0.0), (450.0, 200.0, 0.0)]"), "{authored}");
}

#[test]
fn a_round_hole_is_a_clockwise_polygon_with_the_polygon_volume() {
    let f = Fixture::new();
    let created = f.ok_plate(&["--hole-round", "250,250,100", "--hole-segments", "64"]);
    let plate = at(&created, &["plate"]);
    // Regular 64-gon inscribed in a circle of radius 50: (64 / 2) * r^2 * sin(2 pi / 64).
    let area = 32.0 * 50.0 * 50.0 * (std::f64::consts::TAU / 64.0).sin();
    let expected = (500_000.0 - area) * 10.0;
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!((volume - expected).abs() <= 1e-4, "{volume} vs {expected}");
    // The model keeps the polygon, not a circle: 64 authored points, no curve and no radius.
    assert_eq!(
        at(plate, &["holes_mm"]).array().unwrap()[0]
            .array()
            .unwrap()
            .len(),
        64
    );
    // Without material the mass stays unknown.
    assert_eq!(
        text(at(plate, &["quantities", "mass"]), "status"),
        "unknown"
    );
    // The first point is on the circle and the order is clockwise (the signed area is negative).
    let points: Vec<(f64, f64)> = at(plate, &["holes_mm"]).array().unwrap()[0]
        .array()
        .unwrap()
        .iter()
        .map(|p| {
            let p = p.array().unwrap();
            (p[0].float().unwrap(), p[1].float().unwrap())
        })
        .collect();
    let signed: f64 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|((x0, y0), (x1, y1))| x0 * y1 - x1 * y0)
        .sum::<f64>()
        / 2.0;
    assert!(signed < 0.0, "{signed}");
    assert!((signed.abs() - area).abs() < 1e-6);
}

#[test]
fn a_solid_plate_authors_no_hole_attributes_and_holes_join_a_beam_in_one_scene() {
    let f = Fixture::new();
    let solid = f.ok_plate(&[]);
    assert!(!f.authored_text(&solid).contains("holePointsMm"));
    assert_eq!(num(&solid, &["plate", "hole_count"]), 0.0);
    // A plate with a hole and a beam together, in one pinned snapshot.
    let profile = {
        let (_, reply) = f.run("define-profile", &["--width", "200", "--height", "300"]);
        text(reply.get("definition").unwrap(), "digest")
    };
    let holed = f.ok_plate(&[
        "--hole-rect",
        "100,100,100,100",
        "--translation",
        "0,1000,0",
    ]);
    let (code, beam) = f.run(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            &profile,
        ],
    );
    assert_eq!(code, 0, "{beam:?}");
    let inspected = f.run("inspect-elements", &[]).1;
    assert_eq!(num(&inspected, &["plate_count"]), 2.0);
    assert_eq!(num(&inspected, &["member_count"]), 1.0);
    let elements = at(&inspected, &["snapshot", "elements"]).array().unwrap();
    assert_eq!(elements.len(), 3);
    let holed_id = text(at(&holed, &["plate"]), "element_id");
    let entry = elements.iter().find(|e| text(e, "id") == holed_id).unwrap();
    let corner = |key: &str| -> Vec<f64> {
        at(entry, &[key])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect()
    };
    // The bounds of a holed plate are those of its outline, moved by the placement.
    assert_eq!(corner("world_min_mm"), vec![0.0, 1000.0, 0.0]);
    assert_eq!(corner("world_max_mm"), vec![1000.0, 1500.0, 10.0]);
}

#[test]
fn hole_rejections_use_the_f04_codes_and_publish_nothing() {
    let f = Fixture::new();
    // Not strictly inside the outline.
    assert_eq!(
        f.rejected(&["--hole-rect", "2000,100,100,100"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(&["--hole-rect", "950,100,100,100"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(&["--hole-rect", "0,100,100,100"]),
        "INVALID_ARGUMENT"
    );
    // Overlapping holes.
    assert_eq!(
        f.rejected(&["--hole-rect", "100,100,100,100;150,150,100,100"]),
        "INVALID_ARGUMENT"
    );
    // A mirrored rectangle has the wrong winding: width and height must be positive.
    assert_eq!(
        f.rejected(&["--hole-rect", "100,100,-100,100"]),
        "INVALID_ARGUMENT"
    );
    // Two negative dimensions give a valid clockwise polygon, so the CLI must refuse them itself.
    assert_eq!(
        f.rejected(&["--hole-rect", "200,200,-100,-100"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(&["--hole-round", "250,250,-100"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(&["--hole-round", "250,250,0"]),
        "INVALID_ARGUMENT"
    );
    // Malformed generators.
    assert_eq!(f.rejected(&["--hole-rect", "1,2,3"]), "INVALID_ARGUMENT");
    assert_eq!(f.rejected(&["--hole-round", "1,2,x"]), "INVALID_ARGUMENT");
    assert_eq!(
        f.rejected(&["--hole-round", "250,250,100", "--hole-segments", "4"]),
        "INVALID_ARGUMENT"
    );
    // Non-finite and out-of-cap values.
    assert_eq!(
        f.rejected(&["--hole-rect", "NaN,100,100,100"]),
        "NON_FINITE_NUMBER"
    );
    assert_eq!(
        f.rejected(&["--hole-rect", "2e9,100,100,100"]),
        "LIMIT_EXCEEDED"
    );
    // More holes than the cap.
    let many = (0..65)
        .map(|k| format!("{},100,5,5", 10 + k * 10))
        .collect::<Vec<_>>()
        .join(";");
    assert_eq!(f.rejected(&["--hole-rect", &many]), "LIMIT_EXCEEDED");
    // Nothing was created: there is no revision at all.
    let (code, body) = f.run("inspect-elements", &[]);
    assert_eq!(code, 2, "{body:?}");
}

#[test]
fn a_retry_with_the_same_holes_replays_and_other_holes_are_reuse() {
    let f = Fixture::new();
    let ids = [
        "--new-id",
        "01a00000-0000-7000-8000-000000000021",
        "--op-id",
        "01a00000-0000-7000-8000-000000000022",
    ];
    let create = |holes: &str| {
        let mut args = vec!["--hole-rect", holes];
        args.extend_from_slice(&ids);
        f.plate(&args)
    };
    let (code, first) = create("100,100,100,100");
    assert_eq!(code, 0);
    assert_eq!(text(&first, "outcome"), "plate_created");
    let (code, replay) = create("100,100,100,100");
    assert_eq!(code, 0);
    assert_eq!(text(&replay, "outcome"), "plate_replayed");
    assert_eq!(
        text(at(&replay, &["plate"]), "mesh_digest"),
        text(at(&first, &["plate"]), "mesh_digest")
    );
    let (code, reuse) = create("100,100,120,100");
    assert_eq!(code, 2);
    assert_eq!(text(&reuse, "code"), "OP_ID_REUSE");
}
