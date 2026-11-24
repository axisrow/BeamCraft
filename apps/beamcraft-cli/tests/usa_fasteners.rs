// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// The shipped USA environment lists its RCSC 2025 fastener set (`fasteners-rcsc-2025.usda`):
// the dimensional facts of the public specification RCSC 2025 (June 5, 2025, boltcouncil.org)
// — F3125 Grade A325 and A490 heavy hex structural bolts, their A563 heavy hex nuts and the
// Table 3.1 standard hole per row — re-entered by hand as our own data; provenance and licence
// are in `ATTRIBUTION.md` and in the library's own metadata.

//! The fastener library of the `usa` environment end to end through the real `beamcraft`
//! binary: the environment lists the RCSC set beside its profile library, every admitted entry
//! carries the declared catalogue values, and a project created with the environment resolves a
//! bolt designation through its own copy without any `--fastener-library` option — the reply's
//! lookup naming the answering layer and library (#787) — while a designation the pack does not
//! list stays `NOT_FOUND`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, Scripted, at, list, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

const ENVIRONMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/usa/environment.usda"
);
const LIBRARY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../environments/usa/fasteners-rcsc-2025.usda"
);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000d4";
// The pinned element ids of the bolt round, so the whole round rides one process (#738 P7).
const P1: &str = "0199e7a0-0000-7000-8000-00000000f0d1";
const P2: &str = "0199e7a0-0000-7000-8000-00000000f0d2";
const GROUP: &str = "0199e7a0-0000-7000-8000-00000000f0d3";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("usa-fasteners-tests"),
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

#[test]
fn the_usa_environment_lists_and_resolves_its_fastener_set() {
    let f = Fixture::new();

    // The environment listing names the fastener library beside the profile library.
    let listed = f.ok(&["list-library", "--environment", ENVIRONMENT]);
    assert_eq!(
        at(&listed, &["environment", "fastener_libraries"])
            .array()
            .unwrap(),
        [Json::text("fasteners-rcsc-2025.usda")]
    );

    // The project carries the fastener library in its own environment copy.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &f.path("models"),
        "--name",
        "USA Bolts",
        "--actor-id",
        ACTOR,
        "--config-dir",
        &f.path("config"),
        "--environment",
        "usa",
    ]);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        std::path::Path::new(&store)
            .join("environment")
            .join("fasteners-rcsc-2025.usda")
            .is_file(),
        "the project copy holds the RCSC fastener library"
    );

    // The bolt round rides one process (#738 P7): a bolt designation of the set resolves
    // through the project's own environment copy, with no --fastener-library option naming
    // any file, and the reply's lookup names the answering layer and library (#787). A325-3/4
    // through 2 x 20 mm needs grip 40 + head 11.90625 + nut 18.653125 + protrusion 6 =
    // 76.56, so the declared lengths grid answers 82.55 (3-1/4 in).
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
            "SE-A325",
            "--parts",
            parts.as_str(),
            "--designation",
            "A325-3/4",
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
        // grade ride into the model, which never needs the library file again.
        Scripted::ok(&["definitions", "--store", &store, "--name", "A325-3/4"]),
        // A repin resolves through the environment the same way: A490-1 over the same grip
        // answers 40 + 15.478125 + 25.003125 + 8 = 88.48, so the grid gives 88.9 (3-1/2 in) —
        // a different list answer than A325-3/4's, which proves which entry resolved.
        Scripted::ok(&[
            "update-bolt-group",
            "--store",
            &store,
            "--target",
            GROUP,
            "--repin",
            "true",
            "--designation",
            "A490-1",
        ]),
    ];
    // A designation the pack does not list stays NOT_FOUND, never a near match: the metric
    // A325M series is outside the slice, M20 is not an inch designation, the washers are
    // deferred (F436 is paywalled), no oversize row exists, and the lookup never answers a
    // lowercase variant of a listed designation.
    let absent = ["A325M-16", "M20", "F436-3/4", "A325-3/4-OVS", "a325-3/4"];
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
    assert_eq!(text(&groups[0], "name"), "SE-A325");
    assert_eq!(text(&groups[0], "length_mm"), "82.55");
    // The creation reply says what the name resolved through (#787): the project layer, the
    // environment library, nothing shadowed.
    let resolved = list(made, "lookups")
        .iter()
        .find(|l| text(l, "namespace") == "fastener")
        .unwrap();
    assert_eq!(text(resolved, "name"), "A325-3/4");
    assert_eq!(text(resolved, "layer"), "project");
    assert_eq!(text(resolved, "library"), "beamcraft-rcsc-fasteners");
    assert_eq!(list(resolved, "shadowed").len(), 0);
    let pinned = list(&replies[3], "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "fastener");
    assert_eq!(text(&pinned[0], "designation"), "A325-3/4");
    assert_eq!(text(&pinned[0], "standard"), "ASTM F3125");
    assert_eq!(text(&pinned[0], "grade"), "A325");
    assert_eq!(num(&pinned[0], &["nominal_diameter_mm"]), 19.05);
    assert_eq!(num(&pinned[0], &["hole_clearance_mm"]), 1.5875);
    let repinned = &replies[4];
    let groups = list(repinned, "bolt_groups");
    assert_eq!(text(&groups[0], "name"), "SE-A325");
    assert_eq!(text(&groups[0], "length_mm"), "88.9");

    // Replies 5.. carry the refusals in the authored order.
    for (index, designation) in absent.iter().enumerate() {
        assert_eq!(
            text(&replies[5 + index], "code"),
            "NOT_FOUND",
            "{designation}"
        );
    }
}

#[test]
fn the_usa_pack_lists_the_rcsc_rows_and_refuses_the_unpublisheds() {
    let f = Fixture::new();

    // The shipped library admits every row and refuses none: nine sizes of each grade, the
    // designations sorted, the size the tables print with its hyphens and fraction.
    let pack = f.ok(&["list-library", "--fastener-library", LIBRARY]);
    assert_eq!(num(&pack, &["entry_count"]), 18.0);
    assert_eq!(num(&pack, &["rejected_count"]), 0.0);
    let designations: Vec<String> = at(&pack, &["entries"])
        .array()
        .unwrap()
        .iter()
        .map(|e| text(e, "designation"))
        .collect();
    assert_eq!(
        designations,
        [
            "A325-1",
            "A325-1-1/2",
            "A325-1-1/4",
            "A325-1-1/8",
            "A325-1-3/8",
            "A325-1/2",
            "A325-3/4",
            "A325-5/8",
            "A325-7/8",
            "A490-1",
            "A490-1-1/2",
            "A490-1-1/4",
            "A490-1-1/8",
            "A490-1-3/8",
            "A490-1/2",
            "A490-3/4",
            "A490-5/8",
            "A490-7/8"
        ]
    );

    // One row field by field: the C-2.1 head and nut of F3125 Grade A325, 3/4 in, and the
    // Table 3.1 standard hole (13/16 in for the 3/4-in bolt, clearance 1/16 in). The washers
    // (F436) are deferred: both switches false and no washer dimensions declared.
    let shown = f.ok(&[
        "show-library",
        "--fastener-library",
        LIBRARY,
        "--designation",
        "A325-3/4",
    ]);
    let entry = at(&shown, &["entry"]);
    assert_eq!(text(entry, "designation"), "A325-3/4");
    assert_eq!(text(entry, "standard"), "ASTM F3125");
    assert_eq!(text(entry, "grade"), "A325");
    assert_eq!(text(entry, "fastener_kind"), "bolt");
    assert_eq!(text(entry, "head_kind"), "hex");
    assert_eq!(num(entry, &["nominal_diameter_mm"]), 19.05);
    assert_eq!(num(entry, &["head_across_flats_mm"]), 31.75);
    assert_eq!(num(entry, &["head_height_mm"]), 11.90625);
    assert_eq!(num(entry, &["nut_across_flats_mm"]), 31.75);
    assert_eq!(num(entry, &["nut_height_mm"]), 18.653125);
    assert_eq!(num(entry, &["thread_protrusion_mm"]), 6.0);
    assert_eq!(num(entry, &["hole_clearance_mm"]), 1.5875);
    assert!(!at(entry, &["washer_under_head"]).boolean().unwrap());
    assert!(!at(entry, &["washer_under_nut"]).boolean().unwrap());
    assert_eq!(text(entry, "hole_mode"), "generate");
    assert_eq!(lengths(entry).len(), 22);
    // The library metadata carries the declared provenance.
    assert_eq!(
        text(at(&shown, &["library"]), "id"),
        "beamcraft-rcsc-fasteners"
    );
    assert_eq!(text(at(&shown, &["library"]), "licence"), "MPL-2.0");

    // A designation the set does not publish stays NOT_FOUND.
    for designation in ["A325M-20", "M20", "F436-3/4", "A325-3/4-OVS"] {
        assert_eq!(
            f.code(&[
                "show-library",
                "--fastener-library",
                LIBRARY,
                "--designation",
                designation
            ]),
            "NOT_FOUND",
            "{designation}"
        );
    }
}
