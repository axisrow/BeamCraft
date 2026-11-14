// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Europe environment holds the real EN 10365:2022 nominal dimensions, re-entered by
// hand as our own data; provenance and licence are in `ATTRIBUTION.md` and in the library's own
// metadata. The expected areas and masses in this file are the standard's published values.

//! The regional `europe` environment end to end through the real `beamcraft` binary: it is
//! listed with its provenance, a project created with it resolves its designations through the
//! copy in the project folder, the declared catalogue area drives the quantities, and a
//! designation the series does not publish is refused instead of silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

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
    fn code(&self, args: &[&str]) -> String {
        let (code, body) = self.raw(args);
        assert_eq!(code, 2, "{args:?}: {body:?}");
        text(&body, "code")
    }
}

fn close(got: f64, want: f64, tolerance: f64) {
    assert!((got - want).abs() <= tolerance, "{got} vs {want}");
}

#[test]
fn the_shipped_europe_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    let listed = f.ok(&["list-library", "--environment", ENVIRONMENT]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["environment"]), "id"), "europe");
    assert_eq!(num(&listed, &["entry_count"]), 55.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let library = &at(&listed, &["libraries"]).array().unwrap()[0];
    let info = at(library, &["library"]);
    assert_eq!(text(info, "id"), "europe-en-10365-profiles");
    assert_eq!(
        text(info, "title"),
        "Europe hot-rolled sections (EN 10365: IPE, HEA, HEB)"
    );
    assert_eq!(text(info, "version"), "1");
    assert_eq!(text(info, "licence"), "MPL-2.0");
    assert!(!text(info, "source").is_empty(), "the source is declared");
    let entries = at(&listed, &["entries"]).array().unwrap();
    assert_eq!(entries.len(), 55);
    let ipe300 = entries
        .iter()
        .find(|e| text(e, "designation") == "IPE300")
        .unwrap();
    assert_eq!(text(ipe300, "library_id"), "europe-en-10365-profiles");
    assert_eq!(num(ipe300, &["parameters_mm", "h"]), 300.0);
    assert_eq!(num(ipe300, &["parameters_mm", "tw"]), 7.1);
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

    // A designation of the pack resolves through the project's own environment copy, with no
    // --library option naming any file.
    let made = f.ok(&[
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
        "Steel",
    ]);
    assert_eq!(text(&made, "outcome"), "member_created");
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 5381.0);
    // 5381 mm2 x 6000 mm x 7850 kg/m3.
    close(num(&q, &["mass", "kg"]), 253.4451, 1e-9);

    // A designation of a heavier series member: HEA340, catalogue area 13350 mm2.
    let made = f.ok(&[
        "create-member",
        "--store",
        &store,
        "--kind",
        "column",
        "--start",
        "4000,0,0",
        "--end",
        "4000,0,6000",
        "--profile",
        "HEA340",
        "--material",
        "Steel",
    ]);
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 13350.0);
    close(num(&q, &["mass", "kg"]), 628.785, 1e-9);

    // A designation the series does not publish is refused, not silently substituted.
    assert_eq!(
        f.code(&[
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
            "IPE250",
            "--material",
            "Steel"
        ]),
        "NOT_FOUND"
    );

    // The model is self-contained: a fresh process reopens the store and reports the same
    // catalogue figures without the environment being named.
    let inspected = f.ok(&["inspect-elements", "--store", &store]);
    let members = at(&inspected, &["members"]).array().unwrap();
    assert_eq!(members.len(), 2);
    for member in members {
        let stored = at(member, &["stored_quantities"]);
        assert_eq!(text(at(stored, &["volume"]), "freshness"), "current");
        assert_eq!(text(at(stored, &["mass"]), "freshness"), "current");
    }
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
