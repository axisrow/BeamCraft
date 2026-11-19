// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped India environment lists its IS structural fastener set (`is-fasteners.usda`): the
// dimensional facts of the public standards IS 3757 (high strength structural bolts, large
// series hexagon, 1985 second revision), IS 6623 (high strength structural nuts, 2004 second
// revision) and IS 6649 (hardened and tempered washers, 1985 first revision), read from the
// official BIS texts and re-entered by hand as our own data; provenance and licence are in
// `ATTRIBUTION.md` and in the library's own metadata.

//! The fastener library of the `india` environment end to end through the real `beamcraft`
//! binary: the environment lists the IS set beside its profile libraries, every admitted entry
//! carries the declared catalogue values, and a project created with the environment resolves a
//! bolt designation through its own copy without any `--fastener-library` option, while a
//! designation the pack does not list stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/india/environment.usda"
);
const LIBRARY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/india/is-fasteners.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000e1";
// The pinned element ids of the bolt round, so the whole round rides one process (#738 P7).
const P1: &str = "0199e7a0-0000-7000-8000-00000000f0e1";
const P2: &str = "0199e7a0-0000-7000-8000-00000000f0e2";
const GROUP: &str = "0199e7a0-0000-7000-8000-00000000f0e3";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("india-fasteners-tests"),
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
    /// One `run-script` invocation with the same clean environment `raw` gives its spawns:
    /// the store commands amortize the environment copy's library parse into one process
    /// (#738 P7), the commands' own work unchanged.
    fn script(&self, store: &str, lines: &[Scripted]) -> Vec<Json> {
        support::script_lines_without_config(&self.shared.root, store, lines)
    }
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

fn lengths(json: &Json) -> Vec<f64> {
    at(json, &["lengths_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}

/// The catalogue facts of one pack row, the values the standards print, so no declared field
/// ships unchecked: nominal diameter, head across flats, head height, nut height, thread
/// protrusion (the contributor default of the library), the washer's outer, hole and thickness
/// dimensions, and the assembly-hole clearance the IS 6649 washer hole fixes.
fn assert_entry(entry: &Json, designation: &str, facts: [f64; 9], expected: &[f64]) {
    let [
        nominal,
        across_flats,
        head_height,
        nut_height,
        protrusion,
        washer_outer,
        washer_inner,
        washer_thickness,
        clearance,
    ] = facts;
    assert_eq!(text(entry, "designation"), designation);
    assert_eq!(text(entry, "standard"), "IS 3757");
    assert_eq!(text(entry, "grade"), "8.8");
    assert_eq!(text(entry, "fastener_kind"), "bolt");
    assert_eq!(text(entry, "head_kind"), "hex");
    assert_eq!(num(entry, &["nominal_diameter_mm"]), nominal);
    assert_eq!(num(entry, &["head_across_flats_mm"]), across_flats);
    assert_eq!(num(entry, &["nut_across_flats_mm"]), across_flats);
    assert_eq!(num(entry, &["head_height_mm"]), head_height);
    assert_eq!(num(entry, &["nut_height_mm"]), nut_height);
    assert_eq!(num(entry, &["thread_protrusion_mm"]), protrusion);
    assert_eq!(num(entry, &["washer_outer_diameter_mm"]), washer_outer);
    assert_eq!(num(entry, &["washer_inner_diameter_mm"]), washer_inner);
    assert_eq!(num(entry, &["washer_thickness_mm"]), washer_thickness);
    // The assembly hole is the IS 6649 washer hole minimum (18/22/26), which the IS 3757
    // anti-seizing test fixes as the assembly hole diameter: clearance two millimetres.
    assert_eq!(num(entry, &["hole_clearance_mm"]), clearance);
    assert_eq!(lengths(entry), expected);
    assert!(at(entry, &["washer_under_head"]).boolean().unwrap());
    assert!(at(entry, &["washer_under_nut"]).boolean().unwrap());
    assert_eq!(text(entry, "hole_mode"), "generate");
}

#[test]
fn the_india_environment_lists_and_shows_its_fastener_set() {
    let f = Fixture::new();

    // The environment listing names the fastener library beside the profile libraries.
    let listed = f.ok(&["list-library", "--environment", ENVIRONMENT]);
    assert_eq!(
        at(&listed, &["environment", "fastener_libraries"])
            .array()
            .unwrap(),
        [Json::text("is-fasteners.usda")]
    );

    // The shipped library admits every row and refuses none; the pack lists the IS 3757
    // table 2 recommended combinations from each thread's minimum (40/45/50), cut at 160 mm.
    // The IS 3757 range starts at M16, so there is no M12 row.
    let pack = f.ok(&["list-library", "--fastener-library", LIBRARY]);
    assert_eq!(num(&pack, &["entry_count"]), 3.0);
    assert_eq!(num(&pack, &["rejected_count"]), 0.0);
    let designations: Vec<String> = at(&pack, &["entries"])
        .array()
        .unwrap()
        .iter()
        .map(|e| text(e, "designation"))
        .collect();
    assert_eq!(designations, ["M16", "M20", "M24"]);

    // Every row field by field: the large-series head of IS 3757 (across flats 27/34/41), the
    // heavy structural nut of IS 6623 and the hardened washer of IS 6649.
    let m16 = [
        40.0, 45.0, 50.0, 55.0, 60.0, 65.0, 70.0, 75.0, 80.0, 85.0, 90.0, 95.0, 100.0, 110.0,
        120.0, 130.0, 140.0, 150.0, 160.0,
    ];
    let m20 = [
        45.0, 50.0, 55.0, 60.0, 65.0, 70.0, 75.0, 80.0, 85.0, 90.0, 95.0, 100.0, 110.0, 120.0,
        130.0, 140.0, 150.0, 160.0,
    ];
    let m24 = [
        50.0, 55.0, 60.0, 65.0, 70.0, 75.0, 80.0, 85.0, 90.0, 95.0, 100.0, 110.0, 120.0, 130.0,
        140.0, 150.0, 160.0,
    ];
    let m16_shown = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M16",
    ]);
    assert_entry(
        at(&m16_shown, &["entry"]),
        "M16",
        [16.0, 27.0, 10.0, 17.1, 5.0, 34.0, 18.0, 4.8, 2.0],
        &m16,
    );
    let m20_shown = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M20",
    ]);
    assert_entry(
        at(&m20_shown, &["entry"]),
        "M20",
        [20.0, 34.0, 12.5, 20.7, 6.0, 42.0, 22.0, 4.8, 2.0],
        &m20,
    );
    let m24_shown = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M24",
    ]);
    assert_entry(
        at(&m24_shown, &["entry"]),
        "M24",
        [24.0, 41.0, 15.0, 24.2, 7.0, 50.0, 26.0, 4.6, 2.0],
        &m24,
    );

    // The library declares its provenance like every environment-listed library must.
    let shown = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M16",
    ]);
    assert_eq!(
        text(at(&shown, &["library"]), "id"),
        "beamcraft-is-fasteners"
    );
    assert_eq!(
        text(at(&shown, &["library"]), "source"),
        "Public standards IS 3757 (1985), IS 6623 (2004) and IS 6649 (1985), read from the official BIS texts and re-entered as our own data; the hole follows the IS 6649 washer hole; thread protrusion is a contributor default"
    );
    assert_eq!(text(at(&shown, &["library"]), "licence"), "MPL-2.0");

    // A designation the pack does not list stays NOT_FOUND, never a near match: M12 is below
    // the IS 3757 range (the standard covers M16-M36 only), and the lookup never answers a
    // lowercase variant of a listed designation.
    for absent in ["M12", "m16"] {
        assert_eq!(
            f.code(&[
                "show-library",
                "--fastener-library",
                LIBRARY,
                "--designation",
                absent
            ]),
            "NOT_FOUND",
            "{absent}"
        );
    }
}

#[test]
fn a_project_created_with_the_india_environment_resolves_its_bolts() {
    let f = Fixture::new();

    // The project carries the fastener library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "India Bolts",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "india",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("is-fasteners.usda")
            .is_file(),
        "the project copy holds the IS fastener library"
    );

    // The bolt round rides one process (#738 P7): a bolt designation of the set resolves
    // through the project's own environment copy, with no --fastener-library option naming
    // any file (BC-ENV-002). M16 through 2 x 20 mm needs grip 40 + washers 2 x 4.8 + head 10
    // + nut 17.1 + protrusion 5 = 81.7, so the declared lengths list answers 85.
    let parts = format!("{P1},{P2}");
    let mut lines = vec![
        Scripted::ok(&[
            "create-plate",
            "--store",
            &store,
            "--width",
            "200",
            "--length",
            "200",
            "--thickness",
            "20",
            "--name",
            "P1",
            "--new-id",
            P1,
        ]),
        Scripted::ok(&[
            "create-plate",
            "--store",
            &store,
            "--width",
            "200",
            "--length",
            "200",
            "--thickness",
            "20",
            "--name",
            "P2",
            "--translation",
            "0,0,20",
            "--new-id",
            P2,
        ]),
        Scripted::ok(&[
            "create-bolt-group",
            "--store",
            &store,
            "--name",
            "SE16",
            "--parts",
            parts.as_str(),
            "--designation",
            "M16",
            "--point1",
            "50,50,40",
            "--point2",
            "50,150,40",
            "--plane-normal",
            "0,0,-1",
            "--offset-start",
            "25",
            "--count",
            "2",
            "--pitch",
            "50",
            "--hole-mode",
            "none",
            "--new-id",
            GROUP,
        ]),
        // The pinned definition is the catalogue entry itself: the IS standard, the 8.8
        // class and the large-series across flats 27 (the DIN set's M16 rides a 24)
        // identify the row.
        Scripted::ok(&["definitions", "--store", &store, "--name", "M16"]),
        // A repin resolves through the environment the same way: M24 over the same grip
        // needs 40 + 2 x 4.6 + 15 + 24.2 + 7 = 95.4, so the declared list answers 100.
        Scripted::ok(&[
            "update-bolt-group",
            "--store",
            &store,
            "--target",
            GROUP,
            "--repin",
            "true",
            "--designation",
            "M24",
        ]),
    ];
    // A designation the pack does not list stays NOT_FOUND, never a near match.
    let absent = ["M12", "m16"];
    for designation in absent {
        lines.push(Scripted::refused(&[
            "create-bolt-group",
            "--store",
            &store,
            "--name",
            "BAD",
            "--parts",
            parts.as_str(),
            "--designation",
            designation,
            "--point1",
            "50,50,40",
            "--point2",
            "50,150,40",
            "--plane-normal",
            "0,0,-1",
            "--offset-start",
            "25",
            "--count",
            "2",
            "--pitch",
            "50",
            "--hole-mode",
            "none",
        ]));
    }
    let replies = f.script(&store, &lines);
    let made = &replies[2];
    let groups = list(made, "bolt_groups");
    assert_eq!(groups.len(), 1);
    assert_eq!(text(&groups[0], "length_mm"), "85");
    let pinned = list(&replies[3], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "fastener");
    assert_eq!(text(&pinned[0], "designation"), "M16");
    assert_eq!(text(&pinned[0], "standard"), "IS 3757");
    assert_eq!(text(&pinned[0], "grade"), "8.8");
    assert_eq!(num(&pinned[0], &["nominal_diameter_mm"]), 16.0);
    assert_eq!(num(&pinned[0], &["hole_clearance_mm"]), 2.0);
    let repinned = &replies[4];
    let groups = list(repinned, "bolt_groups");
    assert_eq!(text(&groups[0], "name"), "SE16");
    assert_eq!(text(&groups[0], "length_mm"), "100");
    // Replies 5..7 carry the refusals in the authored order.
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[5 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
}
