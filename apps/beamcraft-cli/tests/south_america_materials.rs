// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped South America environment lists the NCh203 material grades
// (`materials-nch203.usda`): the complete grade set of NCh203.Of2006, the official Chilean
// structural steel standard, in its national designations — the general-construction class ES
// (A240ES, A270ES, A345ES, M345ES, Y345ES) and the dynamic-load class ESP (A250ESP, A345ESP) —
// re-entered by hand as our own data; the density is the 7850 kg/m3 of the shipped profile packs.
// The profile pack beside it is the subject of `south_america_environment.rs`. Provenance and
// licence are in `ATTRIBUTION.md` and in the library's own metadata.

//! The material library of the `south-america` environment end to end through the real
//! `beamcraft` binary (#711): the grades-only environment carries its material library into a
//! project copy, a national grade resolves through it without any option, the mass math uses the
//! catalogue entry's density, the seeded constants stay the neutral fallback, and a grade the set
//! does not publish — the ASTM designations of the standard's informative Annex C relations and
//! the annulled NCh203.Of77 designations included — stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/south-america/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000e8";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("south-america-materials-tests"),
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
fn the_south_america_environment_lists_and_carries_its_material_library() {
    let f = Fixture::new();

    // The project carries the material library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "South America Grades",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "south-america",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("materials-nch203.usda")
            .is_file(),
        "the project copy holds the NCh203 material library"
    );

    // The picker listing is read in its own process: with the W-shape profile library beside
    // the material library a `definitions` reply carries every environment profile, so the
    // one-script summary of #738 P7 would keep only the replies that fit its own budget. The
    // store round stays one script; every shipped grade is listed for pickers until the model
    // defines it: the complete set of the standard's two classes, in the loader's order.
    let listed_before = f.ok(&["definitions", "--store", &store]);
    let available = list(&listed_before, "environment_materials");
    let offered: Vec<String> = available.iter().map(|g| text(g, "grade")).collect();
    assert_eq!(
        offered,
        [
            "A240ES", "A250ESP", "A270ES", "A345ES", "A345ESP", "M345ES", "Y345ES",
        ]
    );

    // The store round rides one script: the two plates whose mass math uses the catalogue
    // density (a 200 x 3000 x 10 plate at 7850 kg/m3 weighs exactly 47.1 kg) and the
    // refusals in their authored order.
    let mut lines = vec![
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
            "A270ES",
            "--actor-id",
            ACTOR,
        ]),
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
    ];
    // A grade the set does not publish stays NOT_FOUND, never a near match: no ASTM
    // designation of the standard's informative Annex C relations, no designation of the
    // annulled NCh203.Of77 edition, no bare strength level, no full designation carrying the
    // optional condition letters and no lowercase variant.
    let absent = [
        "A36",
        "A572 Grade 50",
        "A37-24ES",
        "A270",
        "A270ES N",
        "a270es",
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
    let plate = &replies[0];
    let mass = at(plate, &["plate", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    let kg = at(mass, &["kg"]).float().unwrap();
    assert!((kg - 47.1).abs() < 0.001, "mass {kg} kg");
    let steel_plate = &replies[1];
    let steel_mass = at(steel_plate, &["plate", "quantities", "mass"]);
    let steel_kg = at(steel_mass, &["kg"]).float().unwrap();
    assert!((steel_kg - 47.1).abs() < 0.001, "Steel mass {steel_kg} kg");
    // Replies 2..8 carry the refusals in the authored order.
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[2 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }

    // The pinned definition is the catalogue entry itself: the declared grade, standard and
    // density ride into the model, which never needs the library file again. The reads ride
    // their own processes, after the script above.
    let pinned = f.ok(&["definitions", "--store", &store, "--name", "A270ES"]);
    let pinned_defs = list(&pinned, "definitions");
    assert_eq!(pinned_defs.len(), 1);
    assert_eq!(text(&pinned_defs[0], "kind"), "material");
    assert_eq!(text(&pinned_defs[0], "grade"), "A270ES");
    assert_eq!(text(&pinned_defs[0], "standard"), "NCh203.Of2006");
    assert_eq!(
        at(&pinned_defs[0], &["density_kg_m3"]).float().unwrap(),
        7850.0
    );
    let listed_after = f.ok(&["definitions", "--store", &store]);
    let remaining = list(&listed_after, "environment_materials");
    assert_eq!(remaining.len(), 6);
    assert!(
        remaining.iter().all(|g| text(g, "grade") != "A270ES"),
        "A270ES is spent"
    );
    let pinned_steel = f.ok(&["definitions", "--store", &store, "--name", "Steel"]);
    let pinned_steel_defs = list(&pinned_steel, "definitions");
    assert_eq!(pinned_steel_defs.len(), 1);
    assert!(
        at(&pinned_steel_defs[0], &["grade"]).string().is_err(),
        "the neutral Steel declares no grade"
    );
}

#[test]
fn define_material_takes_an_nch203_grade_from_its_environment_entry() {
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
        "A345ESP",
    ]);
    let entry = at(&defined, &["definition"]);
    assert_eq!(text(entry, "kind"), "material");
    assert_eq!(text(entry, "grade"), "A345ESP");
    assert_eq!(text(entry, "standard"), "NCh203.Of2006");
    assert_eq!(at(entry, &["density_kg_m3"]).float().unwrap(), 7850.0);

    // And an unknown grade is refused, never substituted: the ASTM designation of the Annex C
    // relation of the shipped A345ESP and a designation of the annulled 1977 edition included.
    assert_eq!(
        f.code(&[
            "define-material",
            "--store",
            &plain,
            "--environment",
            ENVIRONMENT,
            "--designation",
            "A572 Grade 50",
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
            "A37-24ES",
        ]),
        "NOT_FOUND"
    );
}
