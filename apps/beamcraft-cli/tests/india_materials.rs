// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped India environment lists the IS 2062:2011 material grades
// (`materials-is-2062.usda`): the complete published grade set of the 2011 edition,
// re-entered by hand as our own data; the density is the 7850 kg/m3 of the shipped profile
// packs. Provenance and licence are in `ATTRIBUTION.md` and in the library's own metadata.

//! The material library of the `india` environment end to end through the real `beamcraft`
//! binary (#648): the environment lists its material library beside the profile and fastener
//! libraries, a project created with the environment carries the library in its own copy, a
//! grade resolves through it without any option, the mass math uses the catalogue entry's
//! density, the seeded constants stay the neutral fallback, and a grade the set does not
//! publish — including the 2006-edition Fe designations and the sub-qualities the higher
//! levels omit — stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/india/environment.usda"
);
const ACTOR: &str = "0199e7a1-0000-7000-8000-000000000127";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("india-materials-tests"),
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
fn the_india_environment_lists_and_carries_its_material_library() {
    let f = Fixture::new();

    // The project carries the material library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "India Grades",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "india",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("materials-is-2062.usda")
            .is_file(),
        "the project copy holds the IS 2062:2011 material library"
    );

    // The listing and the store round ride one process (#738 P7): the library listing, the
    // offered-grade reads, the two members whose mass math uses the catalogue density, and
    // the refusals in their authored order.
    let mut lines = vec![
        // The environment listing names the material library beside the profile and
        // fastener ones.
        Scripted::ok(&["list-library", "--environment", ENVIRONMENT]),
        // Every shipped grade is listed for pickers until the model defines it: the complete
        // 2011 grade set, first and last in the loader's designation order.
        Scripted::ok(&["definitions", "--store", &store]),
        // A grade of the set resolves through the project's own environment copy, with no
        // option naming any file, and the mass math uses the catalogue entry's density: an
        // NPB100x55x8.10 beam of 6 m at the declared catalogue area 1030 mm2 and 7850 kg/m3
        // weighs 48.513 kg.
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
            "NPB100x55x8.10",
            "--material",
            "E350C",
        ]),
        // The pinned definition is the catalogue entry itself: the declared grade, standard
        // and density ride into the model, which never needs the library file again.
        Scripted::ok(&["definitions", "--store", &store, "--name", "E350C"]),
        // The spent grade is no longer offered to pickers; the other 27 still are.
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
            "NPB100x55x8.10",
            "--material",
            "Steel",
        ]),
        Scripted::ok(&["definitions", "--store", &store, "--name", "Steel"]),
    ];
    // A grade the set does not publish stays NOT_FOUND, never a near match: no 2006-edition
    // Fe designations, no bare grade without a sub-quality, no lowercase, no level the
    // edition does not publish, no sub-quality the higher levels omit and no grade of
    // another standard.
    let absent = [
        "Fe410WA", "Fe490A", "Fe590", "E250", "e350c", "E250D", "E590", "E450C", "E550C", "E600C",
        "E650C", "S355",
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
            "NPB100x55x8.10",
            "--material",
            designation,
        ]));
    }
    // The enriched profile listing (the UI profile catalogue fields ride every entry since
    // the #888 UI shell) puts a full listing near 90 KB and the whole-library listing near
    // the envelope ceiling, so the whole script no longer fits one run-script summary
    // envelope (1 MiB): the library listing, the store round and the refusal round ride
    // separate invocations.
    let replies = f.script(&store, &lines[0..1]);
    let replies_b = f.script(&store, &lines[1..4]);
    let replies_c = f.script(&store, &lines[4..7]);
    let replies_d = f.script(&store, &lines[7..]);
    let listed = &replies[0];
    assert_eq!(
        at(listed, &["environment", "material_libraries"])
            .array()
            .unwrap(),
        [Json::text("materials-is-2062.usda")]
    );
    let available = list(&replies_b[0], "environment_materials");
    assert_eq!(available.len(), 28);
    assert_eq!(text(&available[0], "grade"), "E250A");
    assert_eq!(text(&available[27], "grade"), "E650BR");
    let beam = &replies_b[1];
    let mass = at(beam, &["member", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    let kg = at(mass, &["kg"]).float().unwrap();
    assert!((kg - 48.513).abs() < 0.01, "mass {kg} kg");
    let pinned = list(&replies_b[2], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "material");
    assert_eq!(text(&pinned[0], "grade"), "E350C");
    assert_eq!(text(&pinned[0], "standard"), "IS 2062-2011");
    assert_eq!(at(&pinned[0], &["density_kg_m3"]).float().unwrap(), 7850.0);
    let remaining = list(&replies_c[0], "environment_materials");
    assert_eq!(remaining.len(), 27);
    assert!(
        remaining.iter().all(|g| text(g, "grade") != "E350C"),
        "E350C is spent"
    );
    let column = &replies_c[1];
    let steel_mass = at(column, &["member", "quantities", "mass"]);
    let steel_kg = at(steel_mass, &["kg"]).float().unwrap();
    assert!(
        (steel_kg - 1030.0 * 4000.0 * 7850.0 / 1.0e9).abs() < 0.01,
        "Steel mass {steel_kg} kg"
    );
    let pinned_steel = list(&replies_c[2], "definitions");
    assert_eq!(pinned_steel.len(), 1);
    assert!(
        at(&pinned_steel[0], &["grade"]).string().is_err(),
        "the neutral Steel declares no grade"
    );

    // The third script carries the refusals in the authored order.
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies_d[index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
}

#[test]
fn define_material_takes_an_is_grade_from_its_environment_entry() {
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
        "E250B0",
    ]);
    let entry = at(&defined, &["definition"]);
    assert_eq!(text(entry, "kind"), "material");
    assert_eq!(text(entry, "grade"), "E250B0");
    assert_eq!(text(entry, "standard"), "IS 2062-2011");
    assert_eq!(at(entry, &["density_kg_m3"]).float().unwrap(), 7850.0);

    // And an unknown grade is refused, never substituted: the legacy Fe form of the shipped
    // E250 level included.
    assert_eq!(
        f.code(&[
            "define-material",
            "--store",
            &plain,
            "--environment",
            ENVIRONMENT,
            "--designation",
            "Fe410WA",
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
            "E250",
        ]),
        "NOT_FOUND"
    );
}
