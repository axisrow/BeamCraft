// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0.

//! V03 `view-export`: the verified head's view snapshot as one engine bundle. The bundle rebuilds
//! the identical snapshot (same token as `plate-inspect`), shares one mesh between copies, never
//! overwrites a file, draws no body for an assembly and is refused for a store without a revision.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_engine::{Limits, ViewSnapshot};
use beamcraft_protocol::{
    json::{self, Json},
    message,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("v03-export")
        .join(format!("{name}-{}", std::process::id()));
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
    json::decode(bytes, &message::read_limits()).unwrap()
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}

#[test]
fn the_bundle_is_the_engine_snapshot_of_the_head() {
    let root = root("head");
    let store = root.join("store");
    let store = store.to_str().unwrap();
    let plate = "019a7bf0-0000-7000-8000-000000000010";
    let created = run(&[
        "create-plate",
        "--store",
        store,
        "--width",
        "800",
        "--length",
        "400",
        "--thickness",
        "12",
        "--new-id",
        plate,
    ]);
    assert_eq!(created.0, 0);
    for offset in ["1000,0,0", "2000,0,0"] {
        let copied = run(&[
            "copy-element",
            "--store",
            store,
            "--id",
            plate,
            "--offset",
            offset,
        ]);
        assert_eq!(copied.0, 0);
    }
    let (code, bundle) = run(&["view-export", "--store", store, "--out", "-"]);
    assert_eq!(code, 0);
    let snapshot = ViewSnapshot::from_bundle(&bundle, &Limits::default()).unwrap();
    assert_eq!(snapshot.len(), 3);
    // Copies share one mesh blob: the instancing input of the viewport.
    assert_eq!(snapshot.meshes().len(), 1);
    let (code, inspected) = run(&["plate-inspect", "--store", store]);
    assert_eq!(code, 0);
    let inspected = json_of(&inspected);
    let token = text(inspected.get("snapshot").unwrap(), "token");
    assert_eq!(snapshot.token().as_str(), token);

    // Opt-in refresh transports the identical old bundle and the existing definitions page.
    let (code, refresh) = run(&[
        "view-export",
        "--store",
        store,
        "--out",
        "-",
        "--include-definitions",
        "true",
    ]);
    assert_eq!(code, 0);
    let (embedded, metadata) = beamcraft_protocol::refresh::decode(
        &refresh,
        beamcraft_engine::query::bundle::max_bundle_bytes(&Limits::default()),
    )
    .unwrap();
    assert_eq!(embedded, bundle);
    let (code, definitions) = run(&["definitions", "--store", store]);
    assert_eq!(code, 0);
    assert_eq!(json_of(metadata), json_of(&definitions));
    let file_refresh = root.join("refresh.bin");
    let (code, _) = run(&[
        "view-export",
        "--store",
        store,
        "--out",
        file_refresh.to_str().unwrap(),
        "--include-definitions",
        "true",
    ]);
    assert_eq!(code, 0);
    assert_eq!(fs::read(file_refresh).unwrap(), refresh);

    let file = root.join("view.bin");
    let file = file.to_str().unwrap();
    let (code, summary) = run(&["view-export", "--store", store, "--out", file]);
    assert_eq!(code, 0);
    let summary = json_of(&summary);
    assert_eq!(text(&summary, "outcome"), "view_exported");
    assert_eq!(text(&summary, "token"), token);
    assert_eq!(fs::read(file).unwrap(), bundle);
    // An existing file is never overwritten.
    let (code, refused) = run(&["view-export", "--store", store, "--out", file]);
    assert_eq!(code, 2);
    assert_eq!(text(&json_of(&refused), "code"), "INVALID_ARGUMENT");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_store_without_a_revision_and_bad_options_are_refused() {
    let root = root("empty");
    let store = root.join("store");
    let store = store.to_str().unwrap();
    let (code, body) = run(&["view-export", "--store", store, "--out", "-"]);
    assert_eq!(code, 2);
    assert_eq!(text(&json_of(&body), "code"), "NOT_FOUND");
    for args in [
        vec!["view-export", "--store", store],
        vec![
            "view-export",
            "--store",
            store,
            "--out",
            "-",
            "--extra",
            "x",
        ],
        vec!["view-export", "--out", "-"],
        vec![
            "view-export",
            "--store",
            store,
            "--out",
            "-",
            "--include-definitions",
            "false",
        ],
        vec![
            "view-export",
            "--store",
            store,
            "--out",
            "-",
            "--include-definitions",
            "true",
            "--include-definitions",
            "true",
        ],
    ] {
        let (code, body) = run(&args);
        assert_eq!(code, 2, "{args:?}");
        assert_eq!(text(&json_of(&body), "code"), "INVALID_ARGUMENT");
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_assembly_has_no_body_and_its_parts_are_drawn() {
    let root = root("assembly");
    let store = root.join("store");
    let store = store.to_str().unwrap();
    let plate = "019a7bf0-0000-7000-8000-000000000010";
    let assembly = "019a7bf0-0000-7000-8000-000000000030";
    for args in [
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "800",
            "--length",
            "400",
            "--thickness",
            "12",
            "--new-id",
            plate,
        ][..],
        &[
            "create-assembly",
            "--store",
            store,
            "--name",
            "Frame",
            "--new-id",
            assembly,
        ],
        &[
            "assembly-attach",
            "--store",
            store,
            "--id",
            plate,
            "--assembly-id",
            assembly,
        ],
    ] {
        assert_eq!(run(args).0, 0, "{args:?}");
    }
    let (code, bundle) = run(&["view-export", "--store", store, "--out", "-"]);
    assert_eq!(code, 0);
    let snapshot = ViewSnapshot::from_bundle(&bundle, &Limits::default()).unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot.meshes().len(), 1);
    let _ = fs::remove_dir_all(&root);
}
