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

use crate::support::{self, Scripted, at, list, text};

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
    /// One `run-script` invocation with the same clean environment `raw` gives its spawns:
    /// the store commands amortize the environment copy's library parse into one process
    /// (#738 P7; the measured per-spawn cost is ~0.8 s on a store carrying the Russia
    /// libraries, the commands' own work unchanged).
    fn script(&self, store: &str, lines: &[Scripted]) -> Vec<Json> {
        support::script_lines_without_config(&self.shared.root, store, lines)
    }
}

#[test]
fn the_russia_environment_lists_and_carries_its_material_library() {
    let f = Fixture::new();

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

    // The listing and the store round ride one process (#738 P7): the library listing, the
    // offered-grade reads, the two members whose mass math uses the catalogue density, and
    // the refusals in their authored order.
    let mut lines = vec![
        // The environment listing names the material library beside the profile and
        // fastener ones.
        Scripted::ok(&["list-library", "--environment", ENVIRONMENT]),
        // Every shipped grade is listed for pickers until the model defines it: the
        // complete clause 4.1 grade set, first and last in the loader's designation order.
        Scripted::ok(&["definitions", "--store", &store]),
        // A grade of the set resolves through the project's own environment copy, with no
        // option naming any file, and the mass math uses the catalogue entry's density: a
        // 30B1 beam of 6 m at the declared catalogue area 4192 mm2 and 7850 kg/m3 weighs
        // 197.4432 kg.
        Scripted::ok(&[
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
        ]),
        // The pinned definition is the catalogue entry itself: the declared grade, standard
        // and density ride into the model, which never needs the library file again.
        Scripted::ok(&["definitions", "--store", &store, "--name", "С245"]),
        // The spent grade is no longer offered to pickers; the other 13 still are.
        Scripted::ok(&["definitions", "--store", &store]),
        // The seeded constants stay the neutral fallback: Steel still answers by name.
        Scripted::ok(&[
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
        ]),
        Scripted::ok(&["definitions", "--store", &store, "--name", "Steel"]),
    ];
    // A grade the set does not publish stays NOT_FOUND, never a near match: the Latin
    // look-alikes of the Cyrillic С (the C of a Latin keyboard and the transliteration S),
    // no lowercase, no bare strength class and no grade of the superseded 1988 edition.
    let absent = [
        "C245", "S245", "c345", "s355", "С285", "С375", "С440Д", "С595",
    ];
    for designation in absent {
        lines.push(Scripted::refused(&[
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
            designation,
        ]));
    }
    // The enriched profile listing (the UI profile catalogue fields ride every entry since
    // the #888 UI shell) puts a full listing of the 715-row pack near the run-script summary
    // envelope ceiling (1 MiB), so every fat listing rides its own invocation: the library
    // listing, each full definitions listing, the two member rounds and the refusal round.
    let replies = f.script(&store, &lines[0..1]);
    let replies_b = f.script(&store, &lines[1..2]);
    let replies_c = f.script(&store, &lines[2..4]);
    let replies_d = f.script(&store, &lines[4..5]);
    let replies_e = f.script(&store, &lines[5..7]);
    let replies_f = f.script(&store, &lines[7..]);
    let listed = &replies[0];
    assert_eq!(
        at(listed, &["environment", "material_libraries"])
            .array()
            .unwrap(),
        [Json::text("materials-gost-27772.usda")]
    );
    let available = list(&replies_b[0], "environment_materials");
    assert_eq!(available.len(), 14);
    assert_eq!(text(&available[0], "grade"), "С235");
    assert_eq!(text(&available[13], "grade"), "С590");
    let beam = &replies_c[0];
    let mass = at(beam, &["member", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    let kg = at(mass, &["kg"]).float().unwrap();
    assert!((kg - 197.4432).abs() < 0.01, "mass {kg} kg");
    let pinned = list(&replies_c[1], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "material");
    assert_eq!(text(&pinned[0], "grade"), "С245");
    assert_eq!(text(&pinned[0], "standard"), "GOST 27772-2015");
    assert_eq!(at(&pinned[0], &["density_kg_m3"]).float().unwrap(), 7850.0);
    let remaining = list(&replies_d[0], "environment_materials");
    assert_eq!(remaining.len(), 13);
    assert!(
        remaining.iter().all(|g| text(g, "grade") != "С245"),
        "С245 is spent"
    );
    let column = &replies_e[0];
    let steel_mass = at(column, &["member", "quantities", "mass"]);
    let steel_kg = at(steel_mass, &["kg"]).float().unwrap();
    assert!(
        (steel_kg - 21096.0 * 5000.0 * 7850.0 / 1.0e9).abs() < 0.01,
        "Steel mass {steel_kg} kg"
    );
    let pinned_steel = list(&replies_e[1], "definitions");
    assert_eq!(pinned_steel.len(), 1);
    assert!(
        at(&pinned_steel[0], &["grade"]).string().is_err(),
        "the neutral Steel declares no grade"
    );

    // The third script carries the refusals in the authored order.
    for (index, absent) in absent.iter().enumerate() {
        assert_eq!(text(&replies_f[index], "code"), "NOT_FOUND", "{absent}");
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
