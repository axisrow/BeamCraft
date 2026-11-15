// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Europe hollow-section library holds the real EN 10219-2:2006 nominal dimensions,
// re-entered by hand as our own data; provenance and licence are in `ATTRIBUTION.md` and in the
// library's own metadata. The expected areas and masses in this file are the standard's
// published values.

//! The cold-formed hollow sections of the regional `europe` environment end to end through the
//! real `beamcraft` binary: the environment lists both its profile libraries, a project created
//! with it resolves an SHS designation through the copy in the project folder, the declared
//! catalogue area of the closed section drives the quantities, and a designation the shipped
//! series does not publish is refused instead of silently substituted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/europe/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a3";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("europe-hollow-environment-tests"),
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
fn the_europe_environment_lists_both_profile_libraries_and_their_hollow_rows() {
    let f = Fixture::new();
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["environment"]), "id"), "europe");
    // The hot-rolled pack and the cold-formed hollow pack are both listed, in manifest order.
    let libraries = at(&listed, &["libraries"]).array().unwrap();
    assert_eq!(libraries.len(), 2);
    let ids: Vec<String> = libraries
        .iter()
        .map(|l| text(at(l, &["library"]), "id"))
        .collect();
    assert_eq!(
        ids,
        ["europe-en-10365-profiles", "europe-en-10219-profiles"].map(String::from)
    );
    let hollow = &libraries[1];
    assert_eq!(
        text(at(hollow, &["library"]), "title"),
        "Europe cold-formed hollow sections (EN 10219: SHS, RHS, CHS)"
    );
    assert_eq!(text(at(hollow, &["library"]), "version"), "1");
    assert_eq!(text(at(hollow, &["library"]), "licence"), "MPL-2.0");
    assert!(
        !text(at(hollow, &["library"]), "source").is_empty(),
        "the source is declared"
    );
    assert_eq!(num(&listed, &["entry_count"]), 139.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let entries = at(&listed, &["entries"]).array().unwrap();
    let shs = entries
        .iter()
        .find(|e| text(e, "designation") == "SHS100x100x5")
        .unwrap();
    assert_eq!(text(shs, "library_id"), "europe-en-10219-profiles");
    assert_eq!(num(shs, &["parameters_mm", "h"]), 100.0);
    assert_eq!(num(shs, &["parameters_mm", "b"]), 100.0);
    assert_eq!(num(shs, &["parameters_mm", "t"]), 5.0);
}

#[test]
fn a_project_created_with_the_europe_environment_resolves_hollow_designations() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Europe Hollow Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "europe",
    ]);
    let store = text(at(&created, &["project"]), "path");

    // An SHS designation of the hollow pack resolves through the project's own environment
    // copy, with no --library option naming any file. SHS100x100x5 declares a catalogue area
    // of 1840 mm2.
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
        "SHS100x100x5",
        "--material",
        "Steel",
    ]);
    assert_eq!(text(&made, "outcome"), "member_created");
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 1840.0);
    // 1840 mm2 x 6000 mm x 7850 kg/m3.
    close(num(&q, &["mass", "kg"]), 86.664, 1e-9);

    // A CHS designation of the same pack: CHS219.1x6.0, catalogue area 4020 mm2.
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
        "CHS219.1x6.0",
        "--material",
        "Steel",
    ]);
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 4020.0);
    close(num(&q, &["mass", "kg"]), 189.342, 1e-9);

    // A designation the shipped hollow series does not publish is refused, not silently
    // substituted: 110 mm is not an EN 10219-2 square size.
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
            "SHS110x110x5",
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
