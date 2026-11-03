// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected volumes are closed forms. The saved
// layer text is read with plain string matching and no BeamCraft crate.

//! U12 representation levels end to end through the real `beamcraft` binary: the derived layer
//! authors the render purpose on the full body, the reduced level (`default`, cuts kept, holes
//! dropped) and the simplified level (`proxy`, no operations) where their bodies differ, and a
//! member's axis as a `guide` curve. Every level except the full one carries `bim:exact = false`,
//! no level carries a quantity record, and engineering quantities stay the full body's.

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
            shared: support::StoreFixture::new("u12-tests"),
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
    fn plate(&self, extra: &[&str]) -> Json {
        let mut args = vec!["--width", "1000", "--length", "500", "--thickness", "10"];
        args.extend_from_slice(extra);
        self.ok("create-plate", &args)
    }
    fn derived_text(&self, revision: &Json) -> String {
        let layers = at(revision, &["revision", "manifest", "layers"])
            .array()
            .unwrap();
        let layer = layers
            .iter()
            .find(|l| text(l, "name") == "derived.usda")
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

#[test]
fn a_holed_plate_authors_the_reduced_level_and_marks_it_inexact() {
    let f = Fixture::new();
    let created = f.plate(&["--hole-rect", "400,200,50,50"]);
    let derived = f.derived_text(&created);
    // The full body is the render level and keeps the quantity record.
    assert!(
        derived.contains("custom token purpose = \"render\""),
        "{derived}"
    );
    assert!(derived.contains("custom double bim:volumeMm3 = 4975000.0"),);
    // The reduced level is the cuts-without-holes body: the plain box here, tagged default and
    // inexact, with no quantity record of its own.
    assert!(derived.contains("def Mesh \"Reduced\""), "{derived}");
    let reduced = derived
        .split("def Mesh \"Reduced\"")
        .nth(1)
        .unwrap()
        .split("def ")
        .next()
        .unwrap();
    assert!(reduced.contains("custom token purpose = \"default\""),);
    assert!(reduced.contains("custom bool bim:exact = false"),);
    assert!(
        !reduced.contains("bim:volume") && !reduced.contains("bim:mass"),
        "{reduced}"
    );
    // One hole and no cut: no simplified level and no axis (a plate has none).
    assert!(!derived.contains("def Mesh \"Proxy\""), "{derived}");
    assert!(!derived.contains("BasisCurves"), "{derived}");
}

#[test]
fn a_cut_on_a_holed_plate_authors_the_simplified_level_too() {
    let f = Fixture::new();
    let created = f.plate(&["--hole-rect", "400,200,50,50"]);
    let plate = at(&created, &["plate"]);
    let id = text(plate, "element_id");
    let cut = f.ok(
        "cut-by-plane",
        &[
            "--id",
            &id,
            "--operation-id",
            "01a1a0f5-9999-7000-8000-000000000001",
            "--point",
            "200,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    let derived = f.derived_text(&cut);
    assert!(derived.contains("def Mesh \"Reduced\""), "{derived}");
    assert!(derived.contains("def Mesh \"Proxy\""), "{derived}");
    let proxy = derived
        .split("def Mesh \"Proxy\"")
        .nth(1)
        .unwrap()
        .split("def ")
        .next()
        .unwrap();
    assert!(proxy.contains("custom token purpose = \"proxy\""),);
    assert!(proxy.contains("custom bool bim:exact = false"),);
    assert!(
        !proxy.contains("bim:volume") && !proxy.contains("bim:mass"),
        "{proxy}"
    );
    // Engineering reads the full body only: the quantity record stays on the render level and
    // holds the exact kept-half volume, 200 x 500 x 10 mm.
    assert!(
        derived.contains("custom double bim:volumeMm3 = 1000000.0"),
        "{derived}"
    );
    let inspected = f.run("inspect-elements", &[]).1;
    assert_eq!(
        at(
            &at(&inspected, &["plates"]).array().unwrap()[0],
            &["quantities", "volume", "mm3"]
        )
        .float()
        .unwrap(),
        1_000_000.0
    );
}

#[test]
fn a_plain_plate_authors_no_level_prims() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let derived = f.derived_text(&created);
    // No holes and no cuts: one body, the render level, and nothing else.
    assert_eq!(derived.matches("def Mesh").count(), 1, "{derived}");
    assert!(derived.contains("custom token purpose = \"render\""),);
    assert!(!derived.contains("def Mesh \"Reduced\""), "{derived}");
    assert!(!derived.contains("def Mesh \"Proxy\""), "{derived}");
    assert!(!derived.contains("BasisCurves"), "{derived}");
    assert!(!derived.contains("bim:exact"), "{derived}");
}

#[test]
fn a_member_authors_its_axis_as_a_guide_curve() {
    let f = Fixture::new();
    let defined = f.ok("define-profile", &["--width", "200", "--height", "300"]);
    let profile = at(&defined, &["definition"]);
    let created = f.ok(
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            &text(profile, "digest"),
        ],
    );
    let derived = f.derived_text(&created);
    // A plain member has one body and one guide: the axis polyline.
    assert_eq!(derived.matches("def Mesh").count(), 1, "{derived}");
    let axis = derived
        .split("def BasisCurves \"Axis\"")
        .nth(1)
        .expect("the member axis prim is authored")
        .split("def ")
        .next()
        .unwrap();
    assert!(axis.contains("point3f[] points = [(0.0, 0.0, 0.0), (6000.0, 0.0, 0.0)]"),);
    assert!(axis.contains("int[] curveVertexCounts = [2]"),);
    assert!(axis.contains("custom token purpose = \"guide\""),);
    assert!(axis.contains("custom bool bim:exact = false"),);
    assert!(!axis.contains("bim:volume"), "{axis}");
    // A member without holes and cuts authors no level bodies.
    assert!(!derived.contains("def Mesh \"Reduced\""), "{derived}");
    assert!(!derived.contains("def Mesh \"Proxy\""), "{derived}");
}

#[test]
fn reopening_a_leveled_store_reverifies_and_quantities_survive() {
    let f = Fixture::new();
    let created = f.plate(&["--hole-rect", "400,200,50,50"]);
    let plate = at(&created, &["plate"]);
    let inspected = f.run("inspect-elements", &[]).1;
    let reopened = &at(&inspected, &["plates"]).array().unwrap()[0];
    assert_eq!(text(reopened, "mesh_digest"), text(plate, "mesh_digest"));
    assert_eq!(
        at(reopened, &["quantities", "volume", "mm3"])
            .float()
            .unwrap(),
        4_975_000.0
    );
}
