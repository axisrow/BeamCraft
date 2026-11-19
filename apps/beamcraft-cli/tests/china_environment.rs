// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped China environment holds the real GB/T 706-2016 nominal dimensions, re-entered by
// hand as our own data; provenance and licence are in `ATTRIBUTION.md` and in the library's own
// metadata. The expected areas and masses in this file are the standard's published values.

//! The regional `china` environment end to end through the real `beamcraft` binary: it is
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
    "/../../environments/china/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000b4";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("china-environment-tests"),
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
fn the_shipped_china_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["environment"]), "id"), "china");
    assert_eq!(num(&listed, &["entry_count"]), 189.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    let libraries = at(&listed, &["libraries"]).array().unwrap();
    assert_eq!(libraries.len(), 2);
    let info = at(&libraries[0], &["library"]);
    assert_eq!(text(info, "id"), "china-gb-t-706-profiles");
    assert_eq!(text(info, "version"), "2");
    assert_eq!(text(info, "licence"), "MPL-2.0");
    assert!(!text(info, "source").is_empty(), "the source is declared");
    assert_eq!(
        text(at(&libraries[1], &["library"]), "id"),
        "china-gb-t-11263-profiles"
    );
    let entries = at(&listed, &["entries"]).array().unwrap();
    assert_eq!(entries.len(), 189);
    let i10 = entries
        .iter()
        .find(|e| text(e, "designation") == "I10")
        .unwrap();
    assert_eq!(text(i10, "library_id"), "china-gb-t-706-profiles");
    assert_eq!(num(i10, &["parameters_mm", "h"]), 100.0);
    assert_eq!(num(i10, &["parameters_mm", "s"]), 4.5);
    assert_eq!(num(i10, &["parameters_mm", "r1"]), 3.3);
    // The dimensionless slope carries no unit suffix in the parameters list.
    let i55a = entries
        .iter()
        .find(|e| text(e, "designation") == "I55a")
        .unwrap();
    assert_eq!(num(i55a, &["parameters_mm", "h"]), 550.0);
    assert_eq!(num(i55a, &["parameters_mm", "k"]), 1.0 / 6.0);
    // The channel series carries its own prefix: the numeric designations overlap with the
    // I-beams (C10 is the channel, I10 the beam), and the slope is the standard's 1:10.
    let c5 = entries
        .iter()
        .find(|e| text(e, "designation") == "C5")
        .unwrap();
    assert_eq!(num(c5, &["parameters_mm", "h"]), 50.0);
    assert_eq!(num(c5, &["parameters_mm", "k"]), 1.0 / 10.0);
    // The H-section pack of GB/T 11263-2017: parallel flanges, so tw and tf instead of the
    // tapered family's s, t, r1 and slope.
    let hw100 = entries
        .iter()
        .find(|e| text(e, "designation") == "HW100x100")
        .unwrap();
    assert_eq!(text(hw100, "library_id"), "china-gb-t-11263-profiles");
    assert_eq!(num(hw100, &["parameters_mm", "h"]), 100.0);
    assert_eq!(num(hw100, &["parameters_mm", "tw"]), 6.0);
    assert_eq!(num(hw100, &["parameters_mm", "tf"]), 8.0);
    assert_eq!(num(hw100, &["area_mm2"]), 2158.0);
    let hn1008 = entries
        .iter()
        .find(|e| text(e, "designation") == "HN1008x302")
        .unwrap();
    assert_eq!(num(hn1008, &["parameters_mm", "h"]), 1008.0);
    assert_eq!(num(hn1008, &["parameters_mm", "b"]), 302.0);
    assert_eq!(num(hn1008, &["area_mm2"]), 43930.0);
    assert_eq!(num(hn1008, &["mass_per_meter_kg"]), 345.0);
}

#[test]
fn a_project_created_with_the_china_environment_resolves_its_designations() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "China Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "china",
    ]);
    let store = text(at(&created, &["project"]), "path");

    // The designation round rides one process: a designation of the pack resolves through
    // the project's own environment copy, with no --library option naming any file, and
    // every catalogue figure below is the standard's published value.
    // kind, start, end, profile, catalogue area mm2, mass kg (area x length x 7850 kg/m3);
    // the mass stays unchecked where the original round did not pin it.
    type Designation = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        f64,
        Option<f64>,
    );
    let cases: [Designation; 8] = [
        ("beam", "0,0,0", "6000,0,0", "I10", 1433.0, Some(67.4943)),
        // A 2016-edition-only designation.
        (
            "column",
            "4000,0,0",
            "4000,0,6000",
            "I55a",
            13410.0,
            Some(631.611),
        ),
        // The largest beam.
        (
            "beam",
            "8000,0,0",
            "14000,0,0",
            "I63c",
            17980.0,
            Some(846.858),
        ),
        // The channel series resolves beside the beams.
        (
            "beam",
            "16000,0,0",
            "22000,0,0",
            "C5",
            692.5,
            Some(32.61675),
        ),
        // The largest channel.
        (
            "column",
            "24000,0,0",
            "24000,0,6000",
            "C40c",
            9104.0,
            Some(428.7984),
        ),
        // The GB/T 11263-2017 H-section pack resolves through one series designation per
        // family, each as a 4 m column.
        (
            "column",
            "16000,0,4000",
            "16000,0,8000",
            "HW100x100",
            2158.0,
            None,
        ),
        (
            "column",
            "16000,0,4000",
            "16000,0,8000",
            "HM594x302",
            21710.0,
            None,
        ),
        (
            "column",
            "16000,0,4000",
            "16000,0,8000",
            "HN1000x300",
            39510.0,
            None,
        ),
    ];
    // A designation the shipped series does not publish is refused, not silently substituted:
    // the channel series has no 18c or 22c variant (the official table leaves them out; the
    // flawed web transcription printed blank cells there), the I-beam series has no I33, and
    // the thin-wall (HT) and pile (HP) series of GB/T 11263-2017 are not in the H-section pack.
    let absent = ["C18c", "C22c", "I33", "H200x100", "HT200x100", "HP250x250"];
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
        .map(|(kind, start, end, profile, _, _)| member(profile, kind, start, end))
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
    // Replies 0..8 carry the members, 8..14 the refusals (the refused lines commit nothing).
    for (index, (_, _, _, profile, area, mass)) in cases.iter().enumerate() {
        let made = &replies[index];
        assert_eq!(text(made, "outcome"), "member_created", "{profile}");
        let q = at(made, &["member", "quantities"]).clone();
        assert_eq!(text(&q, "area_source"), "catalogue", "{profile}");
        assert_eq!(num(&q, &["section_area_mm2"]), *area, "{profile}");
        if let Some(want) = mass {
            close(num(&q, &["mass", "kg"]), *want, 1e-9);
        }
    }
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[8 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }

    // The model is self-contained: a fresh process reopens the store and reports the same
    // catalogue figures without the environment being named.
    let inspected = f.ok(&["inspect-elements", "--store", &store]);
    let members = at(&inspected, &["members"]).array().unwrap();
    assert_eq!(members.len(), 8);
    for member in members {
        let stored = at(member, &["stored_quantities"]);
        assert_eq!(text(at(stored, &["volume"]), "freshness"), "current");
        assert_eq!(text(at(stored, &["mass"]), "freshness"), "current");
    }
}
