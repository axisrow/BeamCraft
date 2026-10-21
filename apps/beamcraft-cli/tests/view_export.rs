// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0.

//! V03 `view-export`: the verified head's view snapshot as one engine bundle. The bundle rebuilds
//! the identical snapshot (same token as `plate-inspect`), shares one mesh between copies, never
//! overwrites a file, draws no body for an assembly and is refused for a store without a revision.
//! With `--with-definitions true` and `--with-element ID` the same inspection also yields the
//! `definitions` and `plate-inspect --element ID` replies, framed before the unchanged bundle
//! (issue 26: one process per viewport refresh).

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
    // A framed request on an empty store is the same plain rejection, not a frame.
    let (code, body) = run(&[
        "view-export",
        "--store",
        store,
        "--out",
        "-",
        "--with-definitions",
        "true",
        "--with-element",
        PLATE,
    ]);
    assert_eq!(code, 2);
    assert_eq!(text(&json_of(&body), "code"), "NOT_FOUND");
    let file = root.join("framed.bin");
    let file = file.to_str().unwrap();
    for (args, expected) in [
        (
            vec![
                "view-export",
                "--store",
                store,
                "--out",
                file,
                "--with-definitions",
                "true",
            ],
            "INVALID_ARGUMENT",
        ),
        (
            vec![
                "view-export",
                "--store",
                store,
                "--out",
                file,
                "--with-element",
                PLATE,
            ],
            "INVALID_ARGUMENT",
        ),
        (
            vec![
                "view-export",
                "--store",
                store,
                "--out",
                "-",
                "--with-definitions",
                "yes",
            ],
            "INVALID_ARGUMENT",
        ),
        (
            vec![
                "view-export",
                "--store",
                store,
                "--out",
                "-",
                "--with-element",
                "not-an-id",
            ],
            "INVALID_ID",
        ),
    ] {
        let (code, body) = run(&args);
        assert_eq!(code, 2, "{args:?}");
        assert_eq!(text(&json_of(&body), "code"), expected, "{args:?}");
    }
    assert!(
        !Path::new(file).exists(),
        "a refused export creates no file"
    );
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
    ] {
        let (code, body) = run(&args);
        assert_eq!(code, 2, "{args:?}");
        assert_eq!(text(&json_of(&body), "code"), "INVALID_ARGUMENT");
    }
    let _ = fs::remove_dir_all(&root);
}

const PLATE: &str = "019a7bf0-0000-7000-8000-000000000020";

/// Splits a framed reply into the definitions reply, the element reply and the bundle.
fn sections(framed: &[u8]) -> (&[u8], &[u8], &[u8]) {
    assert_eq!(&framed[..8], b"BCVXFRM1");
    let mut rest = &framed[8..];
    let mut next = || {
        let length = u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize;
        let (reply, tail) = rest[4..].split_at(length);
        rest = tail;
        reply
    };
    let definitions = next();
    let element = next();
    (definitions, element, rest)
}

#[test]
fn the_definitions_and_one_element_ride_in_the_same_reply_as_the_bundle() {
    let root = root("framed");
    let store = root.join("store");
    let store = store.to_str().unwrap();
    let (code, defined) = run(&["define-material", "--store", store, "--density", "7850"]);
    assert_eq!(code, 0);
    let material = text(json_of(&defined).get("definition").unwrap(), "digest");
    for (id, offset) in [
        (PLATE, "0,0,0"),
        ("019a7bf0-0000-7000-8000-000000000021", "0,2000,0"),
    ] {
        let (code, _) = run(&[
            "create-plate",
            "--store",
            store,
            "--width",
            "800",
            "--length",
            "400",
            "--thickness",
            "12",
            "--material-digest",
            &material,
            "--translation",
            offset,
            "--new-id",
            id,
        ]);
        assert_eq!(code, 0);
    }
    let (code, plain) = run(&["view-export", "--store", store, "--out", "-"]);
    assert_eq!(code, 0);
    let (code, listing) = run(&["definitions", "--store", store]);
    assert_eq!(code, 0);
    let (code, inspected) = run(&["plate-inspect", "--store", store, "--element", PLATE]);
    assert_eq!(code, 0);
    let (code, framed) = run(&[
        "view-export",
        "--store",
        store,
        "--out",
        "-",
        "--with-definitions",
        "true",
        "--with-element",
        PLATE,
    ]);
    assert_eq!(code, 0);
    let (definitions, element, bundle) = sections(&framed);
    // Byte for byte what the separate commands print.
    assert_eq!(definitions, listing.as_slice());
    assert_eq!(element, inspected.as_slice());
    assert_eq!(bundle, plain.as_slice());
    // And every part belongs to the exported revision.
    let snapshot = ViewSnapshot::from_bundle(bundle, &Limits::default()).unwrap();
    for reply in [definitions, element] {
        let revision = text(json_of(reply).get("revision").unwrap(), "manifest_digest");
        assert_eq!(revision, snapshot.pin().revision().to_string());
    }
    let listed = json_of(definitions);
    let first = &listed.get("definitions").unwrap().array().unwrap()[0];
    assert_eq!(text(first, "digest"), material);
    // Each option alone frames only its reply; a length of zero is a reply not asked for.
    let (code, only) = run(&[
        "view-export",
        "--store",
        store,
        "--out",
        "-",
        "--with-definitions",
        "true",
    ]);
    assert_eq!(code, 0);
    assert_eq!(
        sections(&only),
        (listing.as_slice(), &b""[..], plain.as_slice())
    );
    let (code, only) = run(&[
        "view-export",
        "--store",
        store,
        "--out",
        "-",
        "--with-element",
        PLATE,
    ]);
    assert_eq!(code, 0);
    assert_eq!(
        sections(&only),
        (&b""[..], inspected.as_slice(), plain.as_slice())
    );
    // `false` is the plain bundle.
    let (code, unframed) = run(&[
        "view-export",
        "--store",
        store,
        "--out",
        "-",
        "--with-definitions",
        "false",
    ]);
    assert_eq!(code, 0);
    assert_eq!(unframed, plain);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn plate_inspect_lists_only_the_element_it_is_asked_for() {
    let root = root("element");
    let store = root.join("store");
    let store = store.to_str().unwrap();
    let other = "019a7bf0-0000-7000-8000-000000000021";
    for (id, offset) in [(PLATE, "0,0,0"), (other, "0,2000,0")] {
        let (code, _) = run(&[
            "create-plate",
            "--store",
            store,
            "--width",
            "800",
            "--length",
            "400",
            "--thickness",
            "12",
            "--translation",
            offset,
            "--new-id",
            id,
        ]);
        assert_eq!(code, 0);
    }
    let (code, member) = run(&[
        "define-profile",
        "--store",
        store,
        "--width",
        "200",
        "--height",
        "300",
    ]);
    assert_eq!(code, 0);
    let profile = text(json_of(&member).get("definition").unwrap(), "digest");
    let beam = "019a7bf0-0000-7000-8000-000000000022";
    let (code, _) = run(&[
        "create-member",
        "--store",
        store,
        "--start",
        "0,5000,0",
        "--end",
        "3000,5000,0",
        "--profile-digest",
        &profile,
        "--new-id",
        beam,
    ]);
    assert_eq!(code, 0);
    let (code, full) = run(&["plate-inspect", "--store", store]);
    assert_eq!(code, 0);
    let full = json_of(&full);
    let listed = |reply: &Json, key: &str| -> Vec<Json> {
        reply.get(key).unwrap().array().unwrap().to_vec()
    };
    for id in [PLATE, beam] {
        let (code, one) = run(&["plate-inspect", "--store", store, "--element", id]);
        assert_eq!(code, 0);
        let one = json_of(&one);
        assert_eq!(text(&one, "element_filter"), id);
        // The element exactly as the full report lists it, and nothing else.
        let entries: Vec<Json> = ["plates", "members"]
            .iter()
            .flat_map(|key| listed(&one, key))
            .collect();
        let expected: Vec<Json> = ["plates", "members"]
            .iter()
            .flat_map(|key| listed(&full, key))
            .filter(|e| text(e, "element_id") == id)
            .collect();
        assert_eq!(entries, expected);
        let snapshot = one.get("snapshot").unwrap();
        let elements = listed(snapshot, "elements");
        assert_eq!(elements.len(), 1);
        assert_eq!(text(&elements[0], "id"), id);
        let meshes = listed(snapshot, "meshes");
        assert_eq!(meshes.len(), 1);
        assert_eq!(
            text(&meshes[0], "digest"),
            text(&elements[0], "mesh_digest")
        );
        // The counts and the revision still describe the whole head.
        for key in ["plate_count", "member_count", "revision"] {
            assert_eq!(one.get(key).unwrap(), full.get(key).unwrap(), "{key}");
        }
    }
    // An id the head does not hold lists nothing; a malformed one reads nothing.
    let (code, none) = run(&[
        "plate-inspect",
        "--store",
        store,
        "--element",
        "019a7bf0-0000-7000-8000-0000000000ff",
    ]);
    assert_eq!(code, 0);
    let none = json_of(&none);
    assert!(listed(&none, "plates").is_empty() && listed(&none, "members").is_empty());
    let (code, bad) = run(&["plate-inspect", "--store", store, "--element", "x"]);
    assert_eq!(code, 2);
    assert_eq!(text(&json_of(&bad), "code"), "INVALID_ID");
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
