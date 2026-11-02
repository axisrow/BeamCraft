// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic sections, MPL-2.0. Expected values are closed forms (rectangles
// and quarter circles) and products of them; the section named "30B1-like" is an ordinary
// hot-rolled I section given by its dimensions, with no catalogue data. Layer text is read with
// plain string matching.

//! P-Mass end to end through the real `beamcraft` binary: a member's quantities are
//! `area x axis length x density`, where the area is the declared catalogue area when the profile
//! declares one and otherwise the exact computed area; gross is the nominal body, net the body
//! after the operations (through holes of a plate today), and a store written before this slice
//! reads as stale, never as corrupt.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_commands::plate::inspect;
use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;
use std::f64::consts::PI;

const SHIPPED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-sections.usda"
);

use crate::support;

use support::{at, text};

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("pmass-tests"),
        }
    }
    fn revisions(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.shared.root.join("store")).unwrap())
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
    fn profile(&self, extra: &[&str]) -> String {
        text(
            self.ok("define-profile", extra).get("definition").unwrap(),
            "digest",
        )
    }
    fn steel(&self) -> String {
        text(
            self.ok("define-material", &["--density", "7850"])
                .get("definition")
                .unwrap(),
            "digest",
        )
    }
    /// A 6000 mm beam; the quantities object of the created member.
    fn beam(&self, profile: &str, material: Option<&str>) -> Json {
        let mut args = vec![
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            profile,
        ];
        if let Some(material) = material {
            args.extend_from_slice(&["--material-digest", material]);
        }
        let created = self.ok("create-member", &args);
        at(&created, &["member"]).clone()
    }
    /// Commits a head-based revision with changed layer text and no content verification, the way
    /// a store written by an earlier binary (or edited outside BeamCraft) would arrive.
    fn tamper(&self, edit: impl Fn(&str, String) -> String) {
        let store = self.revisions();
        let head = store.open().unwrap();
        let content = head.manifest().content();
        let mut ids = IdSource::system();
        let mut entries = Vec::new();
        for entry in content.entries() {
            let original = String::from_utf8(head.layer_bytes(entry.name()).unwrap()).unwrap();
            let edited = edit(entry.name(), original);
            store.blobs().publish(edited.as_bytes()).unwrap();
            entries.push(
                LayerEntry::from_bytes(entry.name(), entry.role(), edited.as_bytes()).unwrap(),
            );
        }
        let draft = RevisionDraft {
            project: content.project().clone(),
            workspace: content.workspace().clone(),
            revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            workspace_generation: content.workspace_generation() + 1,
            schema: content.schema(),
            root_layer: content.root_layer().to_owned(),
            entries,
        };
        store
            .commit_with(
                &draft,
                &ExpectedBase::Revision(head.info().revision().clone()),
                None,
                &mut NoFault,
            )
            .unwrap();
    }
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn is_null(json: &Json, path: &[&str]) -> bool {
    matches!(at(json, path), Json::Null)
}
fn close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{actual} vs {expected}"
    );
}

/// A hot-rolled I section 298 x 149, web 5.5, flange 8, root radius 13 (given by dimensions).
const I298: &[&str] = &[
    "--family", "i_shape", "--h", "298", "--b", "149", "--tw", "5.5", "--tf", "8", "--r", "13",
];

/// The exact area: the sharp-corner 3935 mm2 plus four root fillets of (4 - pi) / 4 * r^2.
fn exact_i298() -> f64 {
    3935.0 + (4.0 - PI) * 169.0
}

#[test]
fn a_computed_area_gives_the_exact_mass_of_the_reference_i_section() {
    let f = Fixture::new();
    let profile = f.profile(I298);
    let steel = f.steel();
    let beam = f.beam(&profile, Some(&steel));
    let q = at(&beam, &["quantities"]);
    assert_eq!(text(q, "area_source"), "computed");
    close(num(q, &["section_area_mm2"]), exact_i298(), 1e-9);
    close(num(q, &["volume", "mm3"]), exact_i298() * 6000.0, 1e-9);
    // 4080.0708 mm2 x 6 m x 7850 kg/m3 = 192.168 kg.
    close(
        num(q, &["mass", "kg"]),
        exact_i298() * 6000.0 * 7850.0 / 1e9,
        1e-9,
    );
    assert!((num(q, &["mass", "kg"]) - 192.168).abs() < 0.01);
    assert_eq!(num(q, &["mass", "kg_display"]), 192.2);
    // Without operations gross and net are the same figure, and the old keys are the net ones.
    assert_eq!(num(q, &["volume_gross_mm3"]), num(q, &["volume_net_mm3"]));
    assert_eq!(num(q, &["volume_net_mm3"]), num(q, &["volume", "mm3"]));
    assert_eq!(num(q, &["mass_gross_kg"]), num(q, &["mass_net_kg"]));
    assert_eq!(num(q, &["mass_net_kg"]), num(q, &["mass", "kg"]));
    assert_eq!(num(q, &["mass_net_kg_display"]), 192.2);
    assert_eq!(num(q, &["removed_volume_mm3"]), 0.0);
    // The old polygon figure (192.58 kg) is not what is reported any more.
    assert!(num(q, &["mass", "kg"]) < 192.4);
}

#[test]
fn a_declared_catalogue_area_is_used_and_labelled() {
    let f = Fixture::new();
    let mut args = I298.to_vec();
    args.extend_from_slice(&["--area-mm2", "4080"]);
    let reply = f.ok("define-profile", &args);
    // 4080 is within one percent of the exact area: no warning.
    assert!(is_null(
        at(&reply, &["definition"]),
        &["declared_area_warning"]
    ));
    let profile = text(at(&reply, &["definition"]), "digest");
    let steel = f.steel();
    let beam = f.beam(&profile, Some(&steel));
    let q = at(&beam, &["quantities"]);
    assert_eq!(text(q, "area_source"), "catalogue");
    assert_eq!(num(q, &["section_area_mm2"]), 4080.0);
    assert_eq!(num(q, &["volume", "mm3"]), 4080.0 * 6000.0);
    close(num(q, &["mass", "kg"]), 192.168, 1e-9);
}

#[test]
fn a_declared_area_far_from_the_exact_one_is_a_warning_and_still_used() {
    let f = Fixture::new();
    let mut args = I298.to_vec();
    args.extend_from_slice(&["--area-mm2", "4400"]);
    let reply = f.ok("define-profile", &args);
    let warning = at(&reply, &["definition", "declared_area_warning"]);
    assert_eq!(num(warning, &["declared_mm2"]), 4400.0);
    // The comparison is with the exact area, not with the polygon of the mesh.
    close(num(warning, &["kernel_mm2"]), exact_i298(), 1e-9);
    let beam = f.beam(&text(at(&reply, &["definition"]), "digest"), None);
    assert_eq!(
        num(&beam, &["quantities", "volume", "mm3"]),
        4400.0 * 6000.0
    );
}

#[test]
fn without_a_density_the_mass_is_unknown_and_the_volumes_remain() {
    let f = Fixture::new();
    let beam = f.beam(&f.profile(I298), None);
    let q = at(&beam, &["quantities"]);
    assert!(is_null(q, &["mass", "kg"]));
    assert!(is_null(q, &["mass", "kg_display"]));
    assert!(is_null(q, &["mass_gross_kg"]));
    assert!(is_null(q, &["mass_net_kg"]));
    assert_eq!(text(at(q, &["mass"]), "status"), "unknown");
    assert_eq!(text(q, "area_source"), "computed");
    close(num(q, &["volume_gross_mm3"]), exact_i298() * 6000.0, 1e-9);
}

#[test]
fn custom_and_rectangle_members_use_their_own_areas() {
    let f = Fixture::new();
    let steel = f.steel();
    let custom = f.profile(&[
        "--family",
        "custom",
        "--outer",
        "-50,-50;50,-50;50,50;-50,50",
        "--cavity",
        "-10,-10;-10,10;10,10;10,-10",
    ]);
    let q = at(&f.beam(&custom, Some(&steel)), &["quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "computed");
    assert_eq!(num(&q, &["section_area_mm2"]), 9600.0);
    assert_eq!(num(&q, &["volume", "mm3"]), 9600.0 * 6000.0);
    close(
        num(&q, &["mass", "kg"]),
        9600.0 * 6000.0 * 7850.0 / 1e9,
        1e-12,
    );

    let rectangle = f.profile(&["--width", "200", "--height", "300"]);
    let q = at(&f.beam(&rectangle, Some(&steel)), &["quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "computed");
    assert_eq!(num(&q, &["section_area_mm2"]), 60_000.0);
    assert_eq!(num(&q, &["volume", "mm3"]), 360_000_000.0);
    assert_eq!(num(&q, &["mass", "kg"]), 2826.0);

    // A rectangle that declares its area (within the P02.2 0.1 percent check) is catalogue.
    let declared = f.profile(&["--width", "200", "--height", "300", "--area-mm2", "60030"]);
    let q = at(&f.beam(&declared, Some(&steel)), &["quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["volume", "mm3"]), 60_030.0 * 6000.0);
}

#[test]
fn members_from_the_synthetic_library_follow_their_declared_or_computed_area() {
    let f = Fixture::new();
    let steel = f.steel();
    let from_library =
        |designation: &str| f.profile(&["--library", SHIPPED, "--designation", designation]);
    // Declared 5153 mm2 (the exact area is 5153.2): catalogue.
    let q = at(
        &f.beam(&from_library("SYN-I-300x150"), Some(&steel)),
        &["quantities"],
    )
    .clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["volume", "mm3"]), 5153.0 * 6000.0);
    // No declared area: computed, 2*100*8 + 184*5 + (4 - pi) * 100 = 2605.84.
    let q = at(
        &f.beam(&from_library("SYN-I-200x100"), Some(&steel)),
        &["quantities"],
    )
    .clone();
    assert_eq!(text(&q, "area_source"), "computed");
    close(
        num(&q, &["section_area_mm2"]),
        2520.0 + (4.0 - PI) * 100.0,
        1e-9,
    );
    // Hollow section without a declared area: pi * t * (d - t).
    let q = at(
        &f.beam(&from_library("SYN-CHS-114x5"), Some(&steel)),
        &["quantities"],
    )
    .clone();
    close(num(&q, &["section_area_mm2"]), PI * 5.0 * 109.0, 1e-12);
    // Declared bar and declared custom outline.
    let q = at(
        &f.beam(&from_library("SYN-BAR-100x20"), Some(&steel)),
        &["quantities"],
    )
    .clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 2000.0);
    let q = at(
        &f.beam(&from_library("SYN-TEE-CUSTOM"), Some(&steel)),
        &["quantities"],
    )
    .clone();
    assert_eq!(num(&q, &["section_area_mm2"]), 2400.0);
    // Every member reopens in a fresh process with the same figures and a current record.
    let inspected = f.ok("inspect-elements", &[]);
    for member in at(&inspected, &["members"]).array().unwrap() {
        let stored = at(member, &["stored_quantities"]);
        assert_eq!(text(at(stored, &["volume"]), "freshness"), "current");
        assert_eq!(text(at(stored, &["mass"]), "freshness"), "current");
        assert!(!matches!(
            at(stored, &["superseded_algorithm"]),
            Json::Bool(true)
        ));
        assert!(
            at(member, &["quantities", "section_area_mm2"])
                .float()
                .is_ok()
        );
    }
}

#[test]
fn a_plate_with_holes_has_a_gross_that_ignores_them_and_a_net_that_does_not() {
    let f = Fixture::new();
    let steel = f.steel();
    let created = f.ok(
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--hole-rect",
            "100,100,100,100;400,200,50,50",
            "--material-digest",
            &steel,
        ],
    );
    let q = at(&created, &["plate", "quantities"]);
    assert_eq!(num(q, &["volume_gross_mm3"]), 5_000_000.0);
    close(num(q, &["volume_net_mm3"]), 4_875_000.0, 1e-12);
    close(num(q, &["removed_volume_mm3"]), 125_000.0, 1e-9);
    // The old keys are the net ones.
    assert_eq!(num(q, &["volume", "mm3"]), num(q, &["volume_net_mm3"]));
    assert_eq!(num(q, &["mass", "kg"]), num(q, &["mass_net_kg"]));
    close(num(q, &["mass_gross_kg"]), 39.25, 1e-12);
    close(num(q, &["mass_net_kg"]), 38.268_75, 1e-9);
    assert_eq!(num(q, &["mass_gross_kg_display"]), 39.3);
    assert_eq!(num(q, &["mass_net_kg_display"]), 38.3);
    assert!(is_null(q, &["area_source"]));
    // A plate without holes: gross equals net.
    let solid = f.ok(
        "create-plate",
        &["--width", "1000", "--length", "500", "--thickness", "10"],
    );
    let q = at(&solid, &["plate", "quantities"]);
    assert_eq!(num(q, &["volume_gross_mm3"]), num(q, &["volume_net_mm3"]));
    assert_eq!(num(q, &["removed_volume_mm3"]), 0.0);
}

#[test]
fn a_store_from_before_this_slice_reads_stale_and_stays_usable() {
    let f = Fixture::new();
    let steel = f.steel();
    let profile = f.profile(I298);
    let beam = f.beam(&profile, Some(&steel));
    let id = text(&beam, "element_id");
    let current = f.ok("inspect-elements", &[]);
    let member = &at(&current, &["members"]).array().unwrap()[0];
    assert_eq!(
        text(at(member, &["stored_quantities", "volume"]), "freshness"),
        "current"
    );
    // Rewrite the derived record the way the earlier binary wrote it: its algorithm label, the
    // polygon volume and mass of the detailed outline, and signatures computed from other bytes.
    let old_label =
        "kernel-a07-section-member-volume/1;detailed-arcs-max(0.25mm,0.5%r);tolerance-policy-3";
    let old_hex = "ab".repeat(32);
    f.tamper(|name, layer| {
        if name != "derived.usda" {
            return layer;
        }
        layer
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                let indent = &line[..line.len() - trimmed.len()];
                if trimmed.starts_with("custom string bim:quantityAlgorithm") {
                    format!("{indent}custom string bim:quantityAlgorithm = \"{old_label}\"")
                } else if trimmed.starts_with("custom string bim:shapeSignature") {
                    format!("{indent}custom string bim:shapeSignature = \"{old_hex}\"")
                } else if trimmed.starts_with("custom string bim:massSignature") {
                    format!("{indent}custom string bim:massSignature = \"{old_hex}\"")
                } else if trimmed.starts_with("custom double bim:volumeMm3") {
                    format!("{indent}custom double bim:volumeMm3 = 24555000")
                } else if trimmed.starts_with("custom double bim:massKg") {
                    format!("{indent}custom double bim:massKg = 192.58")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    });
    // Reported stale, as a superseded value; the engine's own figures are the exact ones.
    let inspected = f.ok("inspect-elements", &[]);
    let member = &at(&inspected, &["members"]).array().unwrap()[0];
    let stored = at(member, &["stored_quantities"]);
    assert_eq!(text(at(stored, &["volume"]), "freshness"), "stale");
    assert_eq!(text(at(stored, &["mass"]), "freshness"), "stale");
    assert_eq!(num(stored, &["volume", "stored_value"]), 24_555_000.0);
    assert!(matches!(
        at(stored, &["superseded_algorithm"]),
        Json::Bool(true)
    ));
    close(num(member, &["quantities", "mass", "kg"]), 192.168, 1e-4);
    // The store is not corrupt: other elements can still be published on top of it ...
    f.ok(
        "create-plate",
        &["--width", "100", "--length", "100", "--thickness", "5"],
    );
    // ... and the stale record is recomputed when an edit rewrites the quantities.
    let light = text(
        f.ok("define-material", &["--density", "2700"])
            .get("definition")
            .unwrap(),
        "digest",
    );
    f.ok(
        "update-element",
        &["--id", &id, "--material-digest", &light],
    );
    let rewritten = f.ok("inspect-elements", &[]);
    let member = at(&rewritten, &["members"])
        .array()
        .unwrap()
        .iter()
        .find(|m| text(m, "element_id") == id)
        .unwrap();
    let stored = at(member, &["stored_quantities"]);
    assert_eq!(text(at(stored, &["volume"]), "freshness"), "current");
    assert_eq!(text(at(stored, &["mass"]), "freshness"), "current");
    assert!(matches!(
        at(stored, &["superseded_algorithm"]),
        Json::Bool(false)
    ));
    close(
        num(member, &["quantities", "mass", "kg"]),
        exact_i298() * 6000.0 * 2700.0 / 1e9,
        1e-9,
    );

    // The in-process reader agrees.
    let report = inspect(&f.revisions()).unwrap();
    assert!(
        report
            .quantities
            .values()
            .all(|r| r.assessment.volume.as_str() == "current")
    );
}

#[test]
fn a_record_with_the_current_label_and_a_wrong_value_is_still_refused() {
    let f = Fixture::new();
    let steel = f.steel();
    let profile = f.profile(I298);
    f.beam(&profile, Some(&steel));
    f.tamper(|name, layer| {
        if name != "derived.usda" {
            return layer;
        }
        layer
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("custom double bim:massKg") {
                    "        custom double bim:massKg = 99".to_owned()
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    });
    assert_eq!(
        f.rejected(
            "create-plate",
            &["--width", "100", "--length", "100", "--thickness", "5"]
        ),
        "INTEGRITY_ERROR"
    );
}

#[test]
fn a_plate_record_with_a_superseded_member_label_is_still_refused() {
    let f = Fixture::new();
    f.ok(
        "create-plate",
        &["--width", "100", "--length", "100", "--thickness", "5"],
    );
    // The label exemption is for member records only: a plate whose stored volume is wrong and
    // whose algorithm label is one a member used to carry is a wrong record, not a stale one.
    f.tamper(|name, layer| {
        if name != "derived.usda" {
            return layer;
        }
        layer
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                if trimmed.starts_with("custom double bim:volumeMm3") {
                    "        custom double bim:volumeMm3 = 1".to_owned()
                } else if trimmed.starts_with("custom string bim:quantityAlgorithm") {
                    "        custom string bim:quantityAlgorithm = \"kernel-p03-straight-member-volume/1;tolerance-policy-3\"".to_owned()
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    });
    assert_eq!(
        f.rejected(
            "create-plate",
            &["--width", "200", "--length", "100", "--thickness", "5"]
        ),
        "INTEGRITY_ERROR"
    );
}
