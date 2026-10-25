// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and stores, MPL-2.0.

//! C03.7: all real CLI publication paths respect a pinned accepted head, while a personal draft
//! remains editable. Releasing the guard allows the exact refused operation to be retried.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_host::lease::{PublicationLease, StoreLease};
use beamcraft_host::ledger::Outcome;
use beamcraft_host::revision::{RevisionStore, Uuid7};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::Limits;
use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn a_pinned_publication_refuses_cli_writes_keeps_local_edits_and_retries_after_release() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("publication-guard-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let published = root.join("published");
    let draft = root.join("draft");
    fs::create_dir(&published).unwrap();
    let pub_path = published.to_str().unwrap();
    let draft_path = draft.to_str().unwrap();
    let id = "019a7bf0-0000-7000-8000-000000000001";
    let op = "019a7bf0-0000-7000-8000-000000000003";
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            beamcraft_json::decode(&out.stdout, &Limits::DEFAULT).unwrap(),
        )
    };
    let ok = |args: &[&str]| {
        let (code, json) = run(args);
        assert_eq!(code, 0, "{args:?}: {json:?}");
        json
    };
    ok(&[
        "create-plate",
        "--store",
        pub_path,
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--new-id",
        id,
    ]);
    ok(&[
        "create-workspace",
        "--store",
        pub_path,
        "--workspace-dir",
        draft_path,
        "--user-id",
        "019a7bf0-0000-7000-8000-000000000099",
    ]);
    let published_store = RevisionStore::new(BlobStore::new(&published).unwrap());
    let draft_store = RevisionStore::new(BlobStore::new(&draft).unwrap());
    let serving = StoreLease::shared(published_store.blobs()).unwrap();
    let guard = PublicationLease::exclusive(published_store.blobs()).unwrap();
    let accepted = published_store.head().unwrap().unwrap();
    let old_draft = draft_store.head().unwrap().unwrap();
    let ledger = fs::read(published.join("ledger/outcomes.log")).unwrap();
    let create = [
        "create-plate",
        "--store",
        pub_path,
        "--width",
        "100",
        "--length",
        "50",
        "--thickness",
        "10",
        "--name",
        "New",
        "--new-id",
        "019a7bf0-0000-7000-8000-000000000002",
        "--op-id",
        op,
        "--revision-id",
        "019a7bf0-0000-7000-8000-000000000004",
    ];
    let (code, json) = run(&create);
    assert_eq!(code, 3, "{json:?}");
    assert_eq!(
        json.get("code").unwrap().string().unwrap(),
        "BACKEND_UNAVAILABLE"
    );
    assert_eq!(published_store.head().unwrap().unwrap(), accepted);
    assert_eq!(
        fs::read(published.join("ledger/outcomes.log")).unwrap(),
        ledger
    );
    assert_eq!(
        published_store
            .query_outcome(&Uuid7::parse(op).unwrap())
            .unwrap(),
        Outcome::NotFound
    );
    ok(&[
        "update-element",
        "--store",
        draft_path,
        "--id",
        id,
        "--name",
        "Mine",
    ]);
    let edited = draft_store.head().unwrap().unwrap();
    assert_ne!(edited, old_draft);
    let publish = [
        "publish-workspace",
        "--store",
        pub_path,
        "--workspace-dir",
        draft_path,
    ];
    let (code, json) = run(&publish);
    assert_eq!(code, 3, "{json:?}");
    assert_eq!(
        json.get("code").unwrap().string().unwrap(),
        "BACKEND_UNAVAILABLE"
    );
    assert_eq!(published_store.head().unwrap().unwrap(), accepted);
    assert_eq!(draft_store.head().unwrap().unwrap(), edited);
    drop(guard);
    ok(&publish);
    ok(&create);
    let inspected = ok(&["inspect-elements", "--store", pub_path]);
    let mut names: Vec<_> = inspected
        .get("plates")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|plate| plate.get("name").unwrap().string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["Mine", "New"]);
    assert_eq!(draft_store.head().unwrap().unwrap(), edited);
    drop(serving);
    fs::remove_dir_all(root).unwrap();
}
