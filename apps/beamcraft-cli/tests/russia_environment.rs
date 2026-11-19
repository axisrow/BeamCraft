// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Russia environment holds the real GOST 26020-83 nominal dimensions, re-entered by
// hand as our own data; provenance and licence are in `ATTRIBUTION.md` and in the library's own
// metadata. The expected areas and masses in this file are the standard's published values.

//! The regional `russia` environment end to end through the real `beamcraft` binary: it is
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
    "/../../environments/russia/environment.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a3";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("russia-environment-tests"),
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
    /// (#738 P7; the measured per-spawn cost is ~0.8 s on a store carrying the Russia
    /// libraries, the commands' own work unchanged).
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
fn the_shipped_russia_environment_lists_its_sections_and_provenance() {
    let f = Fixture::new();
    let listed = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--limit",
        "256",
    ]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["environment"]), "id"), "russia");
    assert_eq!(num(&listed, &["entry_count"]), 696.0);
    assert_eq!(at(&listed, &["shadowed"]).array().unwrap().len(), 0);
    // The environment now lists 571 entries, over the 256-entry page cap, so the later
    // pages complete the listing before the per-entry assertions.
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
    assert_eq!(num(&rest, &["entry_count"]), 696.0);
    entries.extend(at(&rest, &["entries"]).array().unwrap().iter().cloned());
    // The band pushes the listing past two pages: the third page carries the tail.
    let tail = f.ok(&[
        "list-library",
        "--environment",
        ENVIRONMENT,
        "--offset",
        "512",
        "--limit",
        "256",
    ]);
    assert_eq!(num(&tail, &["entry_count"]), 696.0);
    entries.extend(at(&tail, &["entries"]).array().unwrap().iter().cloned());
    assert_eq!(entries.len(), 696);
    let libraries = at(&listed, &["libraries"]).array().unwrap();
    assert_eq!(libraries.len(), 3);
    let info = at(&libraries[0], &["library"]);
    assert_eq!(text(info, "id"), "russia-gost-26020-profiles");
    assert_eq!(text(info, "version"), "2");
    assert_eq!(text(info, "licence"), "MPL-2.0");
    assert!(!text(info, "source").is_empty(), "the source is declared");
    let square_info = at(&libraries[1], &["library"]);
    assert_eq!(text(square_info, "id"), "russia-gost-30245-profiles");
    assert_eq!(text(square_info, "version"), "7");
    assert_eq!(text(square_info, "licence"), "MPL-2.0");
    assert!(
        !text(square_info, "source").is_empty(),
        "the source is declared"
    );
    let inclined_info = at(&libraries[2], &["library"]);
    assert_eq!(text(inclined_info, "id"), "russia-gost-8239-8240-profiles");
    assert_eq!(text(inclined_info, "version"), "1");
    assert_eq!(text(inclined_info, "licence"), "MPL-2.0");
    assert!(
        !text(inclined_info, "source").is_empty(),
        "the source is declared"
    );
    let b30_1 = entries
        .iter()
        .find(|e| text(e, "designation") == "30B1")
        .unwrap();
    assert_eq!(text(b30_1, "library_id"), "russia-gost-26020-profiles");
    assert_eq!(num(b30_1, &["parameters_mm", "h"]), 296.0);
    assert_eq!(num(b30_1, &["parameters_mm", "tw"]), 5.8);
    let sh30_1 = entries
        .iter()
        .find(|e| text(e, "designation") == "30Sh1")
        .unwrap();
    assert_eq!(num(sh30_1, &["parameters_mm", "h"]), 291.0);
    assert_eq!(num(sh30_1, &["parameters_mm", "b"]), 200.0);
    // The cold-bent square series rides the same environment as its second library, with the
    // Latinized clause 3.9 designations and the corner rule carried in the radius.
    let square = entries
        .iter()
        .find(|e| text(e, "designation") == "60x60x3")
        .unwrap();
    assert_eq!(text(square, "library_id"), "russia-gost-30245-profiles");
    assert_eq!(num(square, &["parameters_mm", "h"]), 60.0);
    assert_eq!(num(square, &["parameters_mm", "t"]), 3.0);
    assert_eq!(num(square, &["parameters_mm", "r"]), 6.0);
    // The inclined-flange pack rides the tapered families as its third library: the mid-leg
    // flange thickness and the roll-pass-range middle slope are carried per row.
    let inclined = entries
        .iter()
        .find(|e| text(e, "designation") == "I30")
        .unwrap();
    assert_eq!(
        text(inclined, "library_id"),
        "russia-gost-8239-8240-profiles"
    );
    assert_eq!(num(inclined, &["parameters_mm", "h"]), 300.0);
    assert_eq!(num(inclined, &["parameters_mm", "s"]), 6.5);
    assert_eq!(num(inclined, &["parameters_mm", "t"]), 10.2);
    assert_eq!(num(inclined, &["parameters_mm", "r"]), 12.0);
    assert_eq!(num(inclined, &["parameters_mm", "r1"]), 5.0);
    assert_eq!(num(inclined, &["parameters_mm", "k"]), 0.09);
    let channel = entries
        .iter()
        .find(|e| text(e, "designation") == "C16a")
        .unwrap();
    assert_eq!(num(channel, &["parameters_mm", "b"]), 68.0);
    assert_eq!(num(channel, &["parameters_mm", "k"]), 0.07);
    // The rectangular band rides the second library: h carries the height, b the width.
    let rectangle = entries
        .iter()
        .find(|e| text(e, "designation") == "80x60x3")
        .unwrap();
    assert_eq!(text(rectangle, "library_id"), "russia-gost-30245-profiles");
    assert_eq!(num(rectangle, &["parameters_mm", "h"]), 80.0);
    assert_eq!(num(rectangle, &["parameters_mm", "b"]), 60.0);
    assert_eq!(num(rectangle, &["parameters_mm", "t"]), 3.0);
    assert_eq!(num(rectangle, &["parameters_mm", "r"]), 6.0);
    // Band 2 rides the same library: the 90..120 mm height groups.
    let band2 = entries
        .iter()
        .find(|e| text(e, "designation") == "120x80x3")
        .unwrap();
    assert_eq!(text(band2, "library_id"), "russia-gost-30245-profiles");
    assert_eq!(num(band2, &["parameters_mm", "h"]), 120.0);
    assert_eq!(num(band2, &["parameters_mm", "b"]), 80.0);
    assert_eq!(num(band2, &["parameters_mm", "t"]), 3.0);
    assert_eq!(num(band2, &["parameters_mm", "r"]), 6.0);
    // Band 3 rides the same library: the 140..160 mm height groups.
    let band3 = entries
        .iter()
        .find(|e| text(e, "designation") == "160x100x4")
        .unwrap();
    assert_eq!(text(band3, "library_id"), "russia-gost-30245-profiles");
    assert_eq!(num(band3, &["parameters_mm", "h"]), 160.0);
    assert_eq!(num(band3, &["parameters_mm", "b"]), 100.0);
    assert_eq!(num(band3, &["parameters_mm", "t"]), 4.0);
    assert_eq!(num(band3, &["parameters_mm", "r"]), 8.0);
    // Band 4 rides the same library: the 180..200 mm height groups.
    let band4 = entries
        .iter()
        .find(|e| text(e, "designation") == "200x100x4")
        .unwrap();
    assert_eq!(text(band4, "library_id"), "russia-gost-30245-profiles");
    assert_eq!(num(band4, &["parameters_mm", "h"]), 200.0);
    assert_eq!(num(band4, &["parameters_mm", "b"]), 100.0);
    assert_eq!(num(band4, &["parameters_mm", "t"]), 4.0);
    assert_eq!(num(band4, &["parameters_mm", "r"]), 8.0);
    // The final band rides the same library; the assortment's largest row closes it.
    let band5 = entries
        .iter()
        .find(|e| text(e, "designation") == "400x200x10")
        .unwrap();
    assert_eq!(text(band5, "library_id"), "russia-gost-30245-profiles");
    assert_eq!(num(band5, &["parameters_mm", "h"]), 400.0);
    assert_eq!(num(band5, &["parameters_mm", "b"]), 200.0);
    assert_eq!(num(band5, &["parameters_mm", "t"]), 10.0);
    assert_eq!(num(band5, &["parameters_mm", "r"]), 25.0);
}

#[test]
fn a_project_created_with_the_russia_environment_resolves_its_designations() {
    let f = Fixture::new();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Russia Model",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "russia",
    ]);
    let store = text(at(&created, &["project"]), "path");

    // The designation round rides one process: a designation of the pack resolves through
    // the project's own environment copy, with no --library option naming any file, and
    // every catalogue figure below is the standard's published value. The refusals sit in
    // the middle in their authored order: 30K4 and 80Sh1 exist only in the newer
    // GOST R 57837-2017, GOST 8239-89 publishes no 25 and GOST 8240-89 no 24a (the
    // a-variants are 16a and 18a only), GOST 30245-2003 publishes no 220 mm square at all,
    // and its 50x30 rectangular group steps over the 4.5 mm wall.
    let cases: [(&str, &str, &str, &str, f64, f64); 8] = [
        // kind, start, end, profile, catalogue area mm2, mass kg (area x length x 7850 kg/m3).
        ("beam", "0,0,0", "6000,0,0", "30B1", 4192.0, 197.4432),
        (
            "column",
            "4000,0,0",
            "4000,0,6000",
            "40K2",
            21096.0,
            993.6216,
        ),
        ("beam", "8000,0,0", "14000,0,0", "50Sh2", 17660.0, 831.786),
        // The cold-bent square designation of the second library: corner radius 2t = 6 mm.
        (
            "column",
            "16000,0,0",
            "16000,0,3000",
            "60x60x3",
            661.0,
            15.566550,
        ),
        // The largest square of the series (printed 102.0 cm2 at one decimal).
        (
            "column",
            "20000,0,0",
            "20000,0,3000",
            "300x300x9",
            10200.0,
            240.21,
        ),
        // The inclined-flange I beam of the third library, riding the tapered_i family.
        ("beam", "30000,0,0", "36000,0,0", "I30", 4650.0, 219.015),
        // An inclined-flange channel of the third library.
        ("beam", "36000,0,0", "36000,0,3000", "C16a", 1950.0, 45.9225),
        // The cold-bent rectangular designation: the height along h, the width along b.
        ("beam", "22000,0,0", "28000,0,0", "80x60x3", 781.0, 36.7851),
    ];
    let absent = ["30K4", "80Sh1", "I25", "C24a", "220x220x10", "50x30x4.5"];
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
    let mut lines: Vec<Scripted> = cases[..7]
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
    let (kind, start, end, profile, _, _) = cases[7];
    lines.push(member(profile, kind, start, end));
    let replies = f.script(&store, &lines);
    // Replies 0..7 carry the first seven members, 7..13 the refusals, 13 the last member
    // (the refused lines commit nothing, so the order above is the authored order).
    for (index, (_, _, _, profile, area, mass)) in cases.iter().enumerate() {
        let made = if index < 7 {
            &replies[index]
        } else {
            &replies[13]
        };
        assert_eq!(text(made, "outcome"), "member_created", "{profile}");
        let q = at(made, &["member", "quantities"]).clone();
        assert_eq!(text(&q, "area_source"), "catalogue", "{profile}");
        assert_eq!(num(&q, &["section_area_mm2"]), *area, "{profile}");
        close(num(&q, &["mass", "kg"]), *mass, 1e-9);
    }
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[7 + index], "code"),
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
