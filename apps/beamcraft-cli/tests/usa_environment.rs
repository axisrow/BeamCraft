// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped USA environment holds the real ASTM A6/A6M-05a W and HP nominal dimensions (the
// standard's own SI column), re-entered by hand as our own data, with the Nucor-Yamato
// catalog as the radius source and the row cross-check; provenance and licence are in
// `ATTRIBUTION.md` and in the library's own metadata. The expected areas and masses in this
// file are the standard's published values.

//! The `usa` environment end to end through the real `beamcraft` binary (#708, profiles
//! slice): it is listed with its provenance beside its material library, a project created
//! with it resolves its designations through the copy in the project folder, the declared
//! catalogue area drives the quantities with an ASTM grade as the material, the declared-area
//! warning is reported where the standard's area sits off the outline and is null where the
//! two agree, and a designation the shipped edition does not publish is refused instead of
//! silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/usa/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-00000000a708";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("usa-environment-tests"),
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
fn the_shipped_usa_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    // The environment lists 285 entries, over the 256-entry page cap, so the second page
    // completes the listing before the per-entry assertions.
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["environment"]), "id"), "usa");
    assert_eq!(num(&listed, &["entry_count"]), 285.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    // The environment now carries one profile library beside its material library.
    let libraries = at(&listed, &["libraries"]).array().unwrap();
    assert_eq!(libraries.len(), 1);
    let info = at(&libraries[0], &["library"]);
    assert_eq!(text(info, "id"), "usa-astm-a6-w-hp-profiles");
    assert_eq!(
        text(info, "title"),
        "USA hot-rolled sections (ASTM A6/A6M: W, HP)"
    );
    assert_eq!(text(info, "version"), "1");
    assert_eq!(text(info, "licence"), "MPL-2.0");
    assert!(!text(info, "source").is_empty(), "the source is declared");
    let mut entries = at(&listed, &["entries"]).array().unwrap().to_vec();
    assert_eq!(entries.len(), 256);
    let rest = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--offset",
        "256",
        "--limit",
        "256",
    ]);
    assert_eq!(num(&rest, &["entry_count"]), 285.0);
    entries.extend(at(&rest, &["entries"]).array().unwrap().iter().cloned());
    assert_eq!(entries.len(), 285);
    // Every catalogue figure below is the standard's published SI value; the radius is the
    // Nucor-Yamato catalog's, in millimetres.
    let entry = |designation: &str| {
        entries
            .iter()
            .find(|e| text(e, "designation") == designation)
            .unwrap_or_else(|| panic!("{designation} is not listed"))
    };
    let w44 = entry("W44X335");
    assert_eq!(text(w44, "library_id"), "usa-astm-a6-w-hp-profiles");
    assert_eq!(num(w44, &["parameters_mm", "h"]), 1118.0);
    assert_eq!(num(w44, &["parameters_mm", "b"]), 405.0);
    assert_eq!(num(w44, &["parameters_mm", "tw"]), 26.0);
    assert_eq!(num(w44, &["parameters_mm", "tf"]), 45.0);
    assert_eq!(num(w44, &["parameters_mm", "r"]), 30.0);
    assert_eq!(num(w44, &["area_mm2"]), 63500.0);
    assert_eq!(num(w44, &["mass_per_meter_kg"]), 499.0);
    // Where the standard's area and the outline agree within one percent the warning is null.
    assert_eq!(w44.get("declared_area_warning").unwrap(), &Json::Null);
    // The standard prints no radius and rounds its SI dimensions, so on part of the rows the
    // declared area sits more than one percent from the closed-form outline even with the
    // catalog radius: reported, never a refusal (owner decision of 2026-10-18). W24X68 is the
    // widest case.
    let w24 = entry("W24X68");
    let warning = at(w24, &["declared_area_warning"]);
    assert_eq!(num(warning, &["declared_mm2"]), 13000.0);
    close(num(warning, &["relative_difference"]), 0.0431, 0.001);
    assert_eq!(num(warning, &["threshold"]), 0.01);
    assert_eq!(num(warning, &["threshold_percent"]), 1.0);
    // A shape the catalog does not roll carries a sharp corner: radius zero.
    assert_eq!(num(entry("W4X13"), &["parameters_mm", "r"]), 0.0);
    // The bearing-pile series rides the same library as i_shape rows.
    let hp14 = entry("HP14X117");
    assert_eq!(text(hp14, "library_id"), "usa-astm-a6-w-hp-profiles");
    assert_eq!(num(hp14, &["parameters_mm", "h"]), 361.0);
    assert_eq!(num(hp14, &["parameters_mm", "b"]), 378.0);
    assert_eq!(num(hp14, &["area_mm2"]), 22200.0);
    assert_eq!(num(hp14, &["mass_per_meter_kg"]), 174.0);
    // The W 36 X 800 designation cell prints its area before its weight in both public copies;
    // the row is read from its metric designation W920 X 1191.
    let w36 = entry("W36X800");
    assert_eq!(num(w36, &["area_mm2"]), 152500.0);
    assert_eq!(num(w36, &["mass_per_meter_kg"]), 1191.0);
    assert_eq!(num(w36, &["parameters_mm", "r"]), 0.0);
}

#[test]
fn a_project_created_with_the_usa_environment_resolves_its_designations() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "USA Model",
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
            .join("profiles-astm-a6-w-hp.usda")
            .is_file(),
        "the project copy holds the ASTM A6/A6M W and HP library"
    );

    // The designation round rides one process: a designation of the pack resolves through
    // the project's own environment copy, with no --library option naming any file, and
    // every catalogue figure below is the standard's published value. The W-shape grade
    // A992 of the environment's material library is the material of the first member, the
    // neutral Steel of the others; both weigh with 7850 kg/m3.
    // kind, start, end, profile, material, catalogue area mm2, mass kg (area x length x 7850).
    type Designation = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        f64,
        f64,
    );
    let cases: [Designation; 5] = [
        (
            "beam", "0,0,0", "6000,0,0", "W12X40", "A992", 7610.0, 358.431,
        ),
        (
            "column",
            "4000,0,0",
            "4000,0,4000",
            "W44X335",
            "Steel",
            63500.0,
            1993.9,
        ),
        // The sharp-corner row: no catalog radius, the declared area still wins.
        (
            "beam",
            "8000,0,0",
            "10000,0,0",
            "W4X13",
            "Steel",
            2470.0,
            38.779,
        ),
        (
            "column",
            "12000,0,0",
            "12000,0,6000",
            "HP14X117",
            "Steel",
            22200.0,
            1045.62,
        ),
        (
            "column",
            "16000,0,0",
            "16000,0,3000",
            "HP8X36",
            "Steel",
            6840.0,
            161.082,
        ),
    ];
    // A designation the shipped edition does not publish is refused, not silently
    // substituted: no jumbo shape of the newer editions of the standard (W44X408, HP16X183),
    // no other shape family of the same annex (S24X121), and not the metric designation of
    // a shipped row either (W1100X499): the pack names its rows in the EDI inch form only.
    let absent = ["W44X408", "HP16X183", "S24X121", "W1100X499"];
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
    let mut lines: Vec<Scripted> = cases
        .iter()
        .map(|(kind, start, end, profile, material, _, _)| {
            member(profile, kind, start, end, material)
        })
        .collect();
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
            "6000,0,0",
            "--profile",
            designation,
            "--material",
            "Steel",
        ]));
    }
    let replies = f.script(&store, &lines);
    // Replies 0..5 carry the members, 5..9 the refusals (the refused lines commit nothing).
    for (index, (_, _, _, profile, _, area, mass)) in cases.iter().enumerate() {
        let made = &replies[index];
        assert_eq!(text(made, "outcome"), "member_created", "{profile}");
        let q = at(made, &["member", "quantities"]).clone();
        assert_eq!(text(&q, "area_source"), "catalogue", "{profile}");
        assert_eq!(num(&q, &["section_area_mm2"]), *area, "{profile}");
        close(num(&q, &["mass", "kg"]), *mass, 1e-9);
    }
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[5 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
    // The spent designations leave the picker list; the other 280 stay offered, beside the
    // nineteen generic profiles of the `blank` fallback layer every chain reaches. The listing
    // is read in its own process: it carries every environment profile.
    let listed = f.ok(&["definitions", "--store", &store]);
    let offered = list(&listed, "environment_profiles");
    assert_eq!(offered.len(), 299);
    assert!(
        offered.iter().all(|p| text(p, "designation") != "W12X40"),
        "the spent designation is no longer offered"
    );
    assert!(
        offered.iter().any(|p| text(p, "designation") == "W44X290"),
        "an unspent designation of the pack is offered"
    );

    // The model is self-contained: a fresh process reopens the store and reports the same
    // catalogue figures without the environment being named.
    let inspected = f.ok(&["inspect-elements", "--store", &store]);
    let members = at(&inspected, &["members"]).array().unwrap();
    assert_eq!(members.len(), 5);
    for member in members {
        let stored = at(member, &["stored_quantities"]);
        assert_eq!(text(at(stored, &["volume"]), "freshness"), "current");
        assert_eq!(text(at(stored, &["mass"]), "freshness"), "current");
    }
}
