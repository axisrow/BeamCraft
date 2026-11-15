// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped Russia environment lists its GOST fastener set (`gost-fasteners.usda`): the
// dimensional facts of the public standards GOST 7798-70 (edition 2010, amendments 2-6,
// ST SEV 4728-84), GOST 5915-70 (edition 2010, amendments 2-7), GOST 11371-78 (class A) and
// GOST 11284-75 (first hole row), re-entered by hand as our own data; provenance and licence
// are in `ATTRIBUTION.md` and in the library's own metadata.

//! The fastener library of the `russia` environment end to end through the real `beamcraft`
//! binary: the environment lists the GOST set beside its profile library, every admitted entry
//! carries the declared catalogue values, and a project created with the environment resolves a
//! bolt designation through its own copy without any `--fastener-library` option, while a
//! designation the pack does not list stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/russia/environment.usda"
);
const LIBRARY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/russia/gost-fasteners.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000c1";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("russia-fasteners-tests"),
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
    fn plate(&self, store: &str, extra: &[&str]) -> String {
        let mut args: Vec<&str> = vec![
            "create-plate",
            "--store",
            store,
            "--width",
            "200",
            "--length",
            "200",
        ];
        args.extend_from_slice(extra);
        text(at(&self.ok(&args), &["plate"]), "element_id")
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
/// protrusion (the contributor default of the library) and the washer's outer, inner and
/// thickness dimensions.
fn assert_entry(entry: &Json, designation: &str, facts: [f64; 8], expected: &[f64]) {
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
    assert_eq!(text(entry, "standard"), "GOST 7798-70");
    assert_eq!(text(entry, "grade"), "5.8");
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
    // GOST 11284-75 first row: hole 13/17/21/25, one millimetre over the nominal diameter.
    assert_eq!(num(entry, &["hole_clearance_mm"]), 1.0);
    assert_eq!(lengths(entry), expected);
    assert!(at(entry, &["washer_under_head"]).boolean().unwrap());
    assert!(at(entry, &["washer_under_nut"]).boolean().unwrap());
    assert_eq!(text(entry, "hole_mode"), "generate");
}

#[test]
fn the_russia_environment_lists_and_shows_its_fastener_set() {
    let f = Fixture::new();

    // The environment listing names the fastener library beside the profile library.
    let listed = f.ok(&["list-library", "--environment", ENVIRONMENT]);
    assert_eq!(
        at(&listed, &["environment", "fastener_libraries"])
            .array()
            .unwrap(),
        [Json::text("gost-fasteners.usda")]
    );

    // The shipped library admits every row and refuses none; the pack lists the recommended
    // (unbracketed) length series of GOST 7798-70 up to 200 mm, per thread from its own minimum.
    let pack = f.ok(&["list-library", "--fastener-library", LIBRARY]);
    assert_eq!(num(&pack, &["entry_count"]), 4.0);
    assert_eq!(num(&pack, &["rejected_count"]), 0.0);
    let designations: Vec<String> = at(&pack, &["entries"])
        .array()
        .unwrap()
        .iter()
        .map(|e| text(e, "designation"))
        .collect();
    assert_eq!(designations, ["M12", "M16", "M20", "M24"]);

    // Every row field by field: the head and nut of GOST 7798-70/5915-70 (editions 2010), the
    // class-A washer of GOST 11371-78 and the first-row hole of GOST 11284-75.
    let common = [
        20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 65.0, 70.0, 75.0, 80.0, 90.0, 100.0,
        110.0, 120.0, 130.0, 140.0, 150.0, 160.0, 170.0, 180.0, 190.0, 200.0,
    ];
    let m12 = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M12",
    ]);
    assert_entry(
        at(&m12, &["entry"]),
        "M12",
        [12.0, 18.0, 7.5, 10.8, 4.0, 24.0, 13.0, 2.5],
        &common,
    );
    let m16 = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M16",
    ]);
    assert_entry(
        at(&m16, &["entry"]),
        "M16",
        [16.0, 24.0, 10.0, 14.8, 5.0, 30.0, 17.0, 3.0],
        &common,
    );
    let m20 = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M20",
    ]);
    assert_entry(
        at(&m20, &["entry"]),
        "M20",
        [20.0, 30.0, 12.5, 18.0, 6.0, 37.0, 21.0, 3.0],
        &common[1..],
    );
    let m24 = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M24",
    ]);
    assert_entry(
        at(&m24, &["entry"]),
        "M24",
        [24.0, 36.0, 15.0, 21.5, 7.0, 44.0, 25.0, 4.0],
        &common[3..],
    );

    // The library declares its provenance like every environment-listed library must.
    let shown = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "M12",
    ]);
    assert_eq!(
        text(at(&shown, &["library"]), "id"),
        "beamcraft-gost-fasteners"
    );
    assert_eq!(
        text(at(&shown, &["library"]), "source"),
        "Public standards GOST 7798-70 (ed. 2010, am. 2-6, ST SEV 4728-84), GOST 5915-70 (ed. 2010, am. 2-7, ST SEV 3683-82), GOST 11371-78 (class A) and GOST 11284-75 (first row), re-entered as our own data; thread protrusion is a contributor default"
    );
    assert_eq!(text(at(&shown, &["library"]), "licence"), "MPL-2.0");

    // A designation the pack does not list stays NOT_FOUND, never a near match: M30 is a real
    // size of GOST 7798-70 but the pack lists the 12-24 range only, and the lookup never
    // answers a lowercase variant of a listed designation.
    for absent in ["M30", "m16"] {
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
fn a_project_created_with_the_russia_environment_resolves_its_bolts() {
    let f = Fixture::new();

    // The project carries the fastener library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "Russia Bolts",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "russia",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("gost-fasteners.usda")
            .is_file(),
        "the project copy holds the GOST fastener library"
    );

    // A bolt designation of the set resolves through the project's own environment copy, with
    // no --fastener-library option naming any file (BC-ENV-002). M16 through 2 x 20 mm needs
    // grip 40 + washers 2 x 3 + head 10 + nut 14.8 + protrusion 5 = 75.8, so the declared
    // lengths list answers 80.
    let p1 = f.plate(&store, &["--thickness", "20", "--name", "P1"]);
    let p2 = f.plate(
        &store,
        &[
            "--thickness",
            "20",
            "--name",
            "P2",
            "--translation",
            "0,0,20",
        ],
    );
    let parts = format!("{p1},{p2}");
    let made = f.ok(&[
        "create-bolt-group",
        "--store",
        &store,
        "--name",
        "SE16",
        "--parts",
        &parts,
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
    ]);
    let groups = list(&made, "bolt_groups");
    assert_eq!(groups.len(), 1);
    assert_eq!(text(&groups[0], "length_mm"), "80");

    // The pinned definition is the catalogue entry itself: the GOST standard, the 5.8 grade of
    // the standard's own designation example and the first-row hole clearance (1 mm, not the
    // DIN set's 2) prove which entry the resolver answered.
    let definitions = f.ok(&["definitions", "--store", &store, "--name", "M16"]);
    let pinned = list(&definitions, "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "fastener");
    assert_eq!(text(&pinned[0], "designation"), "M16");
    assert_eq!(text(&pinned[0], "standard"), "GOST 7798-70");
    assert_eq!(text(&pinned[0], "grade"), "5.8");
    assert_eq!(num(&pinned[0], &["nominal_diameter_mm"]), 16.0);
    assert_eq!(num(&pinned[0], &["hole_clearance_mm"]), 1.0);
    let group = text(&made, "element_id");

    // A repin resolves through the environment the same way: M24 over the same grip needs
    // 40 + 2 x 4 + 15 + 21.5 + 7 = 91.5, so the declared list answers 100.
    let repinned = f.ok(&[
        "update-bolt-group",
        "--store",
        &store,
        "--target",
        &group,
        "--repin",
        "true",
        "--designation",
        "M24",
    ]);
    let groups = list(&repinned, "bolt_groups");
    assert_eq!(text(&groups[0], "name"), "SE16");
    assert_eq!(text(&groups[0], "length_mm"), "100");

    // A designation the pack does not list stays NOT_FOUND, never a near match.
    for absent in ["M30", "m16"] {
        assert_eq!(
            f.code(&[
                "create-bolt-group",
                "--store",
                &store,
                "--name",
                "BAD",
                "--parts",
                &parts,
                "--designation",
                absent,
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
            ]),
            "NOT_FOUND",
            "{absent}"
        );
    }
}
