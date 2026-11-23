// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped South Africa environment holds the EN 10365:2017 IPE sections
// (`profiles-en-10365-ipe.usda`, the IPE rows of the `europe` pack) and the SANS 50025:2009
// structural steel grades (`materials-sans-50025.usda`), re-entered by hand as our own data.
// Provenance and licence are in `ATTRIBUTION.md` and in each library's own metadata. The
// expected areas, masses and densities in this file are the standards' published values.

//! The regional `south-africa` environment end to end through the real `beamcraft` binary
//! (#714): it is listed with its identity and provenance, a project created with it carries both
//! libraries in its own copy, an IPE designation and a SANS grade resolve through that copy with
//! no option naming a file, the declared catalogue area and density drive the quantities, and
//! what the pack does not publish — the BS UB/UC series, the other EN 10365 series, the grades
//! of the 2019 EN revision — is refused instead of silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/south-africa/environment.usda"
);
const ACTOR: &str = "0199e7a1-0000-7000-8000-000000000133";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("south-africa-environment-tests"),
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
fn the_shipped_south_africa_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    // Seventeen entries fit one page of the listing.
    let listed = f.ok(&["list-library", "--environment", ENVIRONMENT]);
    assert_eq!(text(&listed, "outcome"), "library");
    let environment = at(&listed, &["environment"]);
    assert_eq!(text(environment, "id"), "south-africa");
    assert_eq!(text(environment, "version"), "1");
    assert_eq!(
        text(environment, "title"),
        "South Africa (EN 10365 IPE sections, SANS 50025 grades)"
    );
    assert_eq!(
        at(environment, &["libraries"]).array().unwrap(),
        [Json::text("profiles-en-10365-ipe.usda")]
    );
    assert_eq!(
        at(environment, &["material_libraries"]).array().unwrap(),
        [Json::text("materials-sans-50025.usda")]
    );
    // The environment names no fastener library: its fastener list is empty, not absent.
    assert_eq!(
        at(environment, &["fastener_libraries"])
            .array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(num(&listed, &["entry_count"]), 17.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let library = &at(&listed, &["libraries"]).array().unwrap()[0];
    let info = at(library, &["library"]);
    assert_eq!(text(info, "id"), "south-africa-en-10365-ipe-profiles");
    assert_eq!(
        text(info, "title"),
        "South Africa hot-rolled sections (EN 10365: IPE)"
    );
    assert_eq!(text(info, "version"), "1");
    assert_eq!(text(info, "licence"), "MPL-2.0");
    assert!(!text(info, "source").is_empty(), "the source is declared");
    let entries = at(&listed, &["entries"]).array().unwrap();
    assert_eq!(entries.len(), 17);
    let ipe300 = entries
        .iter()
        .find(|e| text(e, "designation") == "IPE300")
        .unwrap();
    assert_eq!(
        text(ipe300, "library_id"),
        "south-africa-en-10365-ipe-profiles"
    );
    assert_eq!(num(ipe300, &["parameters_mm", "h"]), 300.0);
    assert_eq!(num(ipe300, &["parameters_mm", "tw"]), 7.1);
}

#[test]
fn a_project_created_with_the_south_africa_environment_resolves_designations_and_grades() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "South Africa Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "south-africa",
    ]);
    let store = text(at(&created, &["project"]), "path");
    let copy = std::path::Path::new(&store).join("environment");
    assert!(
        copy.join("profiles-en-10365-ipe.usda").is_file(),
        "the project copy holds the IPE library"
    );
    assert!(
        copy.join("materials-sans-50025.usda").is_file(),
        "the project copy holds the SANS 50025 material library"
    );

    // The store round rides one process: every designation resolves through the project's own
    // environment copy, with no --library option naming any file, and every catalogue figure
    // below is a published value.
    let member = |profile: &str, kind: &str, start: &str, end: &str, material: &str| {
        Scripted::ok(&[
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
    let refused = |profile: &str, material: &str| {
        Scripted::refused(&[
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
            profile,
            "--material",
            material,
        ])
    };
    // profile, start, end, kind, material, catalogue area mm2, mass kg (area x length x 7850
    // kg/m3): the S355JR of everyday practice, the S450J0 the 2004 edition the SANS part adopts
    // still publishes, and the neutral seeded Steel beside them.
    let cases: [(&str, &str, &str, &str, &str, f64, f64); 3] = [
        (
            "IPE300", "0,0,0", "6000,0,0", "beam", "S355JR", 5381.0, 253.4451,
        ),
        (
            "IPE200",
            "4000,0,0",
            "4000,0,5000",
            "column",
            "S450J0",
            2848.0,
            111.784,
        ),
        (
            "IPE300",
            "3000,0,0",
            "3000,0,5000",
            "column",
            "Steel",
            5381.0,
            211.20425,
        ),
    ];
    // A designation the pack does not publish is refused, not silently substituted: no IPE 250
    // (the series steps 240 to 270), none of the other EN 10365 series, and none of the BS UB/UC
    // series of South African practice, which is an open owner question.
    let absent_profiles = ["IPE250", "HEA200", "UB203x133x25", "UC203x203x46"];
    // A grade the set does not publish stays NOT_FOUND, never a near match: the grades the 2019
    // EN revision added, the S185 and E295 grades without a specified chemical composition, no
    // bare strength class, no lowercase and no delivery-condition suffix.
    let absent_grades = [
        "S460JR", "S500J0", "S185", "E295", "S355", "s355j2", "S355K2+N",
    ];
    let mut lines = vec![Scripted::ok(&["definitions", "--store", &store])];
    lines.extend(
        cases
            .iter()
            .map(|(profile, start, end, kind, material, _, _)| {
                member(profile, kind, start, end, material)
            }),
    );
    lines.push(Scripted::ok(&[
        "definitions",
        "--store",
        &store,
        "--name",
        "S355JR",
    ]));
    lines.push(Scripted::ok(&["definitions", "--store", &store]));
    for designation in absent_profiles {
        lines.push(refused(designation, "Steel"));
    }
    for designation in absent_grades {
        lines.push(refused("IPE300", designation));
    }
    let replies = f.script(&store, &lines);

    // Reply 0: every shipped entry is listed for pickers until the model defines it. The
    // profile pickers offer the pack's seventeen IPE designations beside the nineteen generic
    // profiles of the neutral layer every project chain ends with.
    let offered = list(&replies[0], "environment_materials");
    assert_eq!(offered.len(), 27);
    assert_eq!(text(&offered[0], "grade"), "S235J0");
    assert_eq!(text(&offered[26], "grade"), "S460NL");
    let offered_profiles: Vec<String> = list(&replies[0], "environment_profiles")
        .iter()
        .map(|p| text(p, "designation"))
        .collect();
    assert_eq!(offered_profiles.len(), 36, "{offered_profiles:?}");
    for designation in [
        "IPE100", "IPE120", "IPE140", "IPE160", "IPE180", "IPE200", "IPE220", "IPE240", "IPE270",
        "IPE300", "IPE330", "IPE360", "IPE400", "IPE450", "IPE500", "IPE550", "IPE600",
    ] {
        assert!(
            offered_profiles.contains(&designation.to_owned()),
            "{designation} is offered"
        );
    }

    // Replies 1..4 carry the members.
    for (index, (profile, _, _, _, _, area, mass)) in cases.iter().enumerate() {
        let made = &replies[1 + index];
        assert_eq!(text(made, "outcome"), "member_created", "{profile}");
        let q = at(made, &["member", "quantities"]).clone();
        assert_eq!(text(&q, "area_source"), "catalogue", "{profile}");
        assert_eq!(num(&q, &["section_area_mm2"]), *area, "{profile}");
        close(num(&q, &["mass", "kg"]), *mass, 1e-9);
    }

    // Reply 4: the spent grade is pinned as the catalogue entry itself: the declared grade,
    // standard and density ride into the model, which never needs the library file again.
    let pinned = list(&replies[4], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "material");
    assert_eq!(text(&pinned[0], "grade"), "S355JR");
    assert_eq!(text(&pinned[0], "standard"), "SANS 50025-2:2009");
    assert_eq!(at(&pinned[0], &["density_kg_m3"]).float().unwrap(), 7850.0);

    // Reply 5: the two spent grades are no longer offered; the other 25 still are.
    let remaining = list(&replies[5], "environment_materials");
    assert_eq!(remaining.len(), 25);
    assert!(
        remaining
            .iter()
            .all(|g| text(g, "grade") != "S355JR" && text(g, "grade") != "S450J0"),
        "S355JR and S450J0 are spent"
    );

    // Replies 6..16 carry the refusals in the authored order.
    for (index, designation) in absent_profiles
        .iter()
        .chain(absent_grades.iter())
        .enumerate()
    {
        assert_eq!(
            text(&replies[6 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }

    // The model is self-contained: a fresh process reopens the store and reports the same
    // catalogue figures without the environment being named.
    let inspected = f.ok(&["inspect-elements", "--store", &store]);
    let members = at(&inspected, &["members"]).array().unwrap();
    assert_eq!(members.len(), 3);
    for member in members {
        let stored = at(member, &["stored_quantities"]);
        assert_eq!(text(at(stored, &["volume"]), "freshness"), "current");
        assert_eq!(text(at(stored, &["mass"]), "freshness"), "current");
    }
}
