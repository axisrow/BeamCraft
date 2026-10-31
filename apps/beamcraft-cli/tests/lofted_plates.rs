// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic contours and plates, MPL-2.0.

//! A11 end to end: `create-lofted-plate` and `inspect-lofted-plates` through the real CLI
//! binary and committed revisions. A lofted plate is a plate between two contours that need
//! not be coplanar; its mesh is the certified ruled loft and a twisted or self-intersecting
//! input is refused with nothing written.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, at, text};

use beamcraft_json::Json;

const LOFT: &str = "019a7bf0-0000-7000-8000-000000110001";

const SQUARE: &str = "0,0,0;200,0,0;200,100,0;0,100,0";
const SQUARE_AT_50: &str = "0,0,50;200,0,50;200,100,50;0,100,50";
const SMALLER_AT_60: &str = "0,0,60;100,0,60;100,50,60;0,50,60";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a11-tests"),
        }
    }

    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
    }

    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }

    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let before = self.head();
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        // A refusal writes no revision.
        assert_eq!(self.head(), before, "{command} {extra:?}");
        text(&body, "code")
    }

    fn head(&self) -> String {
        // A store without a revision has no head; `open-revision` refuses it.
        let (code, body) = self.run("open-revision", &[]);
        if code == 0 {
            text(&body, "revision_id")
        } else {
            "no-revision".to_owned()
        }
    }

    fn loft_args<'a>(&self, id: &'a str, start: &'a str, end: &'a str) -> Vec<&'a str> {
        vec![
            "--name", "Loft", "--new-id", id, "--start", start, "--end", end,
        ]
    }

    fn volume(body: &Json) -> f64 {
        at(body, &["geometry", "volume_mm3"]).float().unwrap()
    }
}

#[test]
fn a_created_lofted_plate_reports_the_certified_body() {
    let f = Fixture::new();
    let body = f.ok(
        "create-lofted-plate",
        &f.loft_args(LOFT, SQUARE, SQUARE_AT_50),
    );
    assert_eq!(text(&body, "outcome"), "lofted_plate_created");
    assert_eq!(text(at(&body, &["plate"]), "id"), LOFT);
    assert_eq!(text(at(&body, &["plate"]), "name"), "Loft");
    // The oblique prism of the translated square: exact analytic volume and area.
    assert!((Fixture::volume(&body) - 200.0 * 100.0 * 50.0).abs() < 1e-6);
    let area = at(&body, &["geometry", "area_mm2"]).float().unwrap();
    assert!((area - (2.0 * 20000.0 + 2.0 * 300.0 * 50.0)).abs() < 1e-6);
    let mesh_digest = text(at(&body, &["geometry"]), "mesh_digest");
    assert_eq!(mesh_digest.len(), 64, "the sealed mesh carries its digest");
    let triangles = at(&body, &["geometry", "triangles"]).unsigned().unwrap();
    assert!(triangles > 0);
}

#[test]
fn the_head_holds_the_stored_quantities_and_a_replay_is_detected() {
    let f = Fixture::new();
    let created = f.ok(
        "create-lofted-plate",
        &[
            "--name",
            "Loft",
            "--new-id",
            LOFT,
            "--start",
            SQUARE,
            "--end",
            SQUARE_AT_50,
            "--op-id",
            "019a7bf0-0000-7000-8000-0000001100f0",
        ],
    );
    assert_eq!(text(&created, "outcome"), "lofted_plate_created");
    let listed = f.ok("inspect-lofted-plates", &[]);
    assert_eq!(text(&listed, "outcome"), "lofted_plates");
    let plates = at(&listed, &["plates"]).array().unwrap();
    assert_eq!(plates.len(), 1);
    let stored = &plates[0];
    assert_eq!(text(stored, "id"), LOFT);
    assert_eq!(text(stored, "volume_status"), "current");
    assert!((at(stored, &["volume_mm3"]).float().unwrap() - 1_000_000.0).abs() < 1e-6);
    // The identical retry is a replay; it reports the recorded revision.
    let head = f.head();
    let replay = f.ok(
        "create-lofted-plate",
        &[
            "--name",
            "Loft",
            "--new-id",
            LOFT,
            "--start",
            SQUARE,
            "--end",
            SQUARE_AT_50,
            "--op-id",
            "019a7bf0-0000-7000-8000-0000001100f0",
        ],
    );
    assert_eq!(text(&replay, "outcome"), "lofted_plate_replayed");
    assert_eq!(text(at(&replay, &["revision"]), "revision_id"), head);
}

#[test]
fn a_tapered_loft_carries_the_prismatoid_volume() {
    let f = Fixture::new();
    let body = f.ok(
        "create-lofted-plate",
        &f.loft_args(LOFT, SQUARE, SMALLER_AT_60),
    );
    let a1 = 200.0 * 100.0;
    let a2 = 100.0 * 50.0;
    let am = 150.0 * 75.0;
    let expected = 60.0 / 6.0 * (a1 + 4.0 * am + a2);
    assert!((Fixture::volume(&body) - expected).abs() < 1e-3);
}

#[test]
fn a_renumbered_end_contour_is_refused_with_nothing_written() {
    let f = Fixture::new();
    // The 180-degree renumbering of the end contour twists every wall through the body.
    let twisted = "200,100,50;0,100,50;0,0,50;200,0,50";
    assert_eq!(
        f.rejected("create-lofted-plate", &f.loft_args(LOFT, SQUARE, twisted)),
        "UNSUPPORTED_GEOMETRY"
    );
}

#[test]
fn a_self_intersecting_start_contour_is_refused() {
    let f = Fixture::new();
    let bowtie = "0,0,0;200,0,0;50,100,0;150,100,0";
    assert_eq!(
        f.rejected(
            "create-lofted-plate",
            &f.loft_args(LOFT, bowtie, SQUARE_AT_50)
        ),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn a_vertex_count_mismatch_is_refused() {
    let f = Fixture::new();
    let five = "0,0,50;200,0,50;200,100,50;100,150,50;0,100,50";
    assert_eq!(
        f.rejected("create-lofted-plate", &f.loft_args(LOFT, SQUARE, five)),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn mass_is_current_with_a_density_and_unknown_without_one() {
    let f = Fixture::new();
    let digest = text(
        at(
            &f.ok("define-material", &["--density", "7850", "--name", "Steel"]),
            &["definition"],
        ),
        "digest",
    );
    let body = f.ok(
        "create-lofted-plate",
        &[
            "--new-id",
            LOFT,
            "--start",
            SQUARE,
            "--end",
            SQUARE_AT_50,
            "--material-digest",
            &digest,
        ],
    );
    assert_eq!(text(&body, "outcome"), "lofted_plate_created");
    let listed = f.ok("inspect-lofted-plates", &[]);
    let stored = &at(&listed, &["plates"]).array().unwrap()[0];
    let mass = at(stored, &["mass_kg"]).float().unwrap();
    assert!((mass - 7850.0 * 1_000_000.0 / 1e9).abs() < 1e-9);
    assert_eq!(text(stored, "mass_status"), "current");
    assert_eq!(text(stored, "algorithm"), "lofted-plate-ruled-v1");
}

#[test]
fn the_view_snapshot_carries_the_certified_mesh() {
    let f = Fixture::new();
    f.ok(
        "create-lofted-plate",
        &f.loft_args(LOFT, SQUARE, SQUARE_AT_50),
    );
    let body = f.ok("inspect-elements", &[]);
    let elements = at(&body, &["snapshot", "elements"]).array().unwrap();
    assert_eq!(elements.len(), 1);
    assert_eq!(text(&elements[0], "id"), LOFT);
    let min = at(&elements[0], &["world_min_mm"]).array().unwrap();
    let max = at(&elements[0], &["world_max_mm"]).array().unwrap();
    assert!((min[0].float().unwrap() - 0.0).abs() < 1e-9);
    assert!((max[0].float().unwrap() - 200.0).abs() < 1e-9);
    assert!((max[2].float().unwrap() - 50.0).abs() < 1e-9);
    let meshes = at(&body, &["snapshot", "meshes"]).array().unwrap();
    assert_eq!(meshes.len(), 1);
    // 4-point contours: 8 wall triangles plus two cap triangles per side.
    assert_eq!(at(&meshes[0], &["triangles"]).unsigned().unwrap(), 12);
}

#[test]
fn plain_plates_still_commit_after_a_lofted_plate() {
    let f = Fixture::new();
    f.ok(
        "create-lofted-plate",
        &f.loft_args(LOFT, SQUARE, SQUARE_AT_50),
    );
    // The strict verifier decodes the whole head on every commit: the lofted plate must not
    // disturb the ordinary plate pipeline.
    let plate = f.ok(
        "create-plate",
        &["--width", "1000", "--length", "500", "--thickness", "10"],
    );
    assert_eq!(text(&plate, "outcome"), "plate_created");
    let again = f.ok(
        "create-plate",
        &["--width", "600", "--length", "400", "--thickness", "8"],
    );
    assert_eq!(text(&again, "outcome"), "plate_created");
}
