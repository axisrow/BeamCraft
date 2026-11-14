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

use crate::support::{self, at, list, text};

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

    // A bolt designation of the set resolves through the project's own environment copy, with
    // no --fastener-library option naming any file. M16 through 2 x 20 mm needs grip 40 +
    // washers 2 x 3 + head 10 + nut 14.8 + protrusion 5 = 75.8, so the declared lengths list
    // answers 80.
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
    assert_eq!(text(&groups[0], "name"), "SE16");
    assert_eq!(text(&groups[0], "length_mm"), "80");
    let group = text(&made, "element_id");

    // The pinned definition is the catalogue entry itself: the declared standard and diameter
    // ride into the model, which never needs the library file again.
    let definitions = f.ok(&["definitions", "--store", &store, "--name", "M16"]);
    let pinned = list(&definitions, "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "fastener");
    assert_eq!(text(&pinned[0], "designation"), "M16");
    assert_eq!(text(&pinned[0], "standard"), "DIN EN ISO 4014");
    assert_eq!(num(&pinned[0], &["nominal_diameter_mm"]), 16.0);
    assert_eq!(num(&pinned[0], &["hole_clearance_mm"]), 2.0);

    // A repin resolves through the environment the same way: M12 over the same grip answers
    // 40 + 2 x 2.5 + 7.5 + 10.8 + 4 = 67.3, so the declared list gives 70 — a different list
    // than M16's, which proves which entry the resolver answered.
    let repinned = f.ok(&[
        "update-bolt-group",
        "--store",
        &store,
        "--target",
        &group,
        "--repin",
        "true",
        "--designation",
        "M12",
    ]);
    let groups = list(&repinned, "bolt_groups");
    assert_eq!(text(&groups[0], "name"), "SE16");
    assert_eq!(text(&groups[0], "length_mm"), "70");

    // A designation the set does not publish stays NOT_FOUND, never a near match.
    for absent in ["M24", "m16"] {
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

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
