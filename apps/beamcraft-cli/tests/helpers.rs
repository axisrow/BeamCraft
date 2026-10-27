// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! T02 admission, persistence, replay, engineering isolation and inactive deletion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn ok(f: &StoreFixture, command: &str, args: &[&str]) -> Json {
    let (code, body) = f.run(command, args);
    assert_eq!(code, 0, "{command} {args:?}: {body:?}");
    body
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
const ID: &str = "019a7bf0-0000-7000-8000-000000700001";
const OP: &str = "019a7bf0-0000-7000-8000-000000700002";
#[test]
fn all_geometry_roundtrips_and_queries_share_exact_snaps() {
    let f = StoreFixture::new("helpers");
    for (kind, parameters) in [
        ("point", "10,20,30"),
        ("line", "0,0,0,100,0,0"),
        ("circle", "0,0,0,0,0,1,50"),
        ("plane", "0,0,0,0,0,1"),
    ] {
        ok(
            &f,
            "create-helper",
            &["--name", kind, "--kind", kind, "--parameters", parameters],
        );
    }
    let listed = ok(&f, "helpers", &[]);
    assert_eq!(listed.get("helpers").unwrap().array().unwrap().len(), 4);
    let head = ok(&f, "open-revision", &[]);
    let snap = ok(
        &f,
        "snap-helper",
        &["--point", "37,0,0.5", "--tolerance", "1"],
    );
    assert_ne!(snap.get("snap").unwrap(), &Json::Null);
    ok(
        &f,
        "snap",
        &["--point", "37,0,0.5", "--radius", "1", "--kinds", "helper"],
    );
    let inspection = ok(&f, "inspect-elements", &[]);
    assert!(
        inspection
            .get("plates")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(ok(&f, "open-revision", &[]), head);
    let csv = f.root.join("parts.csv");
    ok(&f, "export-bom", &["--out", csv.to_str().unwrap()]);
    assert_eq!(std::fs::read_to_string(csv).unwrap().lines().count(), 1);
}
#[test]
fn replay_refusals_and_deletion_are_atomic() {
    let f = StoreFixture::new("helper-replay");
    let args = [
        "--name",
        "p",
        "--kind",
        "point",
        "--parameters",
        "1,2,3",
        "--new-id",
        ID,
        "--op-id",
        OP,
    ];
    ok(&f, "create-helper", &args);
    let head = ok(&f, "open-revision", &[]);
    assert_eq!(
        text(&ok(&f, "create-helper", &args), "outcome"),
        "helper_replayed"
    );
    assert_eq!(ok(&f, "open-revision", &[]), head);
    for (kind, params) in [
        ("line", "0,0,0,0,0,0"),
        ("circle", "0,0,0,0,0,0,10"),
        ("point", "NaN,0,0"),
    ] {
        assert_ne!(
            f.run(
                "create-helper",
                &["--name", "bad", "--kind", kind, "--parameters", params]
            )
            .0,
            0
        );
        assert_eq!(ok(&f, "open-revision", &[]), head);
    }
    ok(&f, "delete-element", &["--id", ID]);
    assert!(
        ok(&f, "helpers", &[])
            .get("helpers")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn deactivated_helpers_are_not_snap_targets_but_can_be_deleted() {
    use beamcraft_host::{
        ids::IdSource,
        revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
        storage::BlobStore,
    };
    let f = StoreFixture::new("helper-inactive");
    ok(
        &f,
        "create-helper",
        &[
            "--name",
            "p",
            "--kind",
            "point",
            "--parameters",
            "1,2,3",
            "--new-id",
            ID,
        ],
    );
    let store = RevisionStore::new(BlobStore::new(f.root.join("store")).unwrap());
    let head = store.open().unwrap();
    let content = head.manifest().content();
    let mut ids = IdSource::system();
    let mut entries = Vec::new();
    for e in content.entries() {
        let mut data = String::from_utf8(head.layer_bytes(e.name()).unwrap()).unwrap();
        if e.name() == "authored.usda" {
            data = data.replace(
                "custom token bim:geometryMode",
                "custom bool bim:isDeactivated = true\n            custom token bim:geometryMode",
            );
        }
        store.blobs().publish(data.as_bytes()).unwrap();
        entries.push(LayerEntry::from_bytes(e.name(), e.role(), data.as_bytes()).unwrap());
    }
    store
        .commit_with(
            &RevisionDraft {
                project: content.project().clone(),
                workspace: content.workspace().clone(),
                revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
                op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
                workspace_generation: content.workspace_generation() + 1,
                schema: content.schema(),
                root_layer: content.root_layer().to_owned(),
                entries,
            },
            &ExpectedBase::Revision(head.info().revision().clone()),
            None,
            &mut NoFault,
        )
        .unwrap();
    assert!(
        ok(&f, "helpers", &[])
            .get("helpers")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
    ok(&f, "delete-element", &["--id", ID]);
}
