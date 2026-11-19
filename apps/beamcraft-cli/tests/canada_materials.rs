// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Canada environment lists the CSA G40.21 weldable-category material grades
// (`materials-csa-g4021.usda`): the complete metric W series of the combined standard
// CSA G40.20-13/G40.21-13, re-entered by hand as our own data; the density is the 7850 kg/m3
// of the shipped profile packs. The environment carries no profile libraries, so the mass
// check rides a plate, whose mass is exact geometry. Provenance and licence are in
// `ATTRIBUTION.md` and in the library's own metadata.

//! The material library of the `canada` environment end to end through the real `beamcraft`
//! binary (#648): the grades-only environment carries its material library into a project
//! copy, a grade resolves through it without any option, the mass math uses the catalogue
//! entry's density, the seeded constants stay the neutral fallback, and a grade the set does
//! not publish — including the imperial designations and the other categories of the same
//! strength level — stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/canada/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000f7";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("canada-materials-tests"),
        };
        std::fs::create_dir(fixture.shared.root.join("models")).unwrap();
        fixture
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .env_remove("BEAMCRAFT_CONFIG_DIR")
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.raw(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }
    fn code(&self, args: &[&str]) -> String {
        let (code, body) = self.raw(args);
        assert_eq!(code, 2, "{args:?}: {body:?}");
        text(&body, "code")
    }
    /// One `run-script` invocation with the same clean environment `raw` gives its spawns:
    /// the store commands amortize the environment copy's library parse into one process
    /// (#738 P7), the commands' own work unchanged.
    fn script(&self, store: &str, lines: &[Scripted]) -> Vec<Json> {
        support::script_lines_without_config(&self.shared.root, store, lines)
    }
}

#[test]
fn the_canada_environment_lists_and_carries_its_material_library() {
    let f = Fixture::new();

    // The project carries the material library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Canada Grades",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "canada",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("materials-csa-g4021.usda")
            .is_file(),
        "the project copy holds the CSA material library"
    );

    // The listing and the store round ride one process (#738 P7): the library listing, the
    // offered-grade reads, the two plates whose mass math uses the catalogue density, and
    // the refusals in their authored order.
    let mut lines = vec![
        // The grades-only environment names its material library and no other library.
        Scripted::ok(&["list-library", "--environment", ENVIRONMENT]),
        // Every shipped grade is listed for pickers until the model defines it: the metric
        // W series, first and last in the loader's designation order.
        Scripted::ok(&["definitions", "--store", &store]),
        // A grade of the set resolves through the project's own environment copy, with no
        // option naming any file, and the mass math uses the catalogue entry's density: a
        // 200 x 3000 x 10 plate at 7850 kg/m3 weighs exactly 47.1 kg.
        Scripted::ok(&[
            "create-plate",
            "--store",
            &store,
            "--width",
            "200",
            "--length",
            "3000",
            "--thickness",
            "10",
            "--material",
            "350W",
            "--actor-id",
            ACTOR,
        ]),
        // The pinned definition is the catalogue entry itself: the declared grade, standard
        // and density ride into the model, which never needs the library file again.
        Scripted::ok(&["definitions", "--store", &store, "--name", "350W"]),
        // The spent grade is no longer offered to pickers; the other six still are.
        Scripted::ok(&["definitions", "--store", &store]),
        // The seeded constants stay the neutral fallback: Steel still answers by name.
        Scripted::ok(&[
            "create-plate",
            "--store",
            &store,
            "--width",
            "200",
            "--length",
            "3000",
            "--thickness",
            "10",
            "--material",
            "Steel",
            "--actor-id",
            ACTOR,
        ]),
        Scripted::ok(&["definitions", "--store", &store, "--name", "Steel"]),
    ];
    // A grade the set does not publish stays NOT_FOUND, never a near match: no bare strength
    // level, no imperial-ksi designation of the same grade, no other category of the same
    // strength level, no lowercase and no strength level outside the standard's set.
    let absent = [
        "350", "50W", "44W", "350WT", "350A", "350R", "w350", "600W", "700W",
    ];
    for designation in absent {
        lines.push(Scripted::refused(&[
            "create-plate",
            "--store",
            &store,
            "--width",
            "200",
            "--length",
            "3000",
            "--thickness",
            "10",
            "--material",
            designation,
            "--actor-id",
            ACTOR,
        ]));
    }
    let replies = f.script(&store, &lines);
    let listed = &replies[0];
    assert_eq!(
        at(listed, &["environment", "material_libraries"])
            .array()
            .unwrap(),
        [Json::text("materials-csa-g4021.usda")]
    );
    let available = list(&replies[1], "environment_materials");
    assert_eq!(available.len(), 7);
    assert_eq!(text(&available[0], "grade"), "260W");
    assert_eq!(text(&available[6], "grade"), "550W");
    let plate = &replies[2];
    let mass = at(plate, &["plate", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    let kg = at(mass, &["kg"]).float().unwrap();
    assert!((kg - 47.1).abs() < 0.001, "mass {kg} kg");
    let pinned = list(&replies[3], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "material");
    assert_eq!(text(&pinned[0], "grade"), "350W");
    assert_eq!(text(&pinned[0], "standard"), "CSA G40.20-13/G40.21-13");
    assert_eq!(at(&pinned[0], &["density_kg_m3"]).float().unwrap(), 7850.0);
    let remaining = list(&replies[4], "environment_materials");
    assert_eq!(remaining.len(), 6);
    assert!(
        remaining.iter().all(|g| text(g, "grade") != "350W"),
        "350W is spent"
    );
    let steel_plate = &replies[5];
    let steel_mass = at(steel_plate, &["plate", "quantities", "mass"]);
    let steel_kg = at(steel_mass, &["kg"]).float().unwrap();
    assert!((steel_kg - 47.1).abs() < 0.001, "Steel mass {steel_kg} kg");
    let pinned_steel = list(&replies[6], "definitions");
    assert_eq!(pinned_steel.len(), 1);
    assert!(
        at(&pinned_steel[0], &["grade"]).string().is_err(),
        "the neutral Steel declares no grade"
    );

    // Replies 7..16 carry the refusals in the authored order.
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[7 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
}

#[test]
fn define_material_takes_a_csa_grade_from_its_environment_entry() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Plain",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "blank",
    ]);
    let plain = text(at(&created, &["project"]), "path");

    // An explicit --environment names the material library directly: the definition takes every
    // value from the entry and the model never needs the file again.
    let defined = f.ok(&[
        "define-material",
        "--store",
        &plain,
        "--environment",
        ENVIRONMENT,
        "--designation",
        "300W",
    ]);
    let entry = at(&defined, &["definition"]);
    assert_eq!(text(entry, "kind"), "material");
    assert_eq!(text(entry, "grade"), "300W");
    assert_eq!(text(entry, "standard"), "CSA G40.20-13/G40.21-13");
    assert_eq!(at(entry, &["density_kg_m3"]).float().unwrap(), 7850.0);

    // And an unknown grade is refused, never substituted: the imperial designation of the
    // shipped 300W included.
    assert_eq!(
        f.code(&[
            "define-material",
            "--store",
            &plain,
            "--environment",
            ENVIRONMENT,
            "--designation",
            "44W",
        ]),
        "NOT_FOUND"
    );
    assert_eq!(
        f.code(&[
            "define-material",
            "--store",
            &plain,
            "--environment",
            ENVIRONMENT,
            "--designation",
            "260WT",
        ]),
        "NOT_FOUND"
    );
}
