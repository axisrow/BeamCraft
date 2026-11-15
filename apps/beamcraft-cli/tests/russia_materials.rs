// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Russia environment lists the GOST 27772-2015 material grades
// (`materials-gost-27772.usda`): the complete 2015 grade set, re-entered by hand as our own
// data, the designations in the standard's own Cyrillic form (the leading letter is the
// Cyrillic С, not the Latin C or a transliteration); the density is the 7850 kg/m3 of the
// shipped profile packs. Provenance and licence are in `ATTRIBUTION.md` and in the library's
// own metadata.

//! The material libraries of the `russia` environment end to end through the real `beamcraft`
//! binary (#648): the environment lists its material library beside the profile and fastener
//! libraries, a project created with the environment carries the material library in its own
//! copy, a grade resolves through it without any option (the same resolver as profiles and
//! bolts), the mass math uses the catalogue entry's density, the seeded constants stay the
//! neutral fallback, and a grade the set does not publish — including the Latin look-alikes of
//! the Cyrillic С — stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/russia/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000d3";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("russia-materials-tests"),
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
}

#[test]
fn the_russia_environment_lists_and_carries_its_material_library() {
    let f = Fixture::new();

    // The environment listing names the material library beside the profile and fastener ones.
    let listed = f.ok(&["list-library", "--environment", ENVIRONMENT]);
    assert_eq!(
        at(&listed, &["environment", "material_libraries"])
            .array()
            .unwrap(),
        [Json::text("materials-gost-27772.usda")]
    );

    // The project carries the material library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Russia Grades",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "russia",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("materials-gost-27772.usda")
            .is_file(),
        "the project copy holds the GOST 27772-2015 material library"
    );

    // Every shipped grade is listed for pickers until the model defines it: the complete
    // clause 4.1 grade set, first and last in the loader's designation order.
    let before = f.ok(&["definitions", "--store", &store]);
    let available = list(&before, "environment_materials");
    assert_eq!(available.len(), 14);
    assert_eq!(text(&available[0], "grade"), "С235");
    assert_eq!(text(&available[13], "grade"), "С590");

    // A grade of the set resolves through the project's own environment copy, with no option
    // naming any file, and the mass math uses the catalogue entry's density: a 30B1 beam of
    // 6 m at the declared catalogue area 4192 mm2 and 7850 kg/m3 weighs 197.4432 kg.
    let beam = f.ok(&[
        "create-member",
        "--store",
        &store,
        "--kind",
        "beam",
        "--start",
        "0,0,0",
        "--end",
        "6000,0,0",
        "--profile",
        "30B1",
        "--material",
        "С245",
    ]);
    let mass = at(&beam, &["member", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    let kg = at(mass, &["kg"]).float().unwrap();
    assert!((kg - 197.4432).abs() < 0.01, "mass {kg} kg");

    // The pinned definition is the catalogue entry itself: the declared grade, standard and
    // density ride into the model, which never needs the library file again.
    let after = f.ok(&["definitions", "--store", &store, "--name", "С245"]);
    let pinned = list(&after, "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "material");
    assert_eq!(text(&pinned[0], "grade"), "С245");
    assert_eq!(text(&pinned[0], "standard"), "GOST 27772-2015");
    assert_eq!(at(&pinned[0], &["density_kg_m3"]).float().unwrap(), 7850.0);
    // The spent grade is no longer offered to pickers; the other 13 still are.
    let spent = f.ok(&["definitions", "--store", &store]);
    let remaining = list(&spent, "environment_materials");
    assert_eq!(remaining.len(), 13);
    assert!(
        remaining.iter().all(|g| text(g, "grade") != "С245"),
        "С245 is spent"
    );

    // The seeded constants stay the neutral fallback: Steel still answers by name.
    let column = f.ok(&[
        "create-member",
        "--store",
        &store,
        "--kind",
        "column",
        "--start",
        "3000,0,0",
        "--end",
        "3000,0,5000",
        "--profile",
        "40K2",
        "--material",
        "Steel",
    ]);
    let steel_mass = at(&column, &["member", "quantities", "mass"]);
    let steel_kg = at(steel_mass, &["kg"]).float().unwrap();
    assert!(
        (steel_kg - 21096.0 * 5000.0 * 7850.0 / 1.0e9).abs() < 0.01,
        "Steel mass {steel_kg} kg"
    );
    let steel = f.ok(&["definitions", "--store", &store, "--name", "Steel"]);
    let pinned_steel = list(&steel, "definitions");
    assert_eq!(pinned_steel.len(), 1);
    assert!(
        at(&pinned_steel[0], &["grade"]).string().is_err(),
        "the neutral Steel declares no grade"
    );

    // A grade the set does not publish stays NOT_FOUND, never a near match: the Latin
    // look-alikes of the Cyrillic С (the C of a Latin keyboard and the transliteration S),
    // no lowercase, no bare strength class and no grade of the superseded 1988 edition.
    for absent in [
        "C245", "S245", "c345", "s355", "С285", "С375", "С440Д", "С595",
    ] {
        assert_eq!(
            f.code(&[
                "create-member",
                "--store",
                &store,
                "--kind",
                "beam",
                "--start",
                "0,0,0",
                "--end",
                "0,6000,0",
                "--profile",
                "30B1",
                "--material",
                absent,
            ]),
            "NOT_FOUND",
            "{absent}"
        );
    }
}

#[test]
fn define_material_takes_a_russia_grade_from_its_environment_entry() {
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
        "С355",
    ]);
    let entry = at(&defined, &["definition"]);
    assert_eq!(text(entry, "kind"), "material");
    assert_eq!(text(entry, "grade"), "С355");
    assert_eq!(text(entry, "standard"), "GOST 27772-2015");
    assert_eq!(at(entry, &["density_kg_m3"]).float().unwrap(), 7850.0);

    // And an unknown grade is refused, never substituted: the Latin look-alike of the Cyrillic
    // С included.
    assert_eq!(
        f.code(&[
            "define-material",
            "--store",
            &plain,
            "--environment",
            ENVIRONMENT,
            "--designation",
            "S355",
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
            "С690",
        ]),
        "NOT_FOUND"
    );
}
