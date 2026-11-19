// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Australia environment lists the AS/NZS 3678 material grades
// (`materials-as-nzs-3678.usda`): the structural base grades 250, 300, 350, 400 and 450 of
// AS/NZS 3678-2016, re-entered by hand as our own data; the density is the 7850 kg/m3 of the
// shipped profile packs. Provenance and licence are in `ATTRIBUTION.md` and in the library's
// own metadata.

//! The material library of the `australia` environment end to end through the real
//! `beamcraft` binary (#648): the environment lists its material library beside the profile
//! and fastener libraries, a project created with the environment carries the library in its
//! own copy, a grade resolves through it without any option, the mass math uses the catalogue
//! entry's density, the seeded constants stay the neutral fallback, and a grade the set does
//! not publish — including the qualifier forms the region's catalogues use — stays
//! `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/australia/environment.usda"
);
const ACTOR: &str = "0199e7a1-0000-7000-8000-000000000113";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("australia-materials-tests"),
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
fn the_australia_environment_lists_and_carries_its_material_library() {
    let f = Fixture::new();

    // The project carries the material library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Australia Grades",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "australia",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("materials-as-nzs-3678.usda")
            .is_file(),
        "the project copy holds the AS/NZS 3678 material library"
    );

    // The listing and the store round ride one process (#738 P7): the library listing, the
    // offered-grade reads, the two members whose mass math uses the catalogue density, and
    // the refusals in their authored order.
    let mut lines = vec![
        // The environment listing names the material library beside the profile and
        // fastener ones.
        Scripted::ok(&["list-library", "--environment", ENVIRONMENT]),
        // Every shipped grade is listed for pickers until the model defines it: the five
        // base grades, first and last in the loader's designation order.
        Scripted::ok(&["definitions", "--store", &store]),
        // A grade of the set resolves through the project's own environment copy, with no
        // option naming any file, and the mass math uses the catalogue entry's density: a
        // 610UB125 beam of 6 m at the declared catalogue area 16000 mm2 and 7850 kg/m3
        // weighs 753.6 kg.
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
            "610UB125",
            "--material",
            "350",
        ]),
        // The pinned definition is the catalogue entry itself: the declared grade, standard
        // and density ride into the model, which never needs the library file again.
        Scripted::ok(&["definitions", "--store", &store, "--name", "350"]),
        // The spent grade is no longer offered to pickers; the other four still are.
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
            "3000,0,4000",
            "--profile",
            "610UB125",
            "--material",
            "Steel",
        ]),
        Scripted::ok(&["definitions", "--store", &store, "--name", "Steel"]),
    ];
    // A grade the set does not publish stays NOT_FOUND, never a near match: no qualifier
    // forms (the NZ catalogue G-prefix and the Grade word, the AS/NZS 1594 HA form), no
    // impact suffix, no Z value, no lighter grade outside the shipped set and no grade of
    // another standard.
    let absent = [
        "G350",
        "Grade 350",
        "HA250",
        "HW350",
        "C350L0",
        "350L15",
        "350Z25",
        "200",
        "500",
        "A36",
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
            "610UB125",
            "--material",
            designation,
        ]));
    }
    let replies = f.script(&store, &lines);
    let listed = &replies[0];
    assert_eq!(
        at(listed, &["environment", "material_libraries"])
            .array()
            .unwrap(),
        [Json::text("materials-as-nzs-3678.usda")]
    );
    let available = list(&replies[1], "environment_materials");
    assert_eq!(available.len(), 5);
    assert_eq!(text(&available[0], "grade"), "250");
    assert_eq!(text(&available[4], "grade"), "450");
    let beam = &replies[2];
    let mass = at(beam, &["member", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    let kg = at(mass, &["kg"]).float().unwrap();
    assert!((kg - 753.6).abs() < 0.01, "mass {kg} kg");
    let pinned = list(&replies[3], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "material");
    assert_eq!(text(&pinned[0], "grade"), "350");
    assert_eq!(text(&pinned[0], "standard"), "AS/NZS 3678-2016");
    assert_eq!(at(&pinned[0], &["density_kg_m3"]).float().unwrap(), 7850.0);
    let remaining = list(&replies[4], "environment_materials");
    assert_eq!(remaining.len(), 4);
    assert!(
        remaining.iter().all(|g| text(g, "grade") != "350"),
        "350 is spent"
    );
    let column = &replies[5];
    let steel_mass = at(column, &["member", "quantities", "mass"]);
    let steel_kg = at(steel_mass, &["kg"]).float().unwrap();
    assert!(
        (steel_kg - 16000.0 * 4000.0 * 7850.0 / 1.0e9).abs() < 0.01,
        "Steel mass {steel_kg} kg"
    );
    let pinned_steel = list(&replies[6], "definitions");
    assert_eq!(pinned_steel.len(), 1);
    assert!(
        at(&pinned_steel[0], &["grade"]).string().is_err(),
        "the neutral Steel declares no grade"
    );

    // Replies 7..17 carry the refusals in the authored order.
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[7 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
}

#[test]
fn define_material_takes_an_as_nzs_grade_from_its_environment_entry() {
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
        "400",
    ]);
    let entry = at(&defined, &["definition"]);
    assert_eq!(text(entry, "kind"), "material");
    assert_eq!(text(entry, "grade"), "400");
    assert_eq!(text(entry, "standard"), "AS/NZS 3678-2016");
    assert_eq!(at(entry, &["density_kg_m3"]).float().unwrap(), 7850.0);

    // And an unknown grade is refused, never substituted: the qualifier form of the shipped
    // 350 included.
    assert_eq!(
        f.code(&[
            "define-material",
            "--store",
            &plain,
            "--environment",
            ENVIRONMENT,
            "--designation",
            "G350",
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
            "250L15",
        ]),
        "NOT_FOUND"
    );
}
