// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Australia environment holds the real AS/NZS 3679.1:2016 hot-rolled and AS/NZS
// 1163:2016 hollow-section nominal dimensions, re-entered by hand as our own data; provenance and
// licence are in `ATTRIBUTION.md` and in the libraries' own metadata. The expected areas and
// masses in this file are the published values.

//! The regional `australia` environment end to end through the real `beamcraft` binary: it is
//! listed with its provenance, a project created with it resolves its designations through the
//! copy in the project folder, the declared catalogue area drives the quantities, and a
//! designation the shipped series does not publish is refused instead of silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{Scripted, at, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/australia/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a4";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("australia-environment-tests"),
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
fn the_shipped_australia_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    let listed = f.ok(&["list-library", "--environment", ENVIRONMENT]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["environment"]), "id"), "australia");
    assert_eq!(num(&listed, &["entry_count"]), 186.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let libraries = at(&listed, &["libraries"]).array().unwrap();
    assert_eq!(libraries.len(), 2);
    let info = at(&libraries[0], &["library"]);
    assert_eq!(text(info, "id"), "australia-asnzs-3679-profiles");
    assert_eq!(
        text(info, "title"),
        "Australia hot-rolled sections (AS/NZS 3679.1: UB, UC)"
    );
    assert_eq!(text(info, "version"), "1");
    assert_eq!(text(info, "licence"), "MPL-2.0");
    assert!(!text(info, "source").is_empty(), "the source is declared");
    let hollows = at(&libraries[1], &["library"]);
    assert_eq!(text(hollows, "id"), "australia-asnzs-1163-hollows");
    assert_eq!(
        text(hollows, "title"),
        "Australia hollow sections (AS/NZS 1163: CHS, RHS, SHS)"
    );
    assert_eq!(text(hollows, "licence"), "MPL-2.0");
    assert!(
        !text(hollows, "source").is_empty(),
        "the source is declared"
    );
    // The pack is larger than the default reply page: the first page is the default 64 entries
    // and names the offset the next page starts at.
    let entries = at(&listed, &["entries"]).array().unwrap();
    assert_eq!(entries.len(), 64);
    assert_eq!(num(&listed, &["next_offset"]), 64.0);
    assert_eq!(num(&listed, &["page_size"]), 64.0);
    // One page at the reply cap holds the whole pack, hot-rolled and hollows together.
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    let entries = at(&listed, &["entries"]).array().unwrap();
    assert_eq!(entries.len(), 186);
    assert_eq!(at(&listed, &["next_offset"]), &Json::Null);
    let ub610 = entries
        .iter()
        .find(|e| text(e, "designation") == "610UB125")
        .unwrap();
    assert_eq!(text(ub610, "library_id"), "australia-asnzs-3679-profiles");
    assert_eq!(num(ub610, &["parameters_mm", "h"]), 611.6);
    assert_eq!(num(ub610, &["parameters_mm", "tw"]), 11.9);
    let shs100 = entries
        .iter()
        .find(|e| text(e, "designation") == "SHS100x100x3")
        .unwrap();
    assert_eq!(text(shs100, "library_id"), "australia-asnzs-1163-hollows");
    assert_eq!(num(shs100, &["parameters_mm", "h"]), 100.0);
    assert_eq!(num(shs100, &["parameters_mm", "t"]), 3.0);
    assert_eq!(num(shs100, &["parameters_mm", "r"]), 6.0);
}

#[test]
fn a_project_created_with_the_australia_environment_resolves_its_designations() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Australia Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "australia",
    ]);
    let store = text(at(&created, &["project"]), "path");

    // The designation round rides one process: a designation of the pack resolves through
    // the project's own environment copy, with no --library option naming any file, and
    // every catalogue figure below is the published value.
    let cases: [(&str, &str, &str, &str, f64, f64); 4] = [
        // profile, start, end, kind, catalogue area mm2, mass kg (area x length x 7850 kg/m3).
        ("310UB32.0", "0,0,0", "6000,0,0", "beam", 4080.0, 192.168),
        // A heavy column-series designation.
        (
            "310UC158",
            "4000,0,0",
            "4000,0,6000",
            "column",
            20100.0,
            946.71,
        ),
        // A hollow-series designation.
        (
            "SHS100x100x3",
            "0,4000,0",
            "0,4000,6000",
            "column",
            1140.0,
            53.694,
        ),
        // A circular hollow designation.
        (
            "CHS76.1x3.2",
            "0,0,6000",
            "6000,0,6000",
            "beam",
            733.0,
            34.5243,
        ),
    ];
    // A designation the shipped series does not publish is refused, not silently substituted:
    // 530UB72.4 and the 350UC sizes are no longer rolled, the older designation generation
    // (310UC97.7, 150UB17.1) has been renamed in the 2016 assortment, and the hollow sizes the
    // mill lists stock but the standard's Appendix D does not publish stay out of the pack.
    let absent = ["530UB72.4", "SHS300x300x8"];
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
    // Replies 0..4 carry the members, 4..6 the refusals (the refused lines commit nothing).
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
            text(&replies[4 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }

    // The model is self-contained: a fresh process reopens the store and reports the same
    // catalogue figures without the environment being named.
    let inspected = f.ok(&["inspect-elements", "--store", &store]);
    let members = at(&inspected, &["members"]).array().unwrap();
    assert_eq!(members.len(), 4);
    for member in members {
        let stored = at(member, &["stored_quantities"]);
        assert_eq!(text(at(stored, &["volume"]), "freshness"), "current");
        assert_eq!(text(at(stored, &["mass"]), "freshness"), "current");
    }
}
