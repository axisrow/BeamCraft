// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Middle East environment holds the EN 10365:2017 nominal dimensions of the IPE, HEA,
// HEB, HL, HLZ, HD and HP series (the rows of the `europe` pack) and the ASTM grades A36, A572
// and A992 (the rows of the `usa` pack), re-entered by hand as our own data; provenance and
// licence are in `ATTRIBUTION.md` and in the libraries' own metadata. The expected areas and
// masses in this file are the published values.

//! The regional `middle-east` environment end to end through the real `beamcraft` binary: it is
//! listed with its provenance, a project created with it resolves its section designations and
//! its ASTM grades through the copy in the project folder, the declared catalogue figures drive
//! the quantities, and a designation or grade the pack does not ship is refused instead of
//! silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/middle-east/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000e8";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("middle-east-environment-tests"),
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
    /// One `run-script` invocation with the same clean environment `raw` gives its spawns:
    /// the store commands amortize the environment copy's library parse into one process
    /// (#738 P7), the commands' own work unchanged.
    fn script(&self, store: &str, lines: &[Scripted]) -> Vec<Json> {
        support::script_lines_without_config(&self.shared.root, store, lines)
    }
}

fn close(got: f64, want: f64, tolerance: f64) {
    assert!((got - want).abs() <= tolerance, "{got} vs {want}");
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

#[test]
fn the_shipped_middle_east_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    // --limit names one page for the whole pack, which at one hundred seventy-five entries
    // spans several default pages of 64.
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    assert_eq!(text(&listed, "outcome"), "library");
    let environment = at(&listed, &["environment"]);
    assert_eq!(text(environment, "id"), "middle-east");
    assert_eq!(text(environment, "version"), "1");
    let listed_libraries = |key: &str| at(&listed, &["environment", key]).array().unwrap();
    assert_eq!(
        listed_libraries("libraries"),
        [Json::text("profiles-en-10365.usda")]
    );
    assert_eq!(
        listed_libraries("material_libraries"),
        [Json::text("materials-astm.usda")]
    );
    assert!(
        listed_libraries("fastener_libraries").is_empty(),
        "the pack lists no fastener library"
    );
    assert_eq!(num(&listed, &["entry_count"]), 175.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let libraries = at(&listed, &["libraries"]).array().unwrap();
    assert_eq!(libraries.len(), 1);
    let info = at(&libraries[0], &["library"]);
    assert_eq!(text(info, "id"), "middle-east-en-10365-profiles");
    assert_eq!(
        text(info, "title"),
        "Middle East hot-rolled sections (EN 10365: IPE, HEA, HEB, HL, HLZ, HD, HP)"
    );
    assert_eq!(text(info, "version"), "2");
    assert_eq!(text(info, "licence"), "MPL-2.0");
    assert!(!text(info, "source").is_empty(), "the source is declared");
    let entries = at(&listed, &["entries"]).array().unwrap();
    assert_eq!(entries.len(), 175);
    let ipe300 = entries
        .iter()
        .find(|e| text(e, "designation") == "IPE300")
        .unwrap();
    assert_eq!(text(ipe300, "library_id"), "middle-east-en-10365-profiles");
    assert_eq!(num(ipe300, &["parameters_mm", "h"]), 300.0);
    assert_eq!(num(ipe300, &["parameters_mm", "tw"]), 7.1);
    let hea340 = entries
        .iter()
        .find(|e| text(e, "designation") == "HEA340")
        .unwrap();
    assert_eq!(num(hea340, &["parameters_mm", "h"]), 330.0);
    assert_eq!(num(hea340, &["parameters_mm", "b"]), 300.0);
}

#[test]
fn a_project_created_with_the_middle_east_environment_resolves_its_designations_and_grades() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Middle East Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "middle-east",
    ]);
    let store = text(at(&created, &["project"]), "path");
    // The project carries both libraries in its own environment copy.
    for library in ["profiles-en-10365.usda", "materials-astm.usda"] {
        assert!(
            std::path::Path::new(&store)
                .join("environment")
                .join(library)
                .is_file(),
            "the project copy holds {library}"
        );
    }

    // The designation and grade round rides one process: every name resolves through the
    // project's own environment copy, with no --library option naming any file, and every
    // catalogue figure below is the published value.
    // profile, kind, start, end, material, catalogue area mm2, mass kg (area x length x 7850
    // kg/m3).
    let cases: [(&str, &str, &str, &str, &str, f64, f64); 4] = [
        (
            "IPE300", "beam", "0,0,0", "6000,0,0", "A36", 5381.0, 253.4451,
        ),
        (
            "HEA340",
            "column",
            "4000,3000,0",
            "4000,3000,6000",
            "A572 Grade 50",
            13350.0,
            628.785,
        ),
        (
            "HEB100",
            "column",
            "8000,3000,0",
            "8000,3000,4000",
            "A992",
            2600.0,
            81.64,
        ),
        // A designation of the #790 extension: the HL series the EN tables publish without a
        // root radius.
        (
            "HL920x491",
            "beam",
            "12000,0,0",
            "18000,0,0",
            "A36",
            62300.0,
            2934.33,
        ),
    ];
    // A name the pack does not ship is refused, not silently substituted: a designation the
    // series does not publish and a size beyond the shipped range, the HEM series the
    // extension leaves out and the channels it leaves to the `europe` pack; a grade of another
    // ASTM specification, the compact industry shorthand of A572 and the EN 10025 grades,
    // which the curated mix does not carry.
    let absent: [(&str, &str); 7] = [
        ("profile", "IPE250"),
        ("profile", "HEA1100"),
        ("profile", "HEM300"),
        ("profile", "UPE100"),
        ("material", "A709"),
        ("material", "A572Gr50"),
        ("material", "S355J2"),
    ];
    let member = |scripted: fn(&[&str]) -> Scripted,
                  kind: &str,
                  start: &str,
                  end: &str,
                  profile: &str,
                  material: &str| {
        scripted(&[
            "create-member",
            "--store",
            &store,
            "--kind",
            kind,
            "--start",
            start,
            "--end",
            end,
            "--profile",
            profile,
            "--material",
            material,
        ])
    };
    let mut lines: Vec<Scripted> = cases
        .iter()
        .map(|(profile, kind, start, end, material, _, _)| {
            member(Scripted::ok, kind, start, end, profile, material)
        })
        .collect();
    // The pinned definition is the catalogue entry itself: the grade, the edition the regional
    // producer prints and the density ride into the model.
    lines.push(Scripted::ok(&[
        "definitions",
        "--store",
        &store,
        "--name",
        "A992",
    ]));
    for (namespace, designation) in absent {
        let (profile, material) = if namespace == "profile" {
            (designation, "Steel")
        } else {
            ("IPE300", designation)
        };
        lines.push(member(
            Scripted::refused,
            "beam",
            "12000,0,0",
            "18000,0,0",
            profile,
            material,
        ));
    }
    let replies = f.script(&store, &lines);
    // Replies 0..3 carry the members and reply 3 the pinned definition.
    for (index, (profile, _, _, _, _, area, mass)) in cases.iter().enumerate() {
        let made = &replies[index];
        assert_eq!(text(made, "outcome"), "member_created", "{profile}");
        let q = at(made, &["member", "quantities"]).clone();
        assert_eq!(text(&q, "area_source"), "catalogue", "{profile}");
        assert_eq!(num(&q, &["section_area_mm2"]), *area, "{profile}");
        close(num(&q, &["mass", "kg"]), *mass, 1e-9);
    }
    let pinned = list(&replies[cases.len()], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "material");
    assert_eq!(text(&pinned[0], "grade"), "A992");
    assert_eq!(text(&pinned[0], "standard"), "ASTM A992/A992M-20");
    assert_eq!(at(&pinned[0], &["density_kg_m3"]).float().unwrap(), 7850.0);
    // The replies after the members and the pinned definition carry the refusals in the
    // authored order (the refused lines commit nothing).
    for (index, (_, designation)) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[cases.len() + 1 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }

    // The model is self-contained: a fresh process reopens the store and reports the same
    // catalogue figures without the environment being named.
    let inspected = f.ok(&["inspect-elements", "--store", &store]);
    let members = at(&inspected, &["members"]).array().unwrap();
    assert_eq!(members.len(), cases.len());
    for member in members {
        let stored = at(member, &["stored_quantities"]);
        assert_eq!(text(at(stored, &["volume"]), "freshness"), "current");
        assert_eq!(text(at(stored, &["mass"]), "freshness"), "current");
    }
}
