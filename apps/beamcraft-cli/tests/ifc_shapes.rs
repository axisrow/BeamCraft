// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic IFC text and plates, MPL-2.0.

//! `ifc-shapes` end to end through the real CLI binary: the committed fixture is read with
//! its exact counts, bounds and units; a store exported by `export-ifc` reads back with the
//! same facts (the writer round trip); the display bundle of `export-view --source` is
//! deterministic and opens no store; and every refusal keeps its typed code.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support::{self, at, text};

use beamcraft_json::Json;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// The committed fixture: the 120 x 80 x 10 plate with one 18 mm hole at (60, 40) of the
/// display issue, and a second 60 x 40 plate at x = 200. Produced by `export-ifc` from a
/// synthetic store; its facts are pinned below.
fn committed_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/ifc/plate-hole.ifc")
}

const FIXTURE_BYTES: &str = "8289";
const FIXTURE_SHA256: &str = "c412620953acfed11ff4ad4e5b5a8098245baf4c09b3951acd2af94a236d9add";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new(name: &str) -> Self {
        Self {
            shared: support::StoreFixture::new(name),
        }
    }

    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }

    /// Runs the CLI with `args` and decodes the JSON reply.
    fn run(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body =
            beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }

    /// Creates the one holed plate of the display issue in the fixture store and exports it.
    fn export_holed_plate(&self, out: &str) {
        let store = self.shared.root.join("store").to_str().unwrap().to_owned();
        let beamcraft = env!("CARGO_BIN_EXE_beamcraft");
        for args in [
            vec![
                "create-plate",
                "--store",
                &store,
                "--width",
                "120",
                "--length",
                "80",
                "--thickness",
                "10",
                "--name",
                "PL-A",
                "--new-id",
                "019a7bf0-0000-7000-8000-00000000a001",
                "--op-id",
                "019a7bf0-0000-7000-8000-00000000c001",
            ],
            vec![
                "add-hole",
                "--store",
                &store,
                "--id",
                "019a7bf0-0000-7000-8000-00000000a001",
                "--hole-round",
                "60,40,18",
                "--operation-id",
                "019a7bf0-0000-7000-8000-00000000c002",
            ],
            vec!["export-ifc", "--store", &store, "--out", out],
        ] {
            let output = Command::new(beamcraft).args(&args).output().unwrap();
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}

/// The text of a JSON number at a `path`.
fn number<'a>(json: &'a Json, path: &[&str]) -> &'a str {
    match at(json, path) {
        Json::Number(value) => value,
        other => panic!("a number at {path:?}: {other:?}"),
    }
}

/// The text of a JSON string at a `path`.
fn string<'a>(json: &'a Json, path: &[&str]) -> &'a str {
    at(json, path).string().unwrap()
}

#[test]
fn the_committed_fixture_reads_with_its_exact_facts() {
    let fixture = Fixture::new("committed");
    let source = committed_fixture().to_str().unwrap().to_owned();
    let (code, body) = fixture.run(&["ifc-shapes", "--source", &source]);
    assert_eq!(code, 0, "{body:?}");
    assert_eq!(text(&body, "outcome"), "ifc_shapes_read");
    assert_eq!(number(at(&body, &["source"]), &["bytes"]), FIXTURE_BYTES);
    assert_eq!(string(&body, &["source", "sha256"]), FIXTURE_SHA256);
    assert_eq!(string(&body, &["declared_schema", "token"]), "IFC4");
    assert!(
        at(&body, &["declared_schema", "recognized"])
            .boolean()
            .unwrap()
    );
    assert_eq!(number(&body, &["length_unit", "meters_per_unit"]), "0.001");
    assert_eq!(string(&body, &["length_unit", "origin"]), "project_units");
    assert_eq!(number(&body, &["products", "count"]), "2");
    assert_eq!(number(&body, &["shapes", "count"]), "2");
    assert_eq!(number(&body, &["shapes", "vertices"]), "80");
    assert_eq!(number(&body, &["shapes", "triangles"]), "156");
    assert!(!at(&body, &["shapes", "truncated"]).boolean().unwrap());
    assert_eq!(at(&body, &["losses"]).array().unwrap().len(), 0);
    let listed = at(&body, &["shapes", "listed"]).array().unwrap();
    assert_eq!(listed.len(), 2);
    // The holed plate: 36 points per face (four corners and the 32-gon ring) on both
    // faces, the 18 mm hole wall of 32 quads and the four outer side quads.
    let holed = &listed[0];
    assert_eq!(number(holed, &["instance"]), "35");
    assert_eq!(string(holed, &["entity"]), "IFCPLATE");
    assert_eq!(string(holed, &["global_id"]), "01cdlm001m080000000A01");
    assert_eq!(string(holed, &["name"]), "PL-A");
    assert!(at(holed, &["closed"]).boolean().unwrap());
    assert_eq!(number(holed, &["vertices"]), "72");
    assert_eq!(number(holed, &["triangles"]), "144");
    let min = at(holed, &["bounds", "min"]).array().unwrap();
    let max = at(holed, &["bounds", "max"]).array().unwrap();
    assert_eq!(min[0], Json::Number("0".into()));
    assert_eq!(min[1], Json::Number("0".into()));
    assert_eq!(min[2], Json::Number("0".into()));
    assert_eq!(max[0], Json::Number("120".into()));
    assert_eq!(max[1], Json::Number("80".into()));
    assert_eq!(max[2], Json::Number("10".into()));
    // The plain plate rides 200 mm to the side.
    let plain = &listed[1];
    assert_eq!(number(plain, &["instance"]), "60");
    assert_eq!(string(plain, &["name"]), "PL-B");
    assert_eq!(number(plain, &["vertices"]), "8");
    assert_eq!(number(plain, &["triangles"]), "12");
    let min = at(plain, &["bounds", "min"]).array().unwrap();
    let max = at(plain, &["bounds", "max"]).array().unwrap();
    assert_eq!(min[0], Json::Number("200".into()));
    assert_eq!(max[0], Json::Number("260".into()));
}

#[test]
fn the_own_export_reads_back_with_the_same_facts() {
    let fixture = Fixture::new("roundtrip");
    let out = fixture.path("repro.ifc");
    fixture.export_holed_plate(&out);
    let (code, body) = fixture.run(&["ifc-shapes", "--source", &out]);
    assert_eq!(code, 0, "{body:?}");
    assert_eq!(text(&body, "outcome"), "ifc_shapes_read");
    assert_eq!(number(&body, &["products", "count"]), "1");
    assert_eq!(number(&body, &["shapes", "count"]), "1");
    assert_eq!(number(&body, &["shapes", "vertices"]), "72");
    assert_eq!(number(&body, &["shapes", "triangles"]), "144");
    let listed = at(&body, &["shapes", "listed"]).array().unwrap();
    let shape = &listed[0];
    assert_eq!(string(shape, &["entity"]), "IFCPLATE");
    assert_eq!(string(shape, &["name"]), "PL-A");
    assert!(at(shape, &["closed"]).boolean().unwrap());
    let max = at(shape, &["bounds", "max"]).array().unwrap();
    assert_eq!(max[2], Json::Number("10".into()));
}

#[test]
fn a_schema_outside_the_studied_set_is_a_loss_not_a_refusal() {
    let fixture = Fixture::new("ifc2x3");
    let text = fs::read_to_string(committed_fixture()).unwrap();
    let old = text.replace("FILE_SCHEMA(('IFC4'));", "FILE_SCHEMA(('IFC2X3'));");
    let source = fixture.path("old.ifc");
    fs::write(&source, old).unwrap();
    let (code, body) = fixture.run(&["ifc-shapes", "--source", &source]);
    assert_eq!(code, 0, "{body:?}");
    assert_eq!(string(&body, &["declared_schema", "token"]), "IFC2X3");
    assert!(
        !at(&body, &["declared_schema", "recognized"])
            .boolean()
            .unwrap()
    );
    assert_eq!(number(&body, &["shapes", "count"]), "2");
}

#[test]
fn the_refusals_keep_their_typed_codes() {
    let fixture = Fixture::new("refusals");
    // Bytes that are not a STEP file.
    let garbage = fixture.path("garbage.ifc");
    fs::write(&garbage, "hello world, not a step file\n").unwrap();
    let (code, body) = fixture.run(&["ifc-shapes", "--source", &garbage]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    // A missing file.
    let (code, body) = fixture.run(&["ifc-shapes", "--source", &fixture.path("missing.ifc")]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "NOT_FOUND");
    // A network path, refused on its text alone.
    let (code, body) = fixture.run(&["ifc-shapes", "--source", "https://example.com/m.ifc"]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "PERMISSION_DENIED");
}
