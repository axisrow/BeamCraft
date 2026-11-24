// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Brazil environment holds the real ASTM A6/A6M-05a W and HP nominal dimensions (the
// standard's own SI column, the rows of the `usa` pack), re-entered as our own data under the
// designations the regional producers print and the ABNT NBR 15980 standard they declare, with
// the Nucor-Yamato catalog as the radius source and cross-check, and the NBR 7007 grades the
// open public listings corroborate; provenance and licence are in `ATTRIBUTION.md` and in the
// libraries' own metadata. The expected areas and masses in this file are the standard's
// published values.

//! The regional `brazil` environment end to end through the real `beamcraft` binary (#710): it
//! is listed with its provenance, a project created with it resolves the producers' metric
//! designations and the shipped NBR 7007 grades through the copy in the project folder, the
//! declared catalogue figures drive the quantities, and a designation or grade the pack does
//! not ship is refused instead of silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/brazil/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-00000000a710";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("brazil-environment-tests"),
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
fn the_shipped_brazil_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    // The environment lists 284 entries, over the 256-entry page cap, so the second page
    // completes the listing before the per-entry assertions.
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["environment"]), "id"), "brazil");
    assert_eq!(text(at(&listed, &["environment"]), "version"), "1");
    assert_eq!(num(&listed, &["entry_count"]), 284.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let listed_libraries = |key: &str| at(&listed, &["environment", key]).array().unwrap();
    assert_eq!(
        listed_libraries("libraries"),
        [Json::text("profiles-abnt-nbr-15980.usda")]
    );
    assert_eq!(
        listed_libraries("material_libraries"),
        [Json::text("materials-nbr-7007.usda")]
    );
    let libraries = at(&listed, &["libraries"]).array().unwrap();
    assert_eq!(libraries.len(), 1);
    let info = at(&libraries[0], &["library"]);
    assert_eq!(text(info, "id"), "brazil-abnt-nbr-15980-profiles");
    assert_eq!(
        text(info, "title"),
        "Brazil hot-rolled sections (ABNT NBR 15980: W, HP)"
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
    assert_eq!(num(&rest, &["entry_count"]), 284.0);
    entries.extend(at(&rest, &["entries"]).array().unwrap().iter().cloned());
    assert_eq!(entries.len(), 284);
    // Every catalogue figure below is the standard's published SI value; the radius is the
    // Nucor-Yamato catalog's, in millimetres.
    let entry = |designation: &str| {
        entries
            .iter()
            .find(|e| text(e, "designation") == designation)
            .unwrap_or_else(|| panic!("{designation} is not listed"))
    };
    let top = entry("W 1100 x 499,0");
    assert_eq!(text(top, "library_id"), "brazil-abnt-nbr-15980-profiles");
    assert_eq!(num(top, &["parameters_mm", "h"]), 1118.0);
    assert_eq!(num(top, &["parameters_mm", "b"]), 405.0);
    assert_eq!(num(top, &["parameters_mm", "tw"]), 26.0);
    assert_eq!(num(top, &["parameters_mm", "tf"]), 45.0);
    assert_eq!(num(top, &["parameters_mm", "r"]), 30.0);
    assert_eq!(num(top, &["area_mm2"]), 63500.0);
    assert_eq!(num(top, &["mass_per_meter_kg"]), 499.0);
    // Where the standards' area and the outline agree within one percent the warning is null.
    assert_eq!(top.get("declared_area_warning").unwrap(), &Json::Null);
    // The standards print no radius and round their SI dimensions, so on part of the rows the
    // declared area sits more than one percent from the closed-form outline even with the
    // catalog radius: reported, never a refusal (owner decision of 2026-10-18). W 610 x 101,0
    // is the widest case, exactly as in the `usa` pack.
    let warning = at(entry("W 610 x 101,0"), &["declared_area_warning"]);
    assert_eq!(num(warning, &["declared_mm2"]), 13000.0);
    close(num(warning, &["relative_difference"]), 0.0431, 0.001);
    assert_eq!(num(warning, &["threshold"]), 0.01);
    assert_eq!(num(warning, &["threshold_percent"]), 1.0);
    // A shape the catalog does not roll carries a sharp corner: radius zero.
    assert_eq!(num(entry("W 100 x 19,3"), &["parameters_mm", "r"]), 0.0);
    // The bearing-pile series rides the same library as i_shape rows, under the metric depth
    // designation of the annex (HP14 is HP 360) in the producers' print form.
    let pile = entry("HP 360 x 174,0");
    assert_eq!(num(pile, &["parameters_mm", "h"]), 361.0);
    assert_eq!(num(pile, &["parameters_mm", "b"]), 378.0);
    assert_eq!(num(pile, &["area_mm2"]), 22200.0);
    assert_eq!(num(pile, &["mass_per_meter_kg"]), 174.0);
    // The W 36 X 800 designation cell prints its area before its weight in both public copies;
    // the row is read from its metric designation W920 X 1191.
    let jumbo = entry("W 920 x 1191,0");
    assert_eq!(num(jumbo, &["area_mm2"]), 152500.0);
    assert_eq!(num(jumbo, &["mass_per_meter_kg"]), 1191.0);
    assert_eq!(num(jumbo, &["parameters_mm", "r"]), 0.0);
    // The row that keeps the W920 X 345 designation the annex prints twice.
    let shared = entry("W 920 x 345,0");
    assert_eq!(num(shared, &["parameters_mm", "h"]), 943.0);
    assert_eq!(num(shared, &["area_mm2"]), 44000.0);
}

#[test]
fn a_project_created_with_the_brazil_environment_resolves_its_designations() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Brazil Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "brazil",
    ]);
    let store = text(at(&created, &["project"]), "path");
    for library in ["profiles-abnt-nbr-15980.usda", "materials-nbr-7007.usda"] {
        assert!(
            std::path::Path::new(&store)
                .join("environment")
                .join(library)
                .is_file(),
            "the project copy holds {library}"
        );
    }

    // The designation round rides one process: the producers' metric designation resolves
    // through the project's own environment copy, with no --library option naming any file,
    // and every catalogue figure below is the standard's published value. The first member
    // takes the NBR 7007 grade the pack ships; the others weigh with the neutral Steel.
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
            "W 310 x 60,0",
            "beam",
            "0,0,0",
            "6000,0,0",
            "AR 350",
            7610.0,
            358.431,
        ),
        (
            "W 1100 x 499,0",
            "column",
            "4000,0,0",
            "4000,0,4000",
            "Steel",
            63500.0,
            1993.9,
        ),
        // The sharp-corner row: no catalog radius, the declared area still wins.
        (
            "W 100 x 19,3",
            "beam",
            "8000,0,0",
            "10000,0,0",
            "Steel",
            2470.0,
            38.779,
        ),
        (
            "HP 360 x 174,0",
            "column",
            "12000,0,0",
            "12000,0,6000",
            "Steel",
            22200.0,
            1045.62,
        ),
        // The row that keeps the W920 X 345 designation the annex prints twice (W36X232).
        (
            "W 920 x 345,0",
            "column",
            "16000,0,0",
            "16000,0,3000",
            "Steel",
            44000.0,
            1036.2,
        ),
    ];
    // A designation or grade the pack does not ship is refused, not silently substituted: not
    // the EDI inch form of a shipped row (the library has no aliases; it resolves through the
    // `usa` pack), not a mangled designation, not the two light W 150 rows Gerdau rolls outside
    // both standards, not the angle and flat-bar families of the regional producers, not an EN
    // designation; and no grade the library does not carry — not the compact spelling the
    // current Gerdau print runs together, not the light grade one standards-database listing
    // names and no producer corroborates, not the ASTM names of the producers' pages.
    let absent: [(&str, &str); 8] = [
        ("profile", "W12X40"),
        ("profile", "W 310 x 60"),
        ("profile", "W 150 x 9,2"),
        ("profile", "L 102 x 102 x 12,7"),
        ("profile", "BT 100 x 10"),
        ("profile", "IPE300"),
        ("material", "AR350"),
        ("material", "BR 190"),
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
    for (namespace, designation) in absent {
        let (profile, material) = if namespace == "profile" {
            (designation, "Steel")
        } else {
            ("W 310 x 60,0", designation)
        };
        lines.push(member(
            Scripted::refused,
            "beam",
            "20000,0,0",
            "26000,0,0",
            profile,
            material,
        ));
    }
    // The ASTM grade a `usa` project can use is refused here too, in its own run.
    lines.push(member(
        Scripted::refused,
        "beam",
        "26000,0,0",
        "32000,0,0",
        "W 310 x 60,0",
        "A572 Grade 50",
    ));
    let replies = f.script(&store, &lines);
    // Replies 0..5 carry the members, the rest the refusals (the refused lines commit nothing).
    for (index, (profile, _, _, _, _, area, mass)) in cases.iter().enumerate() {
        let made = &replies[index];
        assert_eq!(text(made, "outcome"), "member_created", "{profile}");
        let q = at(made, &["member", "quantities"]).clone();
        assert_eq!(text(&q, "area_source"), "catalogue", "{profile}");
        assert_eq!(num(&q, &["section_area_mm2"]), *area, "{profile}");
        close(num(&q, &["mass", "kg"]), *mass, 1e-9);
    }
    for (index, (_, designation)) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[cases.len() + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
    assert_eq!(
        text(&replies[cases.len() + absent.len()], "code"),
        "NOT_FOUND",
        "A572 Grade 50"
    );

    // The pinned definition is the catalogue entry itself: the grade, the edition the public
    // listings name and the density ride into the model.
    let pinned = f.ok(&["definitions", "--store", &store, "--name", "AR 350"]);
    let grades = list(&pinned, "definitions");
    assert_eq!(grades.len(), 1);
    assert_eq!(text(&grades[0], "kind"), "material");
    assert_eq!(text(&grades[0], "grade"), "AR 350");
    assert_eq!(text(&grades[0], "standard"), "ABNT NBR 7007:2022");
    assert_eq!(at(&grades[0], &["density_kg_m3"]).float().unwrap(), 7850.0);

    // The spent designations leave the picker list; the other 279 stay offered, beside the
    // nineteen generic profiles of the `blank` fallback layer every chain reaches. The listing
    // is read in its own process: it carries every environment profile.
    let listed = f.ok(&["definitions", "--store", &store]);
    let offered = list(&listed, "environment_profiles");
    assert_eq!(offered.len(), 284 + 19 - 5);
    assert!(
        offered
            .iter()
            .all(|p| text(p, "designation") != "W 310 x 60,0"),
        "the spent designation is no longer offered"
    );
    assert!(
        offered
            .iter()
            .any(|p| text(p, "designation") == "W 1100 x 433,0"),
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
