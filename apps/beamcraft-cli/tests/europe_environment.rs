// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Europe environment holds the real EN 10365:2017 nominal dimensions, re-entered by
// hand as our own data; provenance and licence are in `ATTRIBUTION.md` and in the library's own
// metadata. The expected areas and masses in this file are the standard's published values.

//! The regional `europe` environment end to end through the real `beamcraft` binary: it is
//! listed with its provenance, a project created with it resolves its designations through the
//! copy in the project folder, the declared catalogue area drives the quantities, and a
//! designation the series does not publish is refused instead of silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{Scripted, at, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/europe/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a2";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("europe-environment-tests"),
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

#[test]
fn the_shipped_europe_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    // The environment lists two profile libraries; the two hundred fifty-nine entries span
    // two pages of the listing (the limit caps at 256), so both are fetched.
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    let tail = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
        "--offset",
        "256",
    ]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["environment"]), "id"), "europe");
    assert_eq!(num(&listed, &["entry_count"]), 259.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let library = &at(&listed, &["libraries"]).array().unwrap()[0];
    let info = at(library, &["library"]);
    assert_eq!(text(info, "id"), "europe-en-10365-profiles");
    assert_eq!(
        text(info, "title"),
        "Europe hot-rolled sections (EN 10365: IPE, HEA, HEB, HL, HLZ, HD, HP, UPE, UPN)"
    );
    assert_eq!(text(info, "version"), "4");
    assert_eq!(text(info, "licence"), "MPL-2.0");
    assert!(!text(info, "source").is_empty(), "the source is declared");
    let first = at(&listed, &["entries"]).array().unwrap();
    let rest = at(&tail, &["entries"]).array().unwrap();
    assert_eq!(first.len() + rest.len(), 259);
    let entries: Vec<_> = first.iter().chain(rest.iter()).collect();
    let ipe300 = entries
        .iter()
        .find(|e| text(e, "designation") == "IPE300")
        .unwrap();
    assert_eq!(text(ipe300, "library_id"), "europe-en-10365-profiles");
    assert_eq!(num(ipe300, &["parameters_mm", "h"]), 300.0);
    assert_eq!(num(ipe300, &["parameters_mm", "tw"]), 7.1);
    // The UPE channels ship as the pack's parallel-flange channel series.
    let upe100 = entries
        .iter()
        .find(|e| text(e, "designation") == "UPE100")
        .unwrap();
    assert_eq!(num(upe100, &["parameters_mm", "h"]), 100.0);
    assert_eq!(num(upe100, &["parameters_mm", "b"]), 55.0);
    assert_eq!(num(upe100, &["parameters_mm", "tw"]), 4.5);
    assert_eq!(num(upe100, &["parameters_mm", "tf"]), 7.5);
    assert_eq!(num(upe100, &["parameters_mm", "r"]), 10.0);
    // The UPN channels ship as the pack's taper-flange channel series: the parameters carry
    // the web s, the mid-leg flange thickness t, both radii and the dimensionless slope.
    let upn100 = entries
        .iter()
        .find(|e| text(e, "designation") == "UPN100")
        .unwrap();
    assert_eq!(num(upn100, &["parameters_mm", "h"]), 100.0);
    assert_eq!(num(upn100, &["parameters_mm", "b"]), 50.0);
    assert_eq!(num(upn100, &["parameters_mm", "s"]), 6.0);
    assert_eq!(num(upn100, &["parameters_mm", "t"]), 8.5);
    assert_eq!(num(upn100, &["parameters_mm", "r"]), 8.5);
    assert_eq!(num(upn100, &["parameters_mm", "r1"]), 4.5);
    assert_eq!(num(upn100, &["parameters_mm", "k"]), 0.08);
    // One row of each family the #790 extension added: HL, HD and HP ship as the pack's
    // extra-wide flange series beside the deep HEA/HEB ladders.
    let hl920x491 = entries
        .iter()
        .find(|e| text(e, "designation") == "HL920x491")
        .unwrap();
    assert_eq!(num(hl920x491, &["parameters_mm", "h"]), 957.0);
    assert_eq!(num(hl920x491, &["parameters_mm", "b"]), 422.0);
    assert_eq!(num(hl920x491, &["parameters_mm", "tw"]), 25.9);
    assert_eq!(num(hl920x491, &["parameters_mm", "tf"]), 47.0);
    assert_eq!(num(hl920x491, &["parameters_mm", "r"]), 19.0);
    let hd400x818 = entries
        .iter()
        .find(|e| text(e, "designation") == "HD400x818")
        .unwrap();
    assert_eq!(num(hd400x818, &["parameters_mm", "h"]), 514.0);
    assert_eq!(num(hd400x818, &["parameters_mm", "tw"]), 60.5);
    let hp400x231 = entries
        .iter()
        .find(|e| text(e, "designation") == "HP400x231")
        .unwrap();
    assert_eq!(num(hp400x231, &["parameters_mm", "h"]), 372.0);
    assert_eq!(num(hp400x231, &["parameters_mm", "b"]), 402.0);
}

#[test]
fn a_project_created_with_the_europe_environment_resolves_its_designations() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Europe Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "europe",
    ]);
    let store = text(at(&created, &["project"]), "path");

    // The designation round rides one process: a designation of the pack resolves through
    // the project's own environment copy, with no --library option naming any file, and
    // every catalogue figure below is the standard's published value.
    let cases: [(&str, &str, &str, &str, f64, f64); 5] = [
        // profile, start, end, kind, catalogue area mm2, mass kg (area x length x 7850 kg/m3).
        ("IPE300", "0,0,0", "6000,0,0", "beam", 5381.0, 253.4451),
        // A designation of a heavier series member.
        (
            "HEA340",
            "4000,0,0",
            "4000,0,6000",
            "column",
            13350.0,
            628.785,
        ),
        // A designation of the channel series.
        ("UPE100", "8000,0,0", "14000,0,0", "beam", 1250.0, 58.875),
        // A designation of the taper-flange channel series: the declared area wins for the
        // quantities and the taper-flange outline draws without a declared-area warning.
        ("UPN100", "16000,0,0", "22000,0,0", "beam", 1350.0, 63.585),
        // A designation of the #790 extension: the HL series the EN tables publish without a
        // root radius.
        (
            "HL920x491",
            "26000,0,0",
            "32000,0,0",
            "beam",
            62300.0,
            2934.33,
        ),
    ];
    // A designation the series does not publish is refused, not silently substituted: no IPE 250
    // and no UPE 250 exist (the channel sequence steps 240 to 270), and the UPN series steps
    // 300 to 320 without a 270 or 330.
    let absent = ["IPE250", "UPE250", "UPN270", "UPN330"];
    let member = |profile: &str, kind: &str, start: &str, end: &str| {
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
            "Steel",
        ])
    };
    let mut lines: Vec<Scripted> = cases
        .iter()
        .map(|(profile, start, end, kind, _, _)| member(profile, kind, start, end))
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
    for (index, (profile, _, _, _, area, mass)) in cases.iter().enumerate() {
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

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
