// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped India environment holds the real IS 808:2021 nominal dimensions, re-entered by
// hand as our own data; provenance and licence are in `ATTRIBUTION.md` and in the library's own
// metadata. The expected areas and masses in this file are the standard's published values.

//! The regional `india` environment end to end through the real `beamcraft` binary: it is
//! listed with its provenance, a project created with it resolves its designations through the
//! copy in the project folder, the declared catalogue area drives the quantities, and a
//! designation the shipped series does not publish is refused instead of silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/india/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a5";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("india-environment-tests"),
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

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

#[test]
fn the_shipped_india_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["environment"]), "id"), "india");
    assert_eq!(num(&listed, &["entry_count"]), 191.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let libraries = at(&listed, &["libraries"]).array().unwrap();
    assert_eq!(libraries.len(), 3);
    let info = at(&libraries[0], &["library"]);
    assert_eq!(text(info, "id"), "india-is808-npb-profiles");
    assert_eq!(text(info, "version"), "1");
    assert_eq!(text(info, "licence"), "MPL-2.0");
    assert!(!text(info, "source").is_empty(), "the source is declared");
    assert_eq!(
        text(at(&libraries[1], &["library"]), "id"),
        "india-is808-wpb-100-260-profiles"
    );
    assert_eq!(
        text(at(&libraries[2], &["library"]), "id"),
        "india-is808-wpb-280-900-profiles"
    );
    let entries = at(&listed, &["entries"]).array().unwrap();
    assert_eq!(entries.len(), 191);
    let npb300 = entries
        .iter()
        .find(|e| text(e, "designation") == "NPB300x150x42.24")
        .unwrap();
    assert_eq!(text(npb300, "library_id"), "india-is808-npb-profiles");
    assert_eq!(num(npb300, &["parameters_mm", "h"]), 300.0);
    assert_eq!(num(npb300, &["parameters_mm", "tw"]), 7.1);
    let wpb250 = entries
        .iter()
        .find(|e| text(e, "designation") == "WPB250x250x67.22")
        .unwrap();
    assert_eq!(
        text(wpb250, "library_id"),
        "india-is808-wpb-100-260-profiles"
    );
    assert_eq!(num(wpb250, &["parameters_mm", "h"]), 247.0);
    assert_eq!(num(wpb250, &["parameters_mm", "tw"]), 11.0);
    let wpb900 = entries
        .iter()
        .find(|e| text(e, "designation") == "WPB900x300x291.46")
        .unwrap();
    assert_eq!(
        text(wpb900, "library_id"),
        "india-is808-wpb-280-900-profiles"
    );
    assert_eq!(num(wpb900, &["parameters_mm", "h"]), 900.0);
    assert_eq!(num(wpb900, &["parameters_mm", "tw"]), 18.5);
}

#[test]
fn a_project_created_with_the_india_environment_resolves_its_designations() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "India Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "india",
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
        "NPB300x150x42.24",
        "--material",
        "Steel",
    ]);
    assert_eq!(text(&made, "outcome"), "member_created");
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 5380.0);
    // 5380 mm2 x 6000 mm x 7850 kg/m3.
    close(num(&q, &["mass", "kg"]), 253.398, 1e-9);

    // A second designation: NPB100x55x8.10, catalogue area 1030 mm2, as a 3 m column.
    let made = f.ok(&[
        "create-member",
        "--store",
        &store,
        "--kind",
        "column",
        "--start",
        "4000,0,0",
        "--end",
        "4000,0,3000",
        "--profile",
        "NPB100x55x8.10",
        "--material",
        "Steel",
    ]);
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 1030.0);
    close(num(&q, &["mass", "kg"]), 24.2565, 1e-9);

    // A designation the shipped series does not publish is refused, not silently substituted:
    // MB 300 belongs to the sloping-flange medium beam series of the same edition, which this
    // parallel-flange pack does not ship.
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
            "MB300",
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

#[test]
fn the_shipped_india_wpb_100_260_library_lists_and_resolves() {
    let f = Fixture::new();
    let library = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../environments/india/profiles-is808-wpb-100-260.usda"
    );
    let listed = f.ok(&["list-library", "--library", library, "--limit", "100"]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(num(&listed, &["entry_count"]), 52.0);
    let entries = at(&listed, &["entries"]).array().unwrap();
    assert_eq!(entries.len(), 52);
    let wpb250 = entries
        .iter()
        .find(|e| text(e, "designation") == "WPB250x250x67.22")
        .unwrap();
    assert_eq!(num(wpb250, &["area_mm2"]), 8560.0);
    assert_eq!(num(wpb250, &["mass_per_meter_kg"]), 67.22);

    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "India WPB Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "india",
    ]);
    let store = text(at(&created, &["project"]), "path");
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
        "WPB200x200x61.30",
        "--material",
        "Steel",
    ]);
    assert_eq!(text(&made, "outcome"), "member_created");
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 7800.0);
    // 7800 mm2 x 6000 mm x 7850 kg/m3.
    close(num(&q, &["mass", "kg"]), 367.38, 1e-9);
}

#[test]
fn the_shipped_india_wpb_280_900_library_lists_and_resolves() {
    let f = Fixture::new();
    let library = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../environments/india/profiles-is808-wpb-280-900.usda"
    );
    let listed = f.ok(&["list-library", "--library", library, "--limit", "100"]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(num(&listed, &["entry_count"]), 69.0);
    let entries = at(&listed, &["entries"]).array().unwrap();
    assert_eq!(entries.len(), 69);
    let wpb360 = entries
        .iter()
        .find(|e| text(e, "designation") == "WPB360x300x141.80")
        .unwrap();
    assert_eq!(num(wpb360, &["area_mm2"]), 18000.0);
    assert_eq!(num(wpb360, &["mass_per_meter_kg"]), 141.8);

    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "India Deep WPB Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "india",
    ]);
    let store = text(at(&created, &["project"]), "path");
    let made = f.ok(&[
        "create-member",
        "--store",
        &store,
        "--kind",
        "column",
        "--start",
        "0,0,0",
        "--end",
        "0,0,4000",
        "--profile",
        "WPB900x300x291.46",
        "--material",
        "Steel",
    ]);
    assert_eq!(text(&made, "outcome"), "member_created");
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 37100.0);
    // 37100 mm2 x 4000 mm x 7850 kg/m3.
    close(num(&q, &["mass", "kg"]), 1164.94, 1e-9);

    // The printed-erratum masses and the unresolved ghost row are not designations.
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
            "WPB360x300x163.00",
            "--material",
            "Steel"
        ]),
        "NOT_FOUND"
    );
}
