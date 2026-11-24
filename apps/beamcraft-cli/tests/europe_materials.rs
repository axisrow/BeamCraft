// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Europe environment lists the EN 10025 material grades
// (`materials-en-10025.usda`): the grade designations of EN 10025-2/-3/-4, re-entered by hand
// as our own data; the density is the 7850 kg/m3 of the shipped profile packs. Provenance and
// licence are in `ATTRIBUTION.md` and in the library's own metadata.

//! The material libraries of a regional environment end to end through the real `beamcraft`
//! binary (#636): the environment lists its material library beside the profile and fastener
//! libraries, a project created with the environment carries the material library in its own
//! copy, a grade resolves through it without any option (the same resolver as profiles and
//! bolts), the mass math uses the catalogue entry's density, the seeded constants stay the
//! neutral fallback, and a grade the set does not publish stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/europe/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a6";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("europe-materials-tests"),
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
fn the_europe_environment_lists_and_carries_its_material_library() {
    let f = Fixture::new();

    // The project carries the material library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Europe Grades",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "europe",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("materials-en-10025.usda")
            .is_file(),
        "the project copy holds the EN 10025 material library"
    );

    // The listing and the store round ride one process (#738 P7): the library listing, the
    // offered-grade reads and the two members whose mass math uses the catalogue density.
    let lines = vec![
        // The environment listing names the material library beside the profile and
        // fastener ones.
        Scripted::ok(&["list-library", "--environment", ENVIRONMENT]),
        // Every shipped grade is listed for pickers until the model defines it.
        Scripted::ok(&["definitions", "--store", &store]),
        // A grade of the set resolves through the project's own environment copy, with no
        // option naming any file, and the mass math uses the catalogue entry's density: an
        // IPE300 beam of 6 m at the declared catalogue area 5381 mm2 and 7850 kg/m3 weighs
        // 253.4451 kg.
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
            "IPE300",
            "--material",
            "S235JR",
        ]),
        // The pinned definition is the catalogue entry itself: the declared grade, standard
        // and density ride into the model, which never needs the library file again.
        Scripted::ok(&["definitions", "--store", &store, "--name", "S235JR"]),
        // The spent grade is no longer offered to pickers; the other 32 still are.
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
            "IPE300",
            "--material",
            "Steel",
        ]),
    ];
    // A grade the set does not publish stays NOT_FOUND, never a near match.
    let replies = f.script(&store, &lines);
    // The last pinned-definition read and the refusals ride a second invocation: one summary
    // over the whole eleven-line script outgrew the output budget when #790 doubled the
    // europe catalog the definitions replies embed (environment_profiles carries every
    // designation the pickers offer), and the envelope dropped the newest replies as
    // replies_omitted. The second script's summary carries all of its replies.
    let mut tail_lines = vec![Scripted::ok(&[
        "definitions",
        "--store",
        &store,
        "--name",
        "Steel",
    ])];
    tail_lines.extend(
        ["S355", "S235", "s355j2", "S355K2+N"]
            .iter()
            .map(|designation| {
                Scripted::refused(&[
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
                    "IPE300",
                    "--material",
                    designation,
                ])
            }),
    );
    let tail_replies = f.script(&store, &tail_lines);
    let listed = &replies[0];
    assert_eq!(
        at(listed, &["environment", "material_libraries"])
            .array()
            .unwrap(),
        [Json::text("materials-en-10025.usda")]
    );
    let available = list(&replies[1], "environment_materials");
    assert_eq!(available.len(), 33);
    assert_eq!(text(&available[0], "grade"), "S235J0");
    assert_eq!(text(&available[32], "grade"), "S500ML");
    let beam = &replies[2];
    let mass = at(beam, &["member", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    let kg = at(mass, &["kg"]).float().unwrap();
    assert!((kg - 253.4451).abs() < 0.01, "mass {kg} kg");
    let pinned = list(&replies[3], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "material");
    assert_eq!(text(&pinned[0], "grade"), "S235JR");
    assert_eq!(text(&pinned[0], "standard"), "EN 10025-2:2019");
    assert_eq!(at(&pinned[0], &["density_kg_m3"]).float().unwrap(), 7850.0);
    let remaining = list(&replies[4], "environment_materials");
    assert_eq!(remaining.len(), 32);
    assert!(
        remaining.iter().all(|g| text(g, "grade") != "S235JR"),
        "S235JR is spent"
    );
    let column = &replies[5];
    let steel_mass = at(column, &["member", "quantities", "mass"]);
    let steel_kg = at(steel_mass, &["kg"]).float().unwrap();
    assert!(
        (steel_kg - 5381.0 * 5000.0 * 7850.0 / 1.0e9).abs() < 0.01,
        "Steel mass {steel_kg} kg"
    );
    let pinned_steel = list(&tail_replies[0], "definitions");
    assert_eq!(pinned_steel.len(), 1);
    assert!(
        at(&pinned_steel[0], &["grade"]).string().is_err(),
        "the neutral Steel declares no grade"
    );

    // The second invocation's replies after the pinned definition carry the refusals in the
    // authored order.
    for (index, designation) in ["S355", "S235", "s355j2", "S355K2+N"].iter().enumerate() {
        assert_eq!(
            text(&tail_replies[1 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
}

#[test]
fn define_material_takes_its_values_from_an_environment_entry() {
    let f = Fixture::new();
    // A project without environment materials: its grades stay NOT_FOUND and the frame lists
    // nothing, exactly as before the material libraries.
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
    let frame = f.ok(&["definitions", "--store", &plain]);
    assert_eq!(list(&frame, "environment_materials").len(), 0);
    assert_eq!(
        f.code(&[
            "create-member",
            "--store",
            &plain,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile",
            "IPE300",
            "--material",
            "S355J2",
        ]),
        "NOT_FOUND",
        "no environment materials, no grade"
    );

    // An explicit --environment names the material library directly: the definition takes every
    // value from the entry and the model never needs the file again.
    let defined = f.ok(&[
        "define-material",
        "--store",
        &plain,
        "--environment",
        ENVIRONMENT,
        "--designation",
        "S355J2",
    ]);
    let entry = at(&defined, &["definition"]);
    assert_eq!(text(entry, "kind"), "material");
    assert_eq!(text(entry, "grade"), "S355J2");
    assert_eq!(text(entry, "standard"), "EN 10025-2:2019");
    assert_eq!(at(entry, &["density_kg_m3"]).float().unwrap(), 7850.0);

    // The entry's own values combine with nothing: no --density beside a library.
    assert_eq!(
        f.code(&[
            "define-material",
            "--store",
            &plain,
            "--environment",
            ENVIRONMENT,
            "--designation",
            "S355J2",
            "--density",
            "7850",
        ]),
        "INVALID_ARGUMENT"
    );
    // And an unknown grade is refused, never substituted.
    assert_eq!(
        f.code(&[
            "define-material",
            "--store",
            &plain,
            "--environment",
            ENVIRONMENT,
            "--designation",
            "S690",
        ]),
        "NOT_FOUND"
    );
}
