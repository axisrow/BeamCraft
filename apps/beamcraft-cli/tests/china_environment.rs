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

use support::{at, text};

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
        "I10",
        "--material",
        "Steel",
    ]);
    assert_eq!(text(&made, "outcome"), "member_created");
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 1433.0);
    // 1433 mm2 x 6000 mm x 7850 kg/m3.
    close(num(&q, &["mass", "kg"]), 67.4943, 1e-9);

    // A 2016-edition-only designation: I55a, catalogue area 13410 mm2.
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
        "I55a",
        "--material",
        "Steel",
    ]);
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 13410.0);
    close(num(&q, &["mass", "kg"]), 631.611, 1e-9);

    // The largest beam: I63c, catalogue area 17980 mm2.
    let made = f.ok(&[
        "create-member",
        "--store",
        &store,
        "--kind",
        "beam",
        "--start",
        "8000,0,0",
        "--end",
        "14000,0,0",
        "--profile",
        "I63c",
        "--material",
        "Steel",
    ]);
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 17980.0);
    close(num(&q, &["mass", "kg"]), 846.858, 1e-9);

    // The channel series resolves beside the beams: C5, catalogue area 692.5 mm2.
    let made = f.ok(&[
        "create-member",
        "--store",
        &store,
        "--kind",
        "beam",
        "--start",
        "16000,0,0",
        "--end",
        "22000,0,0",
        "--profile",
        "C5",
        "--material",
        "Steel",
    ]);
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 692.5);
    close(num(&q, &["mass", "kg"]), 32.61675, 1e-9);

    // The largest channel: C40c, catalogue area 9104 mm2.
    let made = f.ok(&[
        "create-member",
        "--store",
        &store,
        "--kind",
        "column",
        "--start",
        "24000,0,0",
        "--end",
        "24000,0,6000",
        "--profile",
        "C40c",
        "--material",
        "Steel",
    ]);
    let q = at(&made, &["member", "quantities"]).clone();
    assert_eq!(text(&q, "area_source"), "catalogue");
    assert_eq!(num(&q, &["section_area_mm2"]), 9104.0);
    close(num(&q, &["mass", "kg"]), 428.7984, 1e-9);

    // The GB/T 11263-2017 H-section pack of the same environment resolves through one series
    // designation per family: HW100x100 (2158 mm2), HM594x302 (21710 mm2) and the deepest
    // narrow beam HN1000x300 (39510 mm2), each as a 4 m column.
    for (profile, area) in [
        ("HW100x100", 2158.0),
        ("HM594x302", 21710.0),
        ("HN1000x300", 39510.0),
    ] {
        let made = f.ok(&[
            "create-member",
            "--store",
            &store,
            "--kind",
            "column",
            "--start",
            "16000,0,4000",
            "--end",
            "16000,0,8000",
            "--profile",
            profile,
            "--material",
            "Steel",
        ]);
        let q = at(&made, &["member", "quantities"]).clone();
        assert_eq!(text(&q, "area_source"), "catalogue", "{profile}");
        assert_eq!(num(&q, &["section_area_mm2"]), area, "{profile}");
    }

    // A designation the shipped series does not publish is refused, not silently substituted:
    // the channel series has no 18c or 22c variant (the official table leaves them out; the
    // flawed web transcription printed blank cells there), the I-beam series has no I33, and
    // the thin-wall (HT) and pile (HP) series of GB/T 11263-2017 are not in the H-section pack.
    for absent in ["C18c", "C22c", "I33", "H200x100", "HT200x100", "HP250x250"] {
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
                absent,
                "--material",
                "Steel"
            ]),
            "NOT_FOUND",
            "{absent}"
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
