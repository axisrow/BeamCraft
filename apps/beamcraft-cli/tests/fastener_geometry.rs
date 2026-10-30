// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic fasteners that follow no national standard, MPL-2.0.

//! A14.2 end to end through the real `beamcraft` binary: one prototype of a library
//! definition is built as separate closed meshes, a group of `n` shares it, the volumes match
//! the analytic values within the chord rule, and the mass carries its source.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-fasteners.usda"
);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a14-2-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.root.join(name);
        fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, extra: &[&str]) -> Json {
        let mut args = vec!["fastener-geometry"];
        args.extend_from_slice(extra);
        let (code, body) = self.raw(&args);
        assert_eq!(code, 0, "{extra:?}: {body:?}");
        body
    }
    fn refused(&self, extra: &[&str]) -> (i32, Json) {
        let mut args = vec!["fastener-geometry"];
        args.extend_from_slice(extra);
        self.raw(&args)
    }
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn num(json: &Json, key: &str) -> f64 {
    json.get(key).unwrap().float().unwrap()
}
fn components(body: &Json) -> Vec<Json> {
    body.get("components").unwrap().array().unwrap().to_vec()
}
fn component(body: &Json, name: &str) -> Json {
    components(body)
        .into_iter()
        .find(|c| text(c, "component") == name)
        .unwrap()
}
fn is_null(json: &Json) -> bool {
    matches!(json, Json::Null)
}

#[test]
fn a_group_shares_one_prototype_and_reports_the_components() {
    let fixture = Fixture::new();
    let library = fixture.file("lib.usda", &fs::read(SYNTHETIC).unwrap());
    let body = fixture.ok(&[
        "--fastener-library",
        &library,
        "--designation",
        "SYN-BOLT-12x50",
        "--length",
        "50",
        "--count",
        "3",
        "--density",
        "7850",
    ]);
    assert_eq!(text(&body, "outcome"), "fastener_geometry");
    assert_eq!(text(&body, "mass_source"), "computed");
    assert_eq!(num(&body, "count"), 3.0);
    // Five components: both washers, head, shank, nut.
    let rows = components(&body);
    assert_eq!(rows.len(), 5);
    // One prototype: the two washers are the same mesh, and every count is the group size.
    assert_eq!(
        text(&component(&body, "washer_head"), "mesh_digest"),
        text(&component(&body, "washer_nut"), "mesh_digest")
    );
    for row in &rows {
        assert_eq!(num(row, "count"), 3.0);
        assert_eq!(num(row, "volume_mm3"), num(row, "volume_each_mm3") * 3.0);
        assert_eq!(num(row, "mass_kg"), num(row, "volume_mm3") * 7850.0 / 1e9);
    }
    // The volumes match the analytic values within the chord rule: the hex head and nut are
    // exact, the round parts stay inside their circle.
    let head = component(&body, "head");
    let hexagon = 3.0f64.sqrt() / 2.0 * 17.0 * 17.0 * 6.5;
    assert!((num(&head, "volume_each_mm3") - hexagon).abs() < 1e-6);
    let shank = component(&body, "shank");
    let circle = core::f64::consts::PI * 6.0 * 6.0 * 50.0;
    assert!(num(&shank, "volume_each_mm3") <= circle);
    assert!(circle - num(&shank, "volume_each_mm3") < circle * 0.01);
    // The stack: the bearing face is zero, the shank starts at the under-head plane.
    assert_eq!(num(&component(&body, "washer_head"), "z_offset_mm"), 0.0);
    assert_eq!(num(&component(&body, "head"), "z_offset_mm"), 2.0);
    assert_eq!(num(&component(&body, "shank"), "z_offset_mm"), 2.0);
    assert_eq!(num(&component(&body, "nut"), "z_offset_mm"), 52.0);
    assert_eq!(num(&component(&body, "washer_nut"), "z_offset_mm"), 61.0);
    // The chord rule: a multiple of four vertices, at most 0.05 mm deviation.
    for name in ["washer_head", "shank"] {
        let row = component(&body, name);
        let approximation = row.get("approximation").unwrap();
        assert_eq!(num(approximation, "max_sagitta_mm"), 0.05);
        assert!(num(approximation, "sagitta_bound_mm") <= 0.05);
        assert_eq!(num(approximation, "segments") as u64 % 4, 0);
    }
    assert!(is_null(
        component(&body, "head").get("approximation").unwrap()
    ));
    // The whole-group figures are the union of the components: the shank runs through the
    // head, so the totals sit below the sum of the rows by exactly that overlap prism.
    let each: f64 = rows.iter().map(|r| num(r, "volume_each_mm3")).sum();
    let overlap = num(&shank, "volume_each_mm3") / num(&shank, "height_mm") * 6.5;
    assert!(num(&body, "volume_each_mm3") < each);
    assert!((num(&body, "volume_each_mm3") - (each - overlap)).abs() < 1e-6);
    assert!((num(&body, "mass_kg") - num(&body, "volume_mm3") * 7850.0 / 1e9).abs() < 1e-9);
}

#[test]
fn a_declared_mass_is_shown_as_declared() {
    let fixture = Fixture::new();
    // The shipped synthetic bolt carries no declared mass; one is added to a copy for this
    // test only.
    let declared = String::from_utf8(fs::read(SYNTHETIC).unwrap())
        .unwrap()
        .replacen(
            "double steel:fastener:threadProtrusionMm = 3",
            "double steel:fastener:threadProtrusionMm = 3\n            double steel:fastener:massEachKg = 0.4",
            1,
        );
    let library = fixture.file("declared.usda", declared.as_bytes());
    let body = fixture.ok(&[
        "--fastener-library",
        &library,
        "--designation",
        "SYN-BOLT-12x50",
        "--length",
        "50",
        "--count",
        "2",
    ]);
    assert_eq!(text(&body, "mass_source"), "declared");
    assert_eq!(num(&body, "mass_kg"), 0.8);
    for row in components(&body) {
        assert!(is_null(row.get("mass_kg").unwrap()));
    }
}

#[test]
fn a_screw_has_no_nut_and_the_switches_can_be_overridden() {
    let fixture = Fixture::new();
    let library = fixture.file("lib.usda", &fs::read(SYNTHETIC).unwrap());
    let body = fixture.ok(&[
        "--fastener-library",
        &library,
        "--designation",
        "SYN-SCREW-10x30",
        "--length",
        "30",
    ]);
    let names: Vec<String> = components(&body)
        .iter()
        .map(|c| text(c, "component"))
        .collect();
    // The screw keeps its washer under the head (the definition's preference) and has no nut
    // and no washer under a nut.
    assert_eq!(
        names,
        vec![
            "washer_head".to_owned(),
            "head".to_owned(),
            "shank".to_owned()
        ]
    );
    // A washer switch the definition cannot honour is refused and changes nothing.
    let (code, body) = fixture.raw(&[
        "fastener-geometry",
        "--fastener-library",
        &library,
        "--designation",
        "SYN-SCREW-10x30",
        "--length",
        "30",
        "--washer-under-nut",
        "true",
    ]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
}

#[test]
fn unknown_designations_and_bad_lengths_are_refused() {
    let fixture = Fixture::new();
    let library = fixture.file("lib.usda", &fs::read(SYNTHETIC).unwrap());
    let (code, body) = fixture.refused(&[
        "--fastener-library",
        library.as_str(),
        "--length",
        "50",
        "--designation",
        "M12-NOT-THERE",
    ]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "NOT_FOUND");
    let (code, body) = fixture.refused(&[
        "--fastener-library",
        library.as_str(),
        "--length",
        "0.0005",
        "--designation",
        "SYN-BOLT-12x50",
    ]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    let (code, body) = fixture.refused(&[
        "--fastener-library",
        library.as_str(),
        "--length",
        "50",
        "--count",
        "513",
        "--designation",
        "SYN-BOLT-12x50",
    ]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "LIMIT_EXCEEDED");
}
