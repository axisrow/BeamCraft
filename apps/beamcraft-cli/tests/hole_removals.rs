// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! A04 explicit hole retirement, immutable retries, cross-kind UUIDs and shared undo.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use support::StoreFixture;
const FIRST: &str = "019a7bf0-0000-7000-8000-000000ae0001";
const SECOND: &str = "019a7bf0-0000-7000-8000-000000ae0002";
const THIRD: &str = "019a7bf0-0000-7000-8000-000000ae0003";
const REMOVE: &str = "019a7bf0-0000-7000-8000-000000ae0004";
fn ok(f: &StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
fn volume(p: &Json) -> f64 {
    p.get("quantities")
        .unwrap()
        .get("volume")
        .unwrap()
        .get("mm3")
        .unwrap()
        .float()
        .unwrap()
}
fn plate(f: &StoreFixture) -> String {
    let p = ok(
        f,
        "create-plate",
        &["--width", "100", "--length", "100", "--thickness", "10"],
    );
    text(p.get("plate").unwrap(), "element_id")
}
fn add(f: &StoreFixture, id: &str, record: &str, rect: &str) -> Json {
    ok(
        f,
        "add-hole",
        &["--id", id, "--operation-id", record, "--hole-rect", rect],
    )
}
fn remove(f: &StoreFixture, id: &str, record: &str) -> (i32, Json) {
    f.run(
        "remove-hole",
        &["--id", id, "--operation-id", record, "--op-id", REMOVE],
    )
}
fn records(j: &Json) -> &[Json] {
    j.get("element")
        .unwrap()
        .get("hole_records")
        .unwrap()
        .array()
        .unwrap()
}
#[test]
fn deleting_a_record_keeps_surviving_ordinals_and_cannot_reuse_its_uuid_or_cap() {
    let f = StoreFixture::new("retired-hole-records");
    let id = plate(&f);
    let first = add(&f, &id, FIRST, "10,10,10,10");
    add(&f, &id, SECOND, "40,10,10,10");
    let (code, removed) = remove(&f, &id, FIRST);
    assert_eq!(code, 0, "{removed:?}");
    assert_eq!(volume(removed.get("element").unwrap()), 99000.0);
    assert_eq!(records(&removed).len(), 1);
    assert_eq!(
        records(&removed)[0]
            .get("ordinal")
            .unwrap()
            .unsigned()
            .unwrap(),
        1
    );
    let next = add(&f, &id, THIRD, "10,10,10,10");
    assert_eq!(
        records(&next)[1]
            .get("ordinal")
            .unwrap()
            .unsigned()
            .unwrap(),
        2
    );
    let head = ok(&f, "inspect-elements", &[]);
    let (code, replay) = remove(&f, &id, FIRST);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(replay.get("revision"), removed.get("revision"));
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    let replay = add(&f, &id, FIRST, "10,10,10,10");
    assert_eq!(replay.get("revision"), first.get("revision"));
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    let (code, bad) = f.run(
        "cut-by-plane",
        &[
            "--id",
            &id,
            "--operation-id",
            FIRST,
            "--point",
            "80,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    assert_ne!(code, 0, "{bad:?}");
    assert_eq!(text(&bad, "code"), "DUPLICATE_ID");
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    ok(&f, "delete-element", &["--id", &id]);
    let head = ok(&f, "inspect-elements", &[]);
    assert_eq!(remove(&f, &id, FIRST).0, 0);
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
}
#[test]
fn copied_record_ids_are_retired_even_when_they_were_not_logical_creation_ids() {
    let f = StoreFixture::new("copied-hole-retirement");
    let id = plate(&f);
    add(&f, &id, FIRST, "10,10,10,10");
    let c = ok(&f, "copy-element", &["--id", &id, "--offset", "200,0,0"]);
    let copied = text(c.get("element").unwrap(), "element_id");
    let record = text(&records(&c)[0], "operation_id");
    assert_ne!(record, FIRST);
    let (code, r) = remove(&f, &copied, &record);
    assert_eq!(code, 0, "{r:?}");
    let head = ok(&f, "inspect-elements", &[]);
    let (code, bad) = f.run(
        "add-hole",
        &[
            "--id",
            &copied,
            "--operation-id",
            &record,
            "--hole-rect",
            "20,20,10,10",
        ],
    );
    assert_ne!(code, 0, "{bad:?}");
    assert_eq!(text(&bad, "code"), "DUPLICATE_ID");
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
}
#[test]
fn a_linked_removal_is_one_undo_unit_and_retirement_survives_reopen() {
    let f = StoreFixture::new("linked-hole-retirement");
    let id = plate(&f);
    add(&f, &id, FIRST, "10,10,10,10");
    let c = ok(&f, "copy-instance", &["--id", &id, "--offset", "200,0,0"]);
    let instance = text(c.get("element").unwrap(), "element_id");
    let dir = f.root.join("draft");
    ok(
        &f,
        "create-workspace",
        &["--workspace-dir", dir.to_str().unwrap()],
    );
    let run = |c: &str, a: &[&str]| {
        let (code, j) = if matches!(c, "undo-workspace" | "redo-workspace") {
            let o = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
                .arg(c)
                .arg("--workspace-dir")
                .arg(&dir)
                .args(a)
                .output()
                .unwrap();
            (
                o.status.code().unwrap(),
                beamcraft_json::decode(&o.stdout, &beamcraft_json::Limits::DEFAULT).unwrap(),
            )
        } else {
            StoreFixture::run_in(dir.to_str().unwrap(), c, a)
        };
        assert_eq!(code, 0, "{c}: {j:?}");
        j
    };
    let vols = |j: &Json| {
        j.get("plates")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(volume)
            .collect::<Vec<_>>()
    };
    let before = run("inspect-elements", &[]);
    run(
        "remove-hole",
        &[
            "--id",
            &instance,
            "--operation-id",
            FIRST,
            "--op-id",
            REMOVE,
        ],
    );
    assert_eq!(vols(&run("inspect-elements", &[])), vec![100000.0; 2]);
    run("undo-workspace", &[]);
    assert_eq!(vols(&run("inspect-elements", &[])), vols(&before));
    run("redo-workspace", &[]);
    assert_eq!(vols(&run("inspect-elements", &[])), vec![100000.0; 2]);
    let next = run(
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            SECOND,
            "--hole-rect",
            "10,10,10,10",
        ],
    );
    assert_eq!(
        records(&next)[0]
            .get("ordinal")
            .unwrap()
            .unsigned()
            .unwrap(),
        1
    );
    assert_eq!(vols(&run("inspect-elements", &[])), vec![99000.0; 2]);
}
#[test]
fn unknown_records_and_incompatible_flags_refuse_without_a_revision() {
    let f = StoreFixture::new("remove-hole-refusals");
    let id = plate(&f);
    let head = ok(&f, "inspect-elements", &[]);
    let (code, j) = remove(&f, &id, FIRST);
    assert_ne!(code, 0, "{j:?}");
    assert_eq!(text(&j, "code"), "NOT_FOUND");
    assert_ne!(
        f.run(
            "remove-hole",
            &[
                "--id",
                &id,
                "--operation-id",
                FIRST,
                "--hole-rect",
                "1,1,2,2"
            ]
        )
        .0,
        0
    );
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
}
