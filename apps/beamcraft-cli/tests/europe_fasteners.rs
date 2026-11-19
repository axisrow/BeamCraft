// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Europe environment lists the DIN fastener set (`din-fasteners.usda`): the
// dimensional facts of the public standards DIN EN ISO 4014/4032/7089, re-entered by hand as
// our own data; provenance and licence are in `ATTRIBUTION.md` and in the library's own
// metadata. The expected lengths below follow from those declared values and the EN 1090-2
// hole series.

//! The fastener libraries of a regional environment end to end through the real `beamcraft`
//! binary: the environment lists its fastener library beside its profile library, a project
//! created with the environment carries the fastener library in its own copy, a bolt
//! designation resolves through it without any `--fastener-library` option (BC-ENV-002: the
//! same resolver as profiles), the pinned definition carries the declared catalogue values,
//! and a designation the set does not publish stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/europe/environment.usda"
);
const LIBRARY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/europe/din-fasteners.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a3";
// The pinned element ids of the bolt round, so the whole round rides one process (#738 P7).
const P1: &str = "0199e7a0-0000-7000-8000-00000000f0a1";
const P2: &str = "0199e7a0-0000-7000-8000-00000000f0a2";
const GROUP: &str = "0199e7a0-0000-7000-8000-00000000f0a3";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("europe-fasteners-tests"),
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

#[test]
fn the_europe_environment_lists_and_carries_its_fastener_library() {
    let f = Fixture::new();

    // The environment listing names the fastener library beside the profile library.
    let listed = f.ok(&["list-library", "--environment", ENVIRONMENT]);
    assert_eq!(
        at(&listed, &["environment", "fastener_libraries"])
            .array()
            .unwrap(),
        [Json::text("din-fasteners.usda")]
    );

    // The project carries the fastener library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Europe Bolts",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "europe",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("din-fasteners.usda")
            .is_file(),
        "the project copy holds the DIN fastener library"
    );

    // The bolt round rides one process (#738 P7): a bolt designation of the set resolves
    // through the project's own environment copy, with no --fastener-library option naming
    // any file. M16 through 2 x 20 mm needs grip 40 + washers 2 x 3 + head 10 + nut 14.8 +
    // protrusion 5 = 75.8, so the declared lengths list answers 80.
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
        // The pinned definition is the catalogue entry itself: the declared standard and
        // diameter ride into the model, which never needs the library file again.
        Scripted::ok(&["definitions", "--store", &store, "--name", "M16"]),
        // A repin resolves through the environment the same way: M12 over the same grip
        // answers 40 + 2 x 2.5 + 7.5 + 10.8 + 4 = 67.3, so the declared list gives 70 — a
        // different list than M16's, which proves which entry the resolver answered.
        Scripted::ok(&[
            "update-bolt-group",
            "--store",
            &store,
            "--target",
            GROUP,
            "--repin",
            "true",
            "--designation",
            "M12",
        ]),
    ];
    // A designation the pack does not list stays NOT_FOUND, never a near match: M36 is a real
    // DIN EN ISO 4014 size outside the pack's 12-30 range, and the lookup never answers a
    // lowercase variant of a listed designation.
    let absent = ["M36", "m16"];
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
    assert_eq!(text(&groups[0], "name"), "SE16");
    assert_eq!(text(&groups[0], "length_mm"), "80");
    let pinned = list(&replies[3], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "fastener");
    assert_eq!(text(&pinned[0], "designation"), "M16");
    assert_eq!(text(&pinned[0], "standard"), "DIN EN ISO 4014");
    assert_eq!(num(&pinned[0], &["nominal_diameter_mm"]), 16.0);
    assert_eq!(num(&pinned[0], &["hole_clearance_mm"]), 2.0);
    let repinned = &replies[4];
    let groups = list(repinned, "bolt_groups");
    assert_eq!(text(&groups[0], "name"), "SE16");
    assert_eq!(text(&groups[0], "length_mm"), "70");

    // Replies 5..7 carry the refusals in the authored order.
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[5 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
}

/// The catalogue facts of one of the new pack rows, the values the standards print, so no
/// declared field ships unchecked: nominal diameter, head across flats, head height, nut
/// height, thread protrusion (the contributor default of the library) and the washer's outer,
/// inner and thickness dimensions.
fn assert_series_row(entry: &Json, designation: &str, facts: [f64; 8], expected: &[f64]) {
    let [
        nominal,
        across_flats,
        head_height,
        nut_height,
        protrusion,
        washer_outer,
        washer_inner,
        washer_thickness,
    ] = facts;
    assert_eq!(text(entry, "designation"), designation);
    assert_eq!(text(entry, "standard"), "DIN EN ISO 4014");
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
    // EN 1090-2 table 11 normal round holes for the follow-up sizes only: two millimetres over
    // the bolt at M24 and three from M27, the 26/30/33 hole series these rows ship. The M12 row
    // is not part of this series: it keeps its A14.1 close-fit 13 mm hole (the EN 1090-2 normal
    // value for M12 is 14) and is asserted by the older test above.
    assert_eq!(lengths(entry), expected);
    assert!(at(entry, &["washer_under_head"]).boolean().unwrap());
    assert!(at(entry, &["washer_under_nut"]).boolean().unwrap());
    assert_eq!(text(entry, "hole_mode"), "generate");
}

#[test]
fn the_europe_environment_lists_the_en_1090_2_hole_series_rows() {
    let f = Fixture::new();

    // The pack now carries the six designations of the DIN set: the three original rows and
    // the EN 1090-2 hole-series follow-up rows M24, M27 and M30.
    let pack = f.ok(&["list-library", "--fastener-library", LIBRARY]);
    assert_eq!(num(&pack, &["entry_count"]), 6.0);
    assert_eq!(num(&pack, &["rejected_count"]), 0.0);
    let designations: Vec<String> = at(&pack, &["entries"])
        .array()
        .unwrap()
        .iter()
        .map(|e| text(e, "designation"))
        .collect();
    assert_eq!(designations, ["M12", "M16", "M20", "M24", "M27", "M30"]);

    // Every follow-up row field by field: the DIN EN ISO 4014 head and the DIN EN ISO 4032
    // nut, the DIN EN ISO 7089 washer and the EN 1090-2 hole series (26/30/33, the +2 up to
    // M24 and +3 from M27 normal round holes).
    let m24_shown = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M24",
    ]);
    assert_series_row(
        at(&m24_shown, &["entry"]),
        "M24",
        [24.0, 36.0, 15.0, 21.5, 7.0, 44.0, 25.0, 4.0],
        &[
            55.0, 60.0, 65.0, 70.0, 75.0, 80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 140.0, 150.0,
        ],
    );
    let m27_shown = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M27",
    ]);
    assert_series_row(
        at(&m27_shown, &["entry"]),
        "M27",
        [27.0, 41.0, 17.0, 23.6, 8.0, 50.0, 28.0, 4.0],
        &[
            65.0, 70.0, 75.0, 80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 140.0, 150.0,
        ],
    );
    let m30_shown = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M30",
    ]);
    assert_series_row(
        at(&m30_shown, &["entry"]),
        "M30",
        [30.0, 46.0, 18.7, 25.6, 9.0, 56.0, 31.0, 4.0],
        &[75.0, 80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 140.0, 150.0],
    );

    // The EN 1090-2 hole series is the point of the follow-up: assert the clearances
    // directly, two millimetres on M24 and three on M27 and M30.
    assert_eq!(num(at(&m24_shown, &["entry"]), &["hole_clearance_mm"]), 2.0);
    assert_eq!(num(at(&m27_shown, &["entry"]), &["hole_clearance_mm"]), 3.0);
    assert_eq!(num(at(&m30_shown, &["entry"]), &["hole_clearance_mm"]), 3.0);

    // A designation the pack does not list stays NOT_FOUND: M36 is a real DIN EN ISO 4014
    // size outside the pack's 12-30 range.
    assert_eq!(
        f.code(&[
            "show-library",
            "--fastener-library",
            LIBRARY,
            "--designation",
            "M36"
        ]),
        "NOT_FOUND"
    );
}

fn lengths(json: &Json) -> Vec<f64> {
    at(json, &["lengths_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
