// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped USA environment lists the ASTM material grades
// (`materials-astm.usda`): the grade designations of ASTM A36/A36M-19, A992/A992M-22 and
// A572/A572M-21, re-entered by hand as our own data; the density is the metric 7850 kg/m3 of
// the shipped profile packs. The mass check rides a plate, whose mass is exact geometry, and
// the assertions name the material library alone: the environment's profile library is the
// subject of `usa_environment.rs`. Provenance and licence are in `ATTRIBUTION.md` and in the
// library's own metadata.

//! The material library of the `usa` environment end to end through the real `beamcraft`
//! binary (#648): the environment lists its material library, a project
//! created with the environment carries the library in its own copy, a grade resolves through
//! it without any option, the mass math uses the catalogue entry's density, and a grade the
//! set does not publish — including the compact industry shorthands — stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/usa/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000e5";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("usa-materials-tests"),
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
fn the_usa_environment_lists_and_carries_its_material_library() {
    let f = Fixture::new();

    // The project carries the material library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "USA Grades",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "usa",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("materials-astm.usda")
            .is_file(),
        "the project copy holds the ASTM material library"
    );

    // The listing and the store round ride one process (#738 P7): the library listing, the
    // offered-grade reads, the two plates whose mass math uses the catalogue density, and
    // the refusals in their authored order.
    let mut lines = vec![
        // The environment listing names the material library; the material assertions read
        // that list alone (the profile library beside it is `usa_environment.rs`). One entry
        // is asked for: the 285 profile rows would fill the script's reply budget.
        Scripted::ok(&["list-library", "--environment", ENVIRONMENT, "--limit", "1"]),
        // Every shipped grade is listed for pickers until the model defines it: the single
        // A36, the five A572 grades and A992, first and last in the loader's designation
        // order.
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
            "A36",
            "--actor-id",
            ACTOR,
        ]),
        // The pinned definition is the catalogue entry itself: the declared grade, standard
        // and density ride into the model, which never needs the library file again.
        Scripted::ok(&["definitions", "--store", &store, "--name", "A36"]),
        // The spent grade is no longer offered to pickers; the other six still are. That
        // listing is read in its own process after the script: a project from this
        // environment offers 285 profiles beside the grades, so two full listings would fill
        // the script's reply budget.
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
    // A grade the set does not publish stays NOT_FOUND, never a near match: no bare spec
    // number, no hyphenated or compact industry shorthand (the shipped A572 form is the
    // standard's own Grade form), no lowercase, no grade outside the standard's set and no
    // grade of another specification.
    let absent = [
        "A572",
        "A572-50",
        "A572Gr50",
        "A36M",
        "a992",
        "A572 Grade 70",
        "A500",
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
        [Json::text("materials-astm.usda")]
    );
    let available = list(&replies[1], "environment_materials");
    assert_eq!(available.len(), 7);
    assert_eq!(text(&available[0], "grade"), "A36");
    assert_eq!(text(&available[6], "grade"), "A992");
    let plate = &replies[2];
    let mass = at(plate, &["plate", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    let kg = at(mass, &["kg"]).float().unwrap();
    assert!((kg - 47.1).abs() < 0.001, "mass {kg} kg");
    let pinned = list(&replies[3], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "material");
    assert_eq!(text(&pinned[0], "grade"), "A36");
    assert_eq!(text(&pinned[0], "standard"), "ASTM A36/A36M-19");
    assert_eq!(at(&pinned[0], &["density_kg_m3"]).float().unwrap(), 7850.0);
    let offered = f.ok(&["definitions", "--store", &store]);
    let remaining = list(&offered, "environment_materials");
    assert_eq!(remaining.len(), 6);
    assert!(
        remaining.iter().all(|g| text(g, "grade") != "A36"),
        "A36 is spent"
    );
    let steel_plate = &replies[4];
    let steel_mass = at(steel_plate, &["plate", "quantities", "mass"]);
    let steel_kg = at(steel_mass, &["kg"]).float().unwrap();
    assert!((steel_kg - 47.1).abs() < 0.001, "Steel mass {steel_kg} kg");
    let pinned_steel = list(&replies[5], "definitions");
    assert_eq!(pinned_steel.len(), 1);
    assert!(
        at(&pinned_steel[0], &["grade"]).string().is_err(),
        "the neutral Steel declares no grade"
    );

    // Replies 6..13 carry the refusals in the authored order.
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[6 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
}

#[test]
fn define_material_takes_an_astm_grade_from_its_environment_entry() {
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
        "A992",
    ]);
    let entry = at(&defined, &["definition"]);
    assert_eq!(text(entry, "kind"), "material");
    assert_eq!(text(entry, "grade"), "A992");
    assert_eq!(text(entry, "standard"), "ASTM A992/A992M-22");
    assert_eq!(at(entry, &["density_kg_m3"]).float().unwrap(), 7850.0);

    // And an unknown grade is refused, never substituted: the compact shorthand of the shipped
    // A572 Grade 50 included.
    assert_eq!(
        f.code(&[
            "define-material",
            "--store",
            &plain,
            "--environment",
            ENVIRONMENT,
            "--designation",
            "A572-50",
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
            "A913",
        ]),
        "NOT_FOUND"
    );
}
