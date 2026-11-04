// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic IFC text, MPL-2.0.

//! E04 end to end through the real CLI binary: `ifc-inspect` pins the provenance, inventory,
//! units and losses of received IFC bytes and refuses everything that is not a bounded
//! ISO 10303-21 file. No store is opened and nothing is written.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support::{at, text};

use beamcraft_json::Json;
use std::fs;
use std::process::Command;

/// A contributor-original synthetic IFC4 file: one project, one millimetre unit assignment,
/// two elements, one duplicate GlobalId, one slab whose duplicate is listed.
const VALID: &str = "ISO-10303-21;\n\
HEADER;\n\
FILE_DESCRIPTION(('synthetic study file'),'2;1');\n\
FILE_NAME('site.ifc','2026-11-05T09:00:00',('study'),('apibim'),'handwritten','e04-suite','');\n\
FILE_SCHEMA(('IFC4'));\n\
ENDSEC;\n\
DATA;\n\
#1=IFCPROJECT('0YvucVMOX9kw8jhtQ9abcd',*,$,$,$,$,$,$,#2);\n\
#2=IFCUNITASSIGNMENT((#4));\n\
#3=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#5,$);\n\
#4=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);\n\
#5=IFCAXIS2PLACEMENT3D(#6,$,$);\n\
#6=IFCCARTESIANPOINT((0.,0.,0.));\n\
#7=IFCWALL('1Anr5VMOX0PO8jvHqZabcd',*,'wall',$,$,$,$,$,$);\n\
#8=IFCSLAB('2Bnr5VMOX0PO8jvHqZabcd',*,'slab',$,$,$,$,$,$);\n\
#9=IFCSLAB('1Anr5VMOX0PO8jvHqZabcd',*,'dup',$,$,$,$,$,$);\n\
ENDSEC;\n\
END-ISO-10303-21;\n";

/// The same body declaring IFC2X3: a loss, never a refusal, and never a conversion.
fn ifc2x3() -> String {
    VALID.replace("FILE_SCHEMA(('IFC4'));", "FILE_SCHEMA(('IFC2X3'));")
}

struct Fixture {
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "e04-ifc-inspect-{}-{}",
            std::process::id(),
            crate::support::boot_nonce()
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn file(&self, name: &str, body: &str) -> String {
        let path = self.root.join(name);
        fs::write(&path, body).unwrap();
        path.to_str().unwrap().to_owned()
    }
    fn run(&self, source: &str) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(["ifc-inspect", "--source", source])
            .output()
            .unwrap();
        let body =
            beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
}

/// The text of a JSON number at a `path`.
fn number<'a>(json: &'a Json, path: &[&str]) -> &'a str {
    match at(json, path) {
        Json::Number(text) => text,
        other => panic!("a number at {path:?}: {other:?}"),
    }
}

/// The value of a JSON bool at a `path`.
fn boolean(json: &Json, path: &[&str]) -> bool {
    match at(json, path) {
        Json::Bool(value) => *value,
        other => panic!("a bool at {path:?}: {other:?}"),
    }
}

#[test]
fn a_valid_file_is_pinned_deterministically() {
    let fixture = Fixture::new();
    let path = fixture.file("site.ifc", VALID);
    let (code, body) = fixture.run(&path);
    assert_eq!(code, 0, "{body:?}");
    assert_eq!(text(&body, "outcome"), "ifc_inspected");
    let source = at(&body, &["source"]);
    let expected = beamcraft_model::digest::Sha256::digest(VALID.as_bytes())
        .ok()
        .unwrap()
        .to_string();
    assert_eq!(text(source, "sha256"), expected);
    assert_eq!(number(source, &["bytes"]), VALID.len().to_string());
    let (_, second) = fixture.run(&path);
    assert_eq!(body, second);
}

#[test]
fn the_declared_facts_reach_the_report() {
    let fixture = Fixture::new();
    let path = fixture.file("site.ifc", VALID);
    let (_, body) = fixture.run(&path);
    let schema = at(&body, &["declared_schema"]);
    assert_eq!(text(schema, "token"), "IFC4");
    assert!(boolean(schema, &["recognized"]));
    let header = at(&body, &["header"]);
    assert_eq!(text(header, "file_name"), "site.ifc");
    assert_eq!(text(header, "originating_system"), "e04-suite");
    assert_eq!(text(header, "preprocessor_version"), "handwritten");
    let entities = at(&body, &["entities"]);
    assert_eq!(number(entities, &["count"]), "9");
    let global_ids = at(&body, &["global_ids"]);
    assert_eq!(number(global_ids, &["count"]), "4");
    assert_eq!(number(global_ids, &["distinct"]), "3");
    assert_eq!(number(global_ids, &["duplicates"]), "1");
    assert!(!boolean(global_ids, &["project_without_global_id"]));
    let unit = at(&body, &["length_unit"]);
    assert_eq!(number(unit, &["meters_per_unit"]), "0.001");
    assert_eq!(text(unit, "origin"), "project_units");
    assert!(!boolean(&body, &["losses_truncated"]));
    assert_eq!(at(&body, &["losses"]).array().unwrap().len(), 0, "{body:?}");
}

#[test]
fn a_duplicate_global_id_is_listed_with_its_instance() {
    let fixture = Fixture::new();
    let path = fixture.file("site.ifc", VALID);
    let (_, body) = fixture.run(&path);
    let samples = at(&body, &["global_ids", "duplicate_samples"])
        .array()
        .unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(text(&samples[0], "global_id"), "1Anr5VMOX0PO8jvHqZabcd");
}

#[test]
fn a_declared_other_schema_is_a_loss_not_a_refusal() {
    let fixture = Fixture::new();
    let path = fixture.file("old.ifc", &ifc2x3());
    let (code, body) = fixture.run(&path);
    assert_eq!(code, 0, "{body:?}");
    let schema = at(&body, &["declared_schema"]);
    assert_eq!(text(schema, "token"), "IFC2X3");
    assert!(!boolean(schema, &["recognized"]));
    let losses = at(&body, &["losses"]).array().unwrap();
    assert!(
        losses
            .iter()
            .any(|loss| text(loss, "code") == "SCHEMA_OUTSIDE_STUDIED_SET"),
        "{losses:?}"
    );
}

#[test]
fn bytes_that_are_not_step_are_refused() {
    let fixture = Fixture::new();
    let path = fixture.file("garbage.ifc", "hello world, not a step file\n");
    let (code, body) = fixture.run(&path);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    let (code, body) = fixture.run("https://example.com/model.ifc");
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "PERMISSION_DENIED");
    let (code, body) = fixture.run(fixture.root.join("missing.ifc").to_str().unwrap());
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "NOT_FOUND");
}

#[test]
fn a_file_beyond_the_cap_is_refused_without_parsing() {
    let fixture = Fixture::new();
    let path = fixture.root.join("huge.ifc");
    fs::write(&path, vec![b'a'; 32 * 1024 * 1024 + 1]).unwrap();
    let (code, body) = fixture.run(path.to_str().unwrap());
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "LIMIT_EXCEEDED");
}
