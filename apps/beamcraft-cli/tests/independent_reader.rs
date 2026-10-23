// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and driver, MPL-2.0. The oracle is a std-only Rust
// binary that shares no code with the layer engine.

//! Independent-reader oracle: plates created by the real `beamcraft create-plate` binary are
//! re-read from the committed layer text by an independent reader (the
//! `beamcraft-usd-oracle` process, with its own parser, composition and vector algebra) and
//! compared with the closed form and the CLI output.
//!
//! Unlike the retired OpenUSD Python oracle, the reader runs everywhere and skips nowhere: the
//! independent-reader gate runs in every `cargo test`, locally and in CI.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run_binary(args: &[&str]) -> Json {
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{args:?}");
    beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap()
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}

fn num(json: &Json, path: &[&str]) -> f64 {
    path.iter()
        .fold(json, |j, k| j.get(k).unwrap())
        .float()
        .unwrap()
}

fn plate_args<'a>(
    store: &'a str,
    w: &'a str,
    l: &'a str,
    t: &'a str,
    extra: &[&'a str],
) -> Vec<&'a str> {
    let mut args = vec![
        "create-plate",
        "--store",
        store,
        "--width",
        w,
        "--length",
        l,
        "--thickness",
        t,
    ];
    args.extend_from_slice(extra);
    args
}

/// A fresh scratch root with a store and a layers directory, removed on drop.
struct Scratch {
    root: PathBuf,
    store: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = fs::remove_dir_all(&base);
        let root = base.join(format!("run-{}", std::process::id()));
        let store = root.join("store");
        fs::create_dir_all(&store).unwrap();
        fs::create_dir_all(root.join("layers")).unwrap();
        Scratch { root, store }
    }

    fn store_arg(&self) -> &str {
        self.store.to_str().unwrap()
    }

    fn layers(&self) -> PathBuf {
        self.root.join("layers")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
/// Copies the layers of the head revision under their manifest names, where a plain USD reader
/// finds them next to the root layer.
fn copy_layers(store: &Path, layers: &Path, inspected: &Json) {
    for entry in inspected
        .get("revision")
        .unwrap()
        .get("manifest")
        .unwrap()
        .get("layers")
        .unwrap()
        .array()
        .unwrap()
    {
        let digest = text(entry, "digest");
        fs::copy(
            store.join("blobs").join(&digest[..2]).join(&digest),
            layers.join(text(entry, "name")),
        )
        .unwrap();
    }
}

/// Runs the independent reader over the copied layers and returns its report after checking
/// that the reader passed its own structural and analytical checks.
fn run_oracle(layers: &Path, expect: &str) -> Json {
    let expect_path = layers.join("expect.json");
    fs::write(&expect_path, expect).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft-usd-oracle"))
        .arg("revision")
        .arg(layers.join("root.usda"))
        .arg(&expect_path)
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("READER REPORT: {report}");
    assert!(
        output.status.success(),
        "the independent reader disagreed: {}\n{report}",
        String::from_utf8_lossy(&output.stderr)
    );
    beamcraft_json::decode(report.trim().as_bytes(), &Limits::DEFAULT).unwrap()
}

fn plates(report: &Json) -> usize {
    report.get("plates").unwrap().array().unwrap().len()
}

#[test]
fn a_missing_expectation_file_fails_the_reader() {
    let scratch = Scratch::new("no-expect");
    let store_arg = scratch.store_arg();
    run_binary(&plate_args(
        store_arg,
        "1000",
        "500",
        "10",
        &["--name", "Oracle A"],
    ));
    let inspected = run_binary(&["plate-inspect", "--store", store_arg]);
    copy_layers(Path::new(store_arg), &scratch.layers(), &inspected);
    let absent = scratch.layers().join("absent-expectation.json");
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft-usd-oracle"))
        .arg("revision")
        .arg(scratch.layers().join("root.usda"))
        .arg(&absent)
        .output()
        .unwrap();
    assert_ne!(
        output.status.code(),
        Some(0),
        "a missing expectation file must fail the reader, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn the_reader_reads_the_committed_revision_and_agrees_with_the_closed_form() {
    let scratch = Scratch::new("p01-reader");
    let store_arg = scratch.store_arg();
    let a = run_binary(&plate_args(
        store_arg,
        "1000",
        "500",
        "10",
        &["--name", "Oracle A"],
    ));
    let b = run_binary(&plate_args(
        store_arg,
        "800",
        "300",
        "12.5",
        &["--translation", "2000,-150.5,25", "--name", "Oracle B"],
    ));
    let inspected = run_binary(&["plate-inspect", "--store", store_arg]);
    copy_layers(Path::new(store_arg), &scratch.layers(), &inspected);
    let expectation = |reply: &Json, name: &str, w: f64, l: f64, t: f64, tr: [f64; 3]| {
        format!(
            r#"{{"id":"{}","name":"{name}","width":{w:?},"length":{l:?},"thickness":{t:?},"translation":[{:?},{:?},{:?}],"cli_volume_mm3":{:?}}}"#,
            text(reply.get("plate").unwrap(), "element_id"),
            tr[0],
            tr[1],
            tr[2],
            num(reply, &["plate", "quantities", "volume", "mm3"]),
        )
    };
    let expect = format!(
        "[{},{}]",
        expectation(&a, "Oracle A", 1000.0, 500.0, 10.0, [0.0, 0.0, 0.0]),
        expectation(&b, "Oracle B", 800.0, 300.0, 12.5, [2000.0, -150.5, 25.0]),
    );
    let report = run_oracle(&scratch.layers(), &expect);
    assert_eq!(plates(&report), 2);
}

#[test]
fn the_independent_reader_agrees_with_both_clients_after_rebase_and_publication() {
    let scratch = Scratch::new("c03-independent-reader");
    let store = scratch.store_arg();
    let first = run_binary(&plate_args(
        store,
        "1000",
        "500",
        "10",
        &["--name", "Initial"],
    ));
    let second = run_binary(&plate_args(
        store,
        "800",
        "300",
        "12.5",
        &["--name", "Untouched", "--translation", "2000,-150.5,25"],
    ));
    let first_id = text(first.get("plate").unwrap(), "element_id");
    let second_id = text(second.get("plate").unwrap(), "element_id");
    let a = scratch.root.join("a");
    let b = scratch.root.join("b");
    let a = a.to_str().unwrap();
    let b = b.to_str().unwrap();
    for workspace in [a, b] {
        run_binary(&[
            "workspace-create",
            "--store",
            store,
            "--workspace-dir",
            workspace,
        ]);
    }
    run_binary(&[
        "update-element",
        "--store",
        a,
        "--id",
        &first_id,
        "--name",
        "Accepted name",
    ]);
    run_binary(&[
        "update-element",
        "--store",
        b,
        "--id",
        &first_id,
        "--translation",
        "12,34,56",
    ]);
    let original = run_binary(&["revision-open", "--store", b]);
    run_binary(&["workspace-publish", "--store", store, "--workspace-dir", a]);
    let accepted = run_binary(&["revision-open", "--store", store]);
    run_binary(&[
        "rebase-workspace",
        "--store",
        store,
        "--workspace-dir",
        b,
        "--op-id",
        "019a7bf0-0000-7000-8000-00000000c302",
        "--expected-head",
        &text(&accepted, "revision_id"),
        "--expected-draft",
        &text(&original, "revision_id"),
    ]);
    run_binary(&["workspace-publish", "--store", store, "--workspace-dir", b]);
    let inspected = run_binary(&["plate-inspect", "--store", store]);
    copy_layers(&scratch.store, &scratch.layers(), &inspected);
    // Expected geometry/placement/volume are explicit closed forms, not copied from an engine
    // snapshot. The other process shares no USD parser, composition or geometry implementation.
    let expected = format!(
        r#"[
        {{"id":"{first_id}","name":"Accepted name","width":1000,"length":500,"thickness":10,"translation":[12,34,56],"cli_volume_mm3":5000000}},
        {{"id":"{second_id}","name":"Untouched","width":800,"length":300,"thickness":12.5,"translation":[2000,-150.5,25],"cli_volume_mm3":3000000}}
    ]"#
    );
    assert_eq!(plates(&run_oracle(&scratch.layers(), &expected)), 2);
}

#[test]
fn the_independent_reader_retains_a_later_users_edits_after_compensation() {
    let scratch = Scratch::new("c06-independent-reader");
    let store = scratch.store_arg();
    let owner = "019a7bf0-0000-7000-8000-00000000c603";
    let other = "019a7bf0-0000-7000-8000-00000000c604";
    let first = run_binary(&plate_args(
        store,
        "1000",
        "500",
        "10",
        &["--name", "Before"],
    ));
    let second = run_binary(&plate_args(
        store,
        "800",
        "300",
        "12.5",
        &["--name", "Other"],
    ));
    let first_id = text(first.get("plate").unwrap(), "element_id");
    let second_id = text(second.get("plate").unwrap(), "element_id");
    let a = scratch.root.join("a");
    let b = scratch.root.join("b");
    let c = scratch.root.join("c");
    let a = a.to_str().unwrap();
    let b = b.to_str().unwrap();
    let c = c.to_str().unwrap();
    run_binary(&[
        "workspace-create",
        "--store",
        store,
        "--workspace-dir",
        a,
        "--user-id",
        owner,
    ]);
    run_binary(&[
        "update-element",
        "--store",
        a,
        "--id",
        &first_id,
        "--name",
        "Accepted",
    ]);
    let package = run_binary(&["changeset-create", "--store", store, "--workspace-dir", a]);
    run_binary(&[
        "changeset-apply",
        "--store",
        store,
        "--workspace-dir",
        a,
        "--actor-id",
        owner,
        "--changeset",
        &text(package.get("changeset").unwrap(), "digest"),
    ]);
    let original = run_binary(&["revision-open", "--store", store]);
    run_binary(&[
        "workspace-create",
        "--store",
        store,
        "--workspace-dir",
        b,
        "--user-id",
        other,
    ]);
    run_binary(&[
        "update-element",
        "--store",
        b,
        "--id",
        &first_id,
        "--translation",
        "12,34,56",
    ]);
    run_binary(&[
        "update-element",
        "--store",
        b,
        "--id",
        &second_id,
        "--name",
        "Other later",
    ]);
    run_binary(&["workspace-publish", "--store", store, "--workspace-dir", b]);
    let accepted = run_binary(&["revision-open", "--store", store]);
    run_binary(&[
        "workspace-create",
        "--store",
        store,
        "--workspace-dir",
        c,
        "--user-id",
        owner,
    ]);
    let draft = run_binary(&["revision-open", "--store", c]);
    run_binary(&[
        "compensate-operation",
        "--store",
        store,
        "--workspace-dir",
        c,
        "--source-workspace-dir",
        a,
        "--original-revision",
        &text(&original, "revision_id"),
        "--op-id",
        "019a7bf0-0000-7000-8000-00000000c605",
        "--expected-head",
        &text(&accepted, "revision_id"),
        "--expected-draft",
        &text(&draft, "revision_id"),
    ]);
    assert_eq!(
        text(
            &run_binary(&["revision-open", "--store", store]),
            "revision_id"
        ),
        text(&accepted, "revision_id")
    );
    run_binary(&["workspace-publish", "--store", store, "--workspace-dir", c]);
    let inspected = run_binary(&["plate-inspect", "--store", store]);
    copy_layers(&scratch.store, &scratch.layers(), &inspected);
    let expected = format!(
        r#"[
        {{"id":"{first_id}","name":"Before","width":1000,"length":500,"thickness":10,"translation":[12,34,56],"cli_volume_mm3":5000000}},
        {{"id":"{second_id}","name":"Other later","width":800,"length":300,"thickness":12.5,"translation":[0,0,0],"cli_volume_mm3":3000000}}
    ]"#
    );
    assert_eq!(plates(&run_oracle(&scratch.layers(), &expected)), 2);
}

#[test]
fn the_reader_reads_pinned_materials_and_the_mass_agrees_with_the_closed_form() {
    let scratch = Scratch::new("p02-reader");
    let store_arg = scratch.store_arg();
    let digest_of = |reply: &Json| text(reply.get("definition").unwrap(), "digest");
    let steel = digest_of(&run_binary(&[
        "define-material",
        "--store",
        store_arg,
        "--density",
        "7850",
    ]));
    let bare = digest_of(&run_binary(&["define-material", "--store", store_arg]));
    // A profile is published too: the reader must see it as an ordinary immutable definition prim.
    run_binary(&[
        "define-profile",
        "--store",
        store_arg,
        "--width",
        "200",
        "--height",
        "300",
    ]);
    let pinned = run_binary(&plate_args(
        store_arg,
        "1000",
        "500",
        "10",
        &["--name", "Steel", "--material-digest", &steel],
    ));
    let unknown = run_binary(&plate_args(
        store_arg,
        "800",
        "300",
        "12.5",
        &["--name", "Unknown", "--material-digest", &bare],
    ));
    let plain = run_binary(&plate_args(
        store_arg,
        "600",
        "400",
        "8",
        &["--name", "Plain"],
    ));
    let inspected = run_binary(&["plate-inspect", "--store", store_arg]);
    copy_layers(Path::new(store_arg), &scratch.layers(), &inspected);
    let expectation = |reply: &Json,
                       name: &str,
                       w: f64,
                       l: f64,
                       t: f64,
                       material: Option<(&str, Option<f64>)>| {
        let quantities = |key: &[&str]| {
            let mut node = reply.get("plate").unwrap().get("quantities").unwrap();
            for k in key {
                node = node.get(k).unwrap();
            }
            node
        };
        let mass = match quantities(&["mass", "kg"]) {
            Json::Null => "null".to_owned(),
            value => format!("{:?}", value.float().unwrap()),
        };
        let material = material.map_or("null".to_owned(), |(digest, density)| {
            let density = density.map_or("null".to_owned(), |d| format!("{d:?}"));
            format!(r#"{{"digest":"{digest}","density":{density}}}"#)
        });
        format!(
            r#"{{"id":"{}","name":"{name}","width":{w:?},"length":{l:?},"thickness":{t:?},"translation":[0.0,0.0,0.0],"cli_volume_mm3":{:?},"cli_mass_kg":{mass},"material":{material}}}"#,
            text(reply.get("plate").unwrap(), "element_id"),
            num(reply, &["plate", "quantities", "volume", "mm3"]),
        )
    };
    let expect = format!(
        "[{},{},{}]",
        expectation(
            &pinned,
            "Steel",
            1000.0,
            500.0,
            10.0,
            Some((&steel, Some(7850.0)))
        ),
        expectation(&unknown, "Unknown", 800.0, 300.0, 12.5, Some((&bare, None))),
        expectation(&plain, "Plain", 600.0, 400.0, 8.0, None),
    );
    // The closed-form mass of the pinned plate: 0.005 m3 * 7850 kg/m3.
    assert_eq!(num(&pinned, &["plate", "quantities", "mass", "kg"]), 39.25);
    let report = run_oracle(&scratch.layers(), &expect);
    assert_eq!(plates(&report), 3);
}

#[test]
fn the_reader_reads_a_plate_with_through_holes_against_the_closed_form() {
    let scratch = Scratch::new("a04-reader");
    let store_arg = scratch.store_arg();
    let created = run_binary(&plate_args(
        store_arg,
        "1000",
        "500",
        "10",
        &[
            "--name",
            "Holed",
            "--hole-rect",
            "600,150,200,200",
            "--hole-round",
            "250,250,100",
        ],
    ));
    let inspected = run_binary(&["plate-inspect", "--store", store_arg]);
    copy_layers(Path::new(store_arg), &scratch.layers(), &inspected);
    // Closed form: the box minus a 200 x 200 square and a regular 32-gon inscribed in a circle of
    // diameter 100, whose area is (32 / 2) * r^2 * sin(2 pi / 32).
    let polygon = 16.0 * 50.0 * 50.0 * (std::f64::consts::PI / 16.0).sin();
    let expected = (1000.0 * 500.0 - 200.0 * 200.0 - polygon) * 10.0;
    assert!((num(&created, &["plate", "quantities", "volume", "mm3"]) - expected).abs() < 1e-3);
    let expect = format!(
        r#"[{{"id":"{}","name":"Holed","width":1000.0,"length":500.0,"thickness":10.0,"translation":[0.0,0.0,0.0],"cli_volume_mm3":{:?},"expected_volume_mm3":{expected:?},"hole_counts":[4,32]}}]"#,
        text(created.get("plate").unwrap(), "element_id"),
        num(&created, &["plate", "quantities", "volume", "mm3"]),
    );
    let report = run_oracle(&scratch.layers(), &expect);
    assert_eq!(plates(&report), 1);
}

#[test]
fn the_reader_reads_a_beam_and_a_column_against_the_closed_form() {
    let scratch = Scratch::new("p03-reader");
    let store_arg = scratch.store_arg();
    let digest_of = |reply: &Json| text(reply.get("definition").unwrap(), "digest");
    let steel = digest_of(&run_binary(&[
        "define-material",
        "--store",
        store_arg,
        "--density",
        "7850",
    ]));
    let profile = digest_of(&run_binary(&[
        "define-profile",
        "--store",
        store_arg,
        "--width",
        "200",
        "--height",
        "300",
    ]));
    let beam = run_binary(&[
        "create-member",
        "--store",
        store_arg,
        "--kind",
        "beam",
        "--start",
        "0,0,0",
        "--end",
        "6000,0,0",
        "--profile-digest",
        &profile,
        "--material-digest",
        &steel,
        "--name",
        "Beam",
    ]);
    let column = run_binary(&[
        "create-member",
        "--store",
        store_arg,
        "--kind",
        "column",
        "--start",
        "0,0,0",
        "--end",
        "0,0,3000",
        "--profile-digest",
        &profile,
        "--roll",
        "1.5707963267948966",
        "--name",
        "Column",
    ]);
    let inspected = run_binary(&["plate-inspect", "--store", store_arg]);
    copy_layers(Path::new(store_arg), &scratch.layers(), &inspected);
    let member = |reply: &Json, name: &str, kind: &str, roll: &str, tail: &str| {
        let m = reply.get("member").unwrap();
        let mass = match m
            .get("quantities")
            .unwrap()
            .get("mass")
            .unwrap()
            .get("kg")
            .unwrap()
        {
            Json::Null => "null".to_owned(),
            value => format!("{:?}", value.float().unwrap()),
        };
        format!(
            r#"{{"kind":"{kind}","id":"{}","name":"{name}","axis_start":{a0},"axis_end":{a1},"roll":{roll},"profile":"{profile}","width":200.0,"height":300.0,"cli_volume_mm3":{:?},"cli_mass_kg":{mass},"bounds_min":{bmin},"bounds_max":{bmax},{tail}}}"#,
            text(m, "element_id"),
            num(reply, &["member", "quantities", "volume", "mm3"]),
            a0 = "[0.0,0.0,0.0]",
            a1 = if kind == "beam" {
                "[6000.0,0.0,0.0]"
            } else {
                "[0.0,0.0,3000.0]"
            },
            bmin = if kind == "beam" {
                "[0.0,-100.0,-150.0]"
            } else {
                "[-150.0,-100.0,0.0]"
            },
            bmax = if kind == "beam" {
                "[6000.0,100.0,150.0]"
            } else {
                "[150.0,100.0,3000.0]"
            },
        )
    };
    // Hand-derived from F06 section 4: the +X axis gives Z_m = +X, Y_m = +Z, X_m = +Y, so the
    // 200 mm width spans Y and the 300 mm height spans Z. The vertical axis gives X_m = +X and
    // Y_m = +Y; the quarter-turn roll takes X_m toward Y_m, so the width spans Y and the height X.
    let expect = format!(
        "[{},{}]",
        member(
            &beam,
            "Beam",
            "beam",
            "null",
            r#""volume_mm3":360000000.0,"density":7850.0"#
        ),
        member(
            &column,
            "Column",
            "column",
            "1.5707963267948966",
            r#""volume_mm3":180000000.0,"density":null"#
        ),
    );
    assert_eq!(num(&beam, &["member", "quantities", "mass", "kg"]), 2826.0);
    let report = run_oracle(&scratch.layers(), &expect);
    assert_eq!(plates(&report), 2);
}
