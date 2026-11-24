// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Philippines environment holds the ASTM A6/A6M-05a W and HP facts the `usa` pack
// re-enters, re-declared for the region (ESC Steel Philippines prints no dimension tables), and
// the grade designations the ESC page names with their publicly documented editions; provenance
// and licence are in `ATTRIBUTION.md` and in the libraries' own metadata. The expected areas and
// masses in this file are the standard's published values.

//! The regional `philippines` environment end to end through the real `beamcraft` binary
//! (#713): it is listed with its provenance, a project created with it resolves its inch
//! designations and its ESC-named grades through the copy in the project folder, the declared
//! catalogue figures drive the quantities, and a designation or grade the pack does not ship
//! is refused instead of silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/philippines/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000713";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("philippines-environment-tests"),
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
fn the_shipped_philippines_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    // The environment lists the 285 A6/A6M-05a rows of the `usa` pack, over the 256-entry
    // page cap, so the second page completes the listing before the per-entry assertions.
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    assert_eq!(text(&listed, "outcome"), "library");
    let environment = at(&listed, &["environment"]);
    assert_eq!(text(environment, "id"), "philippines");
    assert_eq!(text(environment, "version"), "1");
    let listed_libraries = |key: &str| at(environment, &[key]).array().unwrap();
    assert_eq!(
        listed_libraries("libraries"),
        [Json::text("profiles-astm-a6-w-hp.usda")]
    );
    assert_eq!(
        listed_libraries("material_libraries"),
        [Json::text("materials-philippines.usda")]
    );
    assert!(
        listed_libraries("fastener_libraries").is_empty(),
        "the pack lists no fastener library"
    );
    assert_eq!(num(&listed, &["entry_count"]), 285.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let libraries = at(&listed, &["libraries"]).array().unwrap();
    assert_eq!(libraries.len(), 1);
    let info = at(&libraries[0], &["library"]);
    assert_eq!(text(info, "id"), "philippines-astm-a6-w-hp-profiles");
    assert_eq!(
        text(info, "title"),
        "Philippines hot-rolled sections (ASTM A6/A6M: W, HP)"
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
    // The re-declared rows are the `usa` pack's: every designation with the same catalogue
    // figures; the spot checks below are the standard's printed SI values, the radius the
    // Nucor-Yamato catalog's, in millimetres.
    let entry = |designation: &str| {
        entries
            .iter()
            .find(|e| text(e, "designation") == designation)
            .unwrap_or_else(|| panic!("{designation} is not listed"))
    };
    let w44 = entry("W44X335");
    assert_eq!(text(w44, "library_id"), "philippines-astm-a6-w-hp-profiles");
    assert_eq!(num(w44, &["parameters_mm", "h"]), 1118.0);
    assert_eq!(num(w44, &["parameters_mm", "b"]), 405.0);
    assert_eq!(num(w44, &["parameters_mm", "tw"]), 26.0);
    assert_eq!(num(w44, &["parameters_mm", "tf"]), 45.0);
    assert_eq!(num(w44, &["parameters_mm", "r"]), 30.0);
    assert_eq!(num(w44, &["area_mm2"]), 63500.0);
    assert_eq!(num(w44, &["mass_per_meter_kg"]), 499.0);
    // Where the standard's area and the outline agree within one percent the warning is null;
    // W24X68 is the widest warned case, reported, never a refusal (owner decision of
    // 2026-10-18, carried over with the facts).
    assert_eq!(w44.get("declared_area_warning").unwrap(), &Json::Null);
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
    assert_eq!(
        text(hp14, "library_id"),
        "philippines-astm-a6-w-hp-profiles"
    );
    assert_eq!(num(hp14, &["parameters_mm", "h"]), 361.0);
    assert_eq!(num(hp14, &["parameters_mm", "b"]), 378.0);
    assert_eq!(num(hp14, &["area_mm2"]), 22200.0);
    assert_eq!(num(hp14, &["mass_per_meter_kg"]), 174.0);
    // The W 36 X 800 designation cell prints its area before its weight in the public
    // copies; the row is read from its metric designation W920 X 1191.
    let w36 = entry("W36X800");
    assert_eq!(num(w36, &["area_mm2"]), 152500.0);
    assert_eq!(num(w36, &["mass_per_meter_kg"]), 1191.0);
    assert_eq!(num(w36, &["parameters_mm", "r"]), 0.0);
}

#[test]
fn a_project_created_with_the_philippines_environment_resolves_its_designations_and_grades() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Philippines Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "philippines",
    ]);
    let store = text(at(&created, &["project"]), "path");
    // The project carries both libraries in its own environment copy.
    for library in ["profiles-astm-a6-w-hp.usda", "materials-philippines.usda"] {
        assert!(
            std::path::Path::new(&store)
                .join("environment")
                .join(library)
                .is_file(),
            "the project copy holds {library}"
        );
    }

    // The listing and the store round ride one process: every name resolves through the
    // project's own environment copy, with no --library option naming any file, and every
    // catalogue figure below is the standard's published value. The materials ride the
    // standards the ESC page names: JIS, ASTM and EN 10025-2; all weigh with 7850 kg/m3.
    // profile, kind, start, end, material, catalogue area mm2, mass kg (area x length x 7850).
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
            "beam", "0,0,0", "6000,0,0", "W12X40", "SS400", 7610.0, 358.431,
        ),
        (
            "column",
            "4000,0,0",
            "4000,0,4000",
            "W44X335",
            "SN490C",
            63500.0,
            1993.9,
        ),
        // The sharp-corner row: no catalog radius, the declared area still wins.
        (
            "beam",
            "8000,0,0",
            "10000,0,0",
            "W4X13",
            "SM490YB",
            2470.0,
            38.779,
        ),
        (
            "column",
            "12000,0,0",
            "12000,0,6000",
            "HP14X117",
            "A36",
            22200.0,
            1045.62,
        ),
        (
            "column",
            "16000,0,0",
            "16000,0,3000",
            "W36X800",
            "S355JR",
            152500.0,
            3591.375,
        ),
    ];
    // A name the pack does not ship is refused, not silently substituted: no metric
    // designation of a shipped row (the pack names its rows in the EDI inch form only), no
    // jumbo row of the newer A6 editions, no S family of the same annex; no grade beyond the
    // ESC list — no SN490A (no grade of G 3136), no SM520B or SS540 of the full JIS sets,
    // no A992 the `usa` pack ships, no S450J0 the 2019 EN revision deleted, no bare SM400.
    let absent: [(&str, &str); 10] = [
        ("profile", "W1100X499"),
        ("profile", "W44X408"),
        ("profile", "HP16X183"),
        ("profile", "S24X121"),
        ("material", "SN490A"),
        ("material", "SM520B"),
        ("material", "SS540"),
        ("material", "A992"),
        ("material", "S450J0"),
        ("material", "SM400"),
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
    // The listing of the three-hundred-strong profile picker rides its own script (the
    // run-script summary keeps its newest replies only within the output budget), so the
    // round is four invocations: the listing, the members and pins, the listing after the
    // spending, and the refusals.
    let listed = f.script(&store, &[Scripted::ok(&["definitions", "--store", &store])]);
    let member_lines: Vec<Scripted> = cases
        .iter()
        .map(|(kind, start, end, profile, material, _, _)| {
            member(Scripted::ok, kind, start, end, profile, material)
        })
        .collect();
    let mut pinned_lines = member_lines;
    // The pinned definitions are the catalogue entries themselves: the grade, the declared
    // edition and the density ride into the model.
    pinned_lines.push(Scripted::ok(&[
        "definitions",
        "--store",
        &store,
        "--name",
        "SS400",
    ]));
    pinned_lines.push(Scripted::ok(&[
        "definitions",
        "--store",
        &store,
        "--name",
        "SN490C",
    ]));
    let replies = f.script(&store, &pinned_lines);
    let listed_after = f.script(&store, &[Scripted::ok(&["definitions", "--store", &store])]);
    let mut refused_lines: Vec<Scripted> = Vec::new();
    for (namespace, designation) in absent {
        let (profile, material) = if namespace == "profile" {
            (designation, "Steel")
        } else {
            ("W12X40", designation)
        };
        refused_lines.push(member(
            Scripted::refused,
            "beam",
            "20000,0,0",
            "26000,0,0",
            profile,
            material,
        ));
    }
    let refusals = f.script(&store, &refused_lines);
    // Reply 0 offers the complete grade list.
    let offered = list(&listed[0], "environment_materials");
    let grades: Vec<String> = offered.iter().map(|g| text(g, "grade")).collect();
    assert_eq!(
        grades,
        [
            "A36",
            "A572 Grade 42",
            "A572 Grade 50",
            "A572 Grade 55",
            "A572 Grade 60",
            "A572 Grade 65",
            "S235J0",
            "S235J2",
            "S235JR",
            "S275J0",
            "S275J2",
            "S275JR",
            "S355J0",
            "S355J2",
            "S355JR",
            "S355K2",
            "S460J0",
            "S460J2",
            "S460JR",
            "S460K2",
            "S500J0",
            "SM400A",
            "SM400B",
            "SM400C",
            "SM490A",
            "SM490B",
            "SM490C",
            "SM490YA",
            "SM490YB",
            "SN400A",
            "SN400B",
            "SN400C",
            "SN490B",
            "SN490C",
            "SS400",
        ]
    );
    // Replies 0..5 carry the members.
    for (index, (_, _, _, profile, _, area, mass)) in cases.iter().enumerate() {
        let made = &replies[index];
        assert_eq!(text(made, "outcome"), "member_created", "{profile}");
        let q = at(made, &["member", "quantities"]).clone();
        assert_eq!(text(&q, "area_source"), "catalogue", "{profile}");
        assert_eq!(num(&q, &["section_area_mm2"]), *area, "{profile}");
        close(num(&q, &["mass", "kg"]), *mass, 1e-9);
    }
    // Replies 5..7 carry the pinned definitions.
    let pinned_ss400 = list(&replies[5], "definitions");
    assert_eq!(pinned_ss400.len(), 1);
    assert_eq!(text(&pinned_ss400[0], "kind"), "material");
    assert_eq!(text(&pinned_ss400[0], "grade"), "SS400");
    assert_eq!(text(&pinned_ss400[0], "standard"), "JIS G 3101:2015");
    assert_eq!(
        at(&pinned_ss400[0], &["density_kg_m3"]).float().unwrap(),
        7850.0
    );
    let pinned_sn490c = list(&replies[6], "definitions");
    assert_eq!(pinned_sn490c.len(), 1);
    assert_eq!(text(&pinned_sn490c[0], "grade"), "SN490C");
    assert_eq!(text(&pinned_sn490c[0], "standard"), "JIS G 3136:2012");
    // The listing after the spending offers the remaining pickers: the two spent grades
    // leave the list, the spent profiles leave theirs, the other 280 A6 rows and the
    // nineteen generic profiles of the `blank` fallback layer stay.
    let remaining = list(&listed_after[0], "environment_materials");
    assert_eq!(remaining.len(), 30);
    assert!(
        remaining.iter().all(|g| {
            let grade = text(g, "grade");
            !["SS400", "SN490C", "SM490YB", "A36", "S355JR"].contains(&grade.as_str())
        }),
        "the spent grades are no longer offered"
    );
    let offered_profiles = list(&listed_after[0], "environment_profiles");
    assert_eq!(offered_profiles.len(), 299);
    assert!(
        offered_profiles
            .iter()
            .all(|p| text(p, "designation") != "W12X40"),
        "the spent designation is no longer offered"
    );
    assert!(
        offered_profiles
            .iter()
            .any(|p| text(p, "designation") == "W44X290"),
        "an unspent designation of the pack is offered"
    );
    // The refusal script carries them in the authored order (the refused lines commit
    // nothing).
    for (index, (_, designation)) in absent.iter().enumerate() {
        assert_eq!(text(&refusals[index], "code"), "NOT_FOUND", "{designation}");
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
