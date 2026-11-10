// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic scenarios; the fastener values are the shipped synthetic
// library's, which follows no national standard, MPL-2.0.

//! Issue 512 end to end: a committed A14.3 bolt group joins the exported view bundle with its
//! fasteners, so `export-view` and the native viewport show the bolts at the group's bearing
//! points without any re-modelling. Two identical plates share one mesh and the group adds its
//! own; the group element's bounds carry the placed stack; the framed reply the viewport reads
//! (`--with-grids true`) carries the same bundle; and a committed group whose fastener
//! definition is missing refuses the export instead of losing its bolts from the picture.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::text;

use beamcraft_engine::{ElementKind, Limits, ViewSnapshot};
use beamcraft_json::{Json, Limits as JsonLimits};
use beamcraft_model::digest::Sha256;
use beamcraft_model::id::ElementId;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-fasteners.usda"
);
const GROUP: &str = "019a7bf0-0000-7000-8000-000000710001";
const PLATE_LOW: &str = "019a7bf0-0000-7000-8000-0000007100f1";
const PLATE_HIGH: &str = "019a7bf0-0000-7000-8000-0000007100f2";

fn root(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("v03-bolt-groups-view")
        .join(format!("{name}-{}", crate::support::boot_nonce()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("store")).unwrap();
    root
}

fn run(args: &[&str]) -> (i32, Vec<u8>) {
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .output()
        .unwrap();
    (output.status.code().unwrap(), output.stdout)
}

fn json_of(bytes: &[u8]) -> Json {
    beamcraft_json::decode(bytes, &JsonLimits::DEFAULT).unwrap()
}

fn ok(args: &[&str]) -> Json {
    let (code, body) = run(args);
    assert_eq!(code, 0, "{args:?}: {}", String::from_utf8_lossy(&body));
    json_of(&body)
}

/// Two stacked plates (1000 x 500 x 10 at z = 0 and z = 10) and one washered group of two
/// SYN-BOLT-12x50 fasteners: the row runs along `x` at `y = 250` from `offset_start` 100 with
/// spacing 150, so the bearing points are `(100, 250, 0)` and `(250, 250, 0)`. The grip is 20,
/// so `auto` resolves the 40 length: the stack reaches `z = 53` above the plates' `z = 20`.
fn store_with_group(name: &str) -> (PathBuf, String) {
    let root = root(name);
    let store = root.join("store");
    let store = store.to_str().unwrap().to_owned();
    for (id, translation) in [(PLATE_LOW, None), (PLATE_HIGH, Some("0,0,10"))] {
        let mut args = vec![
            "create-plate",
            "--store",
            &store,
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--new-id",
            id,
        ];
        if let Some(at) = translation {
            args.extend(["--translation", at]);
        }
        ok(&args);
    }
    ok(&[
        "create-bolt-group",
        "--store",
        &store,
        "--name",
        "G1",
        "--new-id",
        GROUP,
        "--parts",
        &format!("{PLATE_LOW},{PLATE_HIGH}"),
        "--fastener-library",
        SYNTHETIC,
        "--designation",
        "SYN-BOLT-12x50",
        "--point1",
        "0,250,0",
        "--point2",
        "600,250,0",
        "--plane-normal",
        "0,0,1",
        "--offset-start",
        "100",
        "--spacings",
        "150",
    ]);
    (root, store)
}

#[test]
fn a_committed_group_exports_its_fasteners_into_the_bundle() {
    let (root, store) = store_with_group("bundle");
    let file = root.join("view.bundle");
    let file = file.to_str().unwrap();
    let summary = ok(&["export-view", "--store", &store, "--out", file]);
    assert_eq!(text(&summary, "outcome"), "view_exported");
    // The two plates keep one shared mesh; the group adds its own.
    assert_eq!(text(&summary, "elements"), "3");
    assert_eq!(text(&summary, "meshes"), "2");

    let bundle = fs::read(file).unwrap();
    let snapshot = ViewSnapshot::from_bundle(&bundle, &Limits::default()).unwrap();
    let id = ElementId::parse(GROUP).unwrap();
    let group = snapshot.element(&id).unwrap();
    assert_eq!(group.kind, ElementKind::Connection);
    assert_eq!(group.placement_mm, [0.0; 3]);
    assert_eq!(group.orientation_xyzw, [0.0, 0.0, 0.0, 1.0]);
    // The placed stack, by hand: washer −2..0 and head −8.5..−2 outside the joint, shank
    // −8.5..41.5 through it, nut 41.5..50.5, washer nut 50.5..52.5; the washer's 22 mm outer
    // diameter is the widest part.
    let bounds = snapshot.element_bounds(&id).unwrap();
    let expected = [[89.0, 239.0, -8.5], [261.0, 261.0, 52.5]];
    for (got, want) in bounds.min.iter().zip(expected[0]) {
        assert!((got - want).abs() <= 1e-6, "min {got} != {want}");
    }
    for (got, want) in bounds.max.iter().zip(expected[1]) {
        assert!((got - want).abs() <= 1e-6, "max {got} != {want}");
    }
    // The plates keep their own bounds; the group mesh is distinct from theirs.
    for (element, z) in [
        (ElementId::parse(PLATE_LOW).unwrap(), 10.0),
        (ElementId::parse(PLATE_HIGH).unwrap(), 20.0),
    ] {
        let bounds = snapshot.element_bounds(&element).unwrap();
        assert_eq!(bounds.min, [0.0, 0.0, z - 10.0]);
        assert_eq!(bounds.max, [1000.0, 500.0, z]);
    }
    assert_eq!(snapshot.meshes().len(), 2);
    let plate = snapshot
        .element(&ElementId::parse(PLATE_LOW).unwrap())
        .unwrap();
    let bolt_mesh = snapshot.mesh(&group.full.mesh_digest).unwrap();
    let plate_mesh = snapshot.mesh(&plate.full.mesh_digest).unwrap();
    assert_ne!(group.full.mesh_digest, plate.full.mesh_digest);
    assert!(
        bolt_mesh.vertex_count() > 100,
        "a two-fastener merged mesh, got {}",
        bolt_mesh.vertex_count()
    );
    // The definition's `hole_mode = generate` also cut the plates: each carries the group's
    // two holes, so the plate mesh is no longer the bare box.
    assert!(plate_mesh.vertex_count() > 8);
    // The exported snapshot is the inspected head's snapshot.
    let inspected = ok(&["inspect-elements", "--store", &store]);
    let token = text(inspected.get("snapshot").unwrap(), "token");
    assert_eq!(snapshot.token().as_str(), token);
    assert_eq!(text(&summary, "token"), token);

    // The framed reply the viewport reads carries the same bundle after the grid section.
    let (code, framed) = run(&[
        "export-view",
        "--store",
        &store,
        "--out",
        "-",
        "--with-grids",
        "true",
    ]);
    assert_eq!(code, 0);
    assert_eq!(&framed[..8], b"BCVXFRM2");
    let mut at = 8;
    for _ in 0..3 {
        let len = u32::from_le_bytes(framed[at..at + 4].try_into().unwrap()) as usize;
        at += 4 + len;
    }
    assert_eq!(
        &framed[at..],
        &bundle[..],
        "the framed bundle is the export"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_group_whose_fastener_definition_is_missing_refuses_the_export() {
    let (root, store) = store_with_group("damaged");
    // The authored layer of the committed head, mutated in place: the group's fastener digest
    // is replaced with a well-formed digest of nothing, so the record is incomplete and the
    // export must refuse it rather than draw a group without its fasteners.
    let inspected = ok(&["inspect-elements", "--store", &store]);
    let layer = inspected
        .get("revision")
        .unwrap()
        .get("manifest")
        .unwrap()
        .get("layers")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .find(|l| l.get("name").unwrap().string().unwrap() == "authored.usda")
        .unwrap();
    let digest = text(layer, "digest");
    let authored = fs::read(
        Path::new(&store)
            .join("blobs")
            .join(&digest[..2])
            .join(&digest),
    )
    .unwrap();
    let marker = "steel:bolt:fastener = \"";
    let start = authored
        .windows(marker.len())
        .position(|w| w == marker.as_bytes())
        .unwrap()
        + marker.len();
    let forged = "ab".repeat(32);
    let mut damaged = authored.clone();
    damaged[start..start + 64].copy_from_slice(forged.as_bytes());

    // The mutated layer commits as a first revision of a fresh store (commit-revision checks
    // digests, not USD content), exactly the hostile-layer path.
    let damaged_file = root.join("root.usda");
    fs::write(&damaged_file, &damaged).unwrap();
    let derived_file = root.join("derived.usda");
    fs::write(
        &derived_file,
        b"#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\n",
    )
    .unwrap();
    let fresh = root.join("fresh");
    fs::create_dir(&fresh).unwrap();
    let fresh = fresh.to_str().unwrap().to_owned();
    let put = |file: &str| {
        ok(&["put-blob", "--store", &fresh, "--file", file]);
    };
    put(damaged_file.to_str().unwrap());
    put(derived_file.to_str().unwrap());
    let schema = beamcraft_layer_engine::authored::schema_identity()
        .unwrap()
        .digest
        .to_string();
    let layers = format!(
        "authored:root.usda:{},derived:derived.usda:{}",
        Sha256::digest(&damaged).unwrap(),
        Sha256::digest(b"#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\n").unwrap()
    );
    ok(&[
        "commit-revision",
        "--store",
        &fresh,
        "--project-id",
        "0199e7a0-0000-7000-8000-000000000001",
        "--workspace-id",
        "0199e7a0-0000-7000-8000-000000000002",
        "--revision-id",
        "0199e7a0-0000-7000-8000-0000000000a1",
        "--op-id",
        "0199e7a0-0000-7000-8000-0000000000b1",
        "--generation",
        "1",
        "--base",
        "none",
        "--schema",
        &schema,
        "--root-layer",
        "root.usda",
        "--layers",
        &layers,
    ]);
    let (code, refused) = run(&["export-view", "--store", &fresh, "--out", "-"]);
    assert_eq!(code, 2, "an incomplete group refuses the export");
    let refused = json_of(&refused);
    assert_eq!(text(&refused, "outcome"), "rejected");
    assert_ne!(text(&refused, "code"), "view_exported");
    let _ = fs::remove_dir_all(&root);
}
