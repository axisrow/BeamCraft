// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected volumes and masses are closed forms
// (width * length * thickness, volume * density).

//! P-Degenerate end to end (A-2 D-7): a wrongly built plate (a reversed contour) is kept as a
//! degenerate element without derived data, geometry-dependent operations refuse it honestly,
//! and once its contour is fixed the body and quantities are computed. An update that would
//! degrade an evaluated plate into a wrong one is refused with nothing written.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support::{self, at, text};

use beamcraft_json::Json;

/// The clockwise order of the origin rectangle: the D-7 example of a wrongly built element.
const REVERSED: &str = "0,0,0;0,500,0;1000,500,0;1000,0,0";
/// The counter-clockwise origin rectangle: the repaired contour.
const FIXED: &str = "0,0,0;1000,0,0;1000,500,0;0,500,0";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("p-degenerate-tests"),
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
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        text(&body, "code")
    }
    fn material(&self) -> String {
        text(
            at(
                &self.ok("define-material", &["--density", "7850"]),
                &["definition"],
            ),
            "digest",
        )
    }
    fn degenerate_plate(&self, material: &str) -> String {
        let created = self.ok(
            "create-plate",
            &[
                "--contour",
                REVERSED,
                "--thickness",
                "10",
                "--material-digest",
                material,
            ],
        );
        assert_eq!(text(&created, "outcome"), "plate_created");
        let plate = at(&created, &["plate"]);
        assert_eq!(text(plate, "validity"), "degenerate");
        assert_eq!(text(plate, "geometry_status"), "degenerate");
        assert_eq!(text(plate, "geometry_code"), "INVALID_ARGUMENT");
        assert_eq!(text(plate, "geometry_reason"), "contour_winding_clockwise");
        assert!(matches!(plate.get("mesh_digest"), Ok(Json::Null)));
        assert!(matches!(plate.get("world_bounds_mm"), Ok(Json::Null)));
        let volume = at(plate, &["quantities", "volume"]);
        assert!(matches!(volume.get("mm3"), Ok(Json::Null)));
        assert_eq!(text(volume, "status"), "degenerate");
        text(plate, "element_id")
    }
    fn inspected(&self) -> Json {
        self.ok("inspect-elements", &[])
    }
}

fn plate_of<'a>(inspection: &'a Json, id: &str) -> &'a Json {
    at(inspection, &["plates"])
        .array()
        .unwrap()
        .iter()
        .find(|p| text(p, "element_id") == id)
        .unwrap_or_else(|| panic!("plate {id} is not listed"))
}

fn changed_layers(update: &Json) -> Vec<String> {
    at(update, &["changed_layers"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect()
}

fn num_at(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

#[test]
fn a_wrongly_built_plate_is_kept_degenerate_and_repairs_once_fixed() {
    let f = Fixture::new();
    let steel = f.material();
    let id = f.degenerate_plate(&steel);

    // The inspection lists the degenerate plate with its inputs and no derived values.
    let inspection = f.inspected();
    let listed = plate_of(&inspection, &id);
    assert_eq!(text(listed, "geometry_status"), "degenerate");
    assert!(matches!(listed.get("mesh_digest"), Ok(Json::Null)));

    // A non-shape edit rewrites the authored layer alone and keeps the element degenerate.
    // The retry is the same request only under the same operation id.
    let rename_op = "019a7bf0-0000-7000-8000-0000000000c1";
    let renamed = f.ok(
        "update-element",
        &["--id", &id, "--name", "Wrong plate", "--op-id", rename_op],
    );
    assert_eq!(text(&renamed, "outcome"), "element_updated");
    assert_eq!(text(at(&renamed, &["element"]), "validity"), "degenerate");
    assert_eq!(changed_layers(&renamed), ["authored.usda"]);
    let replay = f.ok(
        "update-element",
        &["--id", &id, "--name", "Wrong plate", "--op-id", rename_op],
    );
    assert_eq!(text(&replay, "outcome"), "element_update_replayed");
    assert_eq!(text(at(&replay, &["element"]), "validity"), "degenerate");

    // The repair: a whole-contour replacement. The reclassified element now has a body, a
    // mesh, and stored quantities that read current. An evaluated plate carries no
    // degeneracy labels: its mesh digest is the proof of the evaluated class.
    let repaired = f.ok("update-element", &["--id", &id, "--contour", FIXED]);
    assert_eq!(text(&repaired, "outcome"), "element_updated");
    let element = at(&repaired, &["element"]);
    text(element, "mesh_digest");
    assert_eq!(
        num_at(element, &["quantities", "volume", "mm3"]),
        5_000_000.0
    );
    // (1000 * 500 * 10) mm3 * 7850 kg/m3 = 39.25 kg.
    assert_eq!(num_at(element, &["quantities", "mass", "kg"]), 39.25);
    assert_eq!(changed_layers(&repaired), ["authored.usda", "derived.usda"]);

    // A fresh read sees the repaired element with its derived data.
    let inspection = f.inspected();
    let listed = plate_of(&inspection, &id);
    text(listed, "mesh_digest");
    assert_eq!(
        num_at(listed, &["stored_quantities", "volume", "stored_value"]),
        5_000_000.0
    );
    assert_eq!(
        text(at(listed, &["stored_quantities", "volume"]), "freshness"),
        "current"
    );
}

#[test]
fn a_degrade_update_and_dependent_operations_refuse_without_writes() {
    let f = Fixture::new();
    let steel = f.material();
    let id = f.degenerate_plate(&steel);

    // Geometry-dependent operations refuse the degenerate target instead of misreporting it.
    assert_eq!(
        f.rejected(
            "add-hole",
            &[
                "--id",
                &id,
                "--operation-id",
                "019a7bf0-0000-7000-8000-0000000000b1",
                "--hole-rect",
                "100,100,100,100"
            ]
        ),
        "DEPENDENCY_UNAVAILABLE"
    );
    assert_eq!(
        f.rejected("copy-element", &["--id", &id]),
        "DEPENDENCY_UNAVAILABLE"
    );

    // The repair first, so the next update targets an evaluated element.
    f.ok("update-element", &["--id", &id, "--contour", FIXED]);

    // A patch that would degrade the evaluated element into a wrongly built one is refused:
    // the kernel refuses the result, and the head keeps its mesh and quantities.
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--contour", REVERSED]),
        "INVALID_ARGUMENT"
    );
    let inspection = f.inspected();
    let listed = plate_of(&inspection, &id);
    text(listed, "mesh_digest");
}

#[test]
fn a_contour_edit_can_repair_a_degenerate_contour() {
    let f = Fixture::new();
    // A zero signed area is retained degenerate; moving the last vertex back repairs the
    // contour into the origin rectangle through the ordinary contour edit.
    let created = f.ok(
        "create-plate",
        &[
            "--contour",
            "0,0,0;1000,0,0;1000,500,0;2000,500,0",
            "--thickness",
            "10",
        ],
    );
    let id = text(at(&created, &["plate"]), "element_id");
    assert_eq!(
        text(at(&created, &["plate"]), "geometry_reason"),
        "contour_area_vector_zero"
    );
    let repaired = f.ok(
        "move-plate-vertex",
        &["--id", &id, "--vertex", "3", "--to", "0,500"],
    );
    let element = at(&repaired, &["element"]);
    text(element, "mesh_digest");
    assert_eq!(
        num_at(element, &["quantities", "volume", "mm3"]),
        5_000_000.0
    );
}
