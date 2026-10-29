// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! A03 in-place cut updates: immutable retry evidence, atomic refusal and workspace history.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use support::StoreFixture;
fn ok(f: &StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
fn update(f: &StoreFixture, id: &str, record: &str, point: &str, op: &str) -> (i32, Json) {
    f.run(
        "update-cut",
        &[
            "--id",
            id,
            "--operation-id",
            record,
            "--point",
            point,
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
            "--op-id",
            op,
        ],
    )
}
fn volume(j: &Json) -> f64 {
    j.get("element")
        .unwrap()
        .get("quantities")
        .unwrap()
        .get("volume")
        .unwrap()
        .get("mm3")
        .unwrap()
        .float()
        .unwrap()
}
const RECORD: &str = "019a7bf0-0000-7000-8000-000000ab0001";
const EDIT: &str = "019a7bf0-0000-7000-8000-000000ab0002";
const LATER: &str = "019a7bf0-0000-7000-8000-000000ab0003";
#[test]
fn updates_preserve_identity_and_retry_the_original_revision_after_later_edits() {
    let f = StoreFixture::new("cut-updates");
    let p = ok(
        &f,
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    let id = text(p.get("plate").unwrap(), "element_id");
    let before = ok(
        &f,
        "cut-by-plane",
        &[
            "--id",
            &id,
            "--operation-id",
            RECORD,
            "--point",
            "80,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    let (code, a) = update(&f, &id, RECORD, "60,0,0", EDIT);
    assert_eq!(code, 0, "{a:?}");
    assert_eq!(volume(&a), 30000.0);
    assert_eq!(a.get("record"), before.get("record"));
    let (code, b) = update(&f, &id, RECORD, "50,0,0", LATER);
    assert_eq!(code, 0, "{b:?}");
    assert_eq!(volume(&b), 25000.0);
    let head = ok(&f, "inspect-elements", &[]);
    let (code, replay) = update(&f, &id, RECORD, "60,0,0", EDIT);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(text(&replay, "outcome"), "cut_replayed");
    assert_eq!(replay.get("revision"), a.get("revision"));
    assert_eq!(volume(&replay), 30000.0);
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    assert_ne!(update(&f, &id, RECORD, "61,0,0", EDIT).0, 0);
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    for point in ["0,0,0", "nan,0,0", "10000000000,0,0"] {
        assert_ne!(
            update(
                &f,
                &id,
                RECORD,
                point,
                "019a7bf0-0000-7000-8000-000000ab0010"
            )
            .0,
            0
        );
        assert_eq!(ok(&f, "inspect-elements", &[]), head);
    }
    assert_ne!(
        update(
            &f,
            &id,
            "019a7bf0-0000-7000-8000-000000abffff",
            "40,0,0",
            "019a7bf0-0000-7000-8000-000000ab0011"
        )
        .0,
        0
    );
    ok(&f, "delete-element", &["--id", &id]);
    let deleted = ok(&f, "inspect-elements", &[]);
    assert_eq!(update(&f, &id, RECORD, "60,0,0", EDIT).0, 0);
    assert_eq!(ok(&f, "inspect-elements", &[]), deleted);
}
#[test]
fn a_shared_cut_update_is_one_workspace_undo_and_redo_unit() {
    let f = StoreFixture::new("shared-cut-update");
    let p = ok(
        &f,
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    let id = text(p.get("plate").unwrap(), "element_id");
    ok(
        &f,
        "cut-by-plane",
        &[
            "--id",
            &id,
            "--operation-id",
            RECORD,
            "--point",
            "80,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    let copy = ok(&f, "copy-instance", &["--id", &id, "--offset", "200,0,0"]);
    let _ = copy;
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
    let before = run("inspect-elements", &[]);
    run(
        "update-cut",
        &[
            "--id",
            &id,
            "--operation-id",
            RECORD,
            "--point",
            "60,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
            "--op-id",
            EDIT,
        ],
    );
    let after = run("inspect-elements", &[]);
    let values = |j: &Json| {
        j.get("plates")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|p| {
                p.get("quantities")
                    .unwrap()
                    .get("volume")
                    .unwrap()
                    .get("mm3")
                    .unwrap()
                    .float()
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(values(&after), vec![30000.0; 2]);
    run("undo-workspace", &[]);
    assert_eq!(values(&run("inspect-elements", &[])), values(&before));
    run("redo-workspace", &[]);
    assert_eq!(values(&run("inspect-elements", &[])), values(&after));
}

#[test]
fn updating_a_rotated_member_cut_preserves_other_records_and_placement() {
    let f = StoreFixture::new("member-cut-update");
    ok(
        &f,
        "define-profile",
        &[
            "--designation",
            "TestRect",
            "--width",
            "20",
            "--height",
            "10",
        ],
    );
    let m = ok(
        &f,
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "100,0,0",
            "--profile",
            "TestRect",
        ],
    );
    let id = text(m.get("member").unwrap(), "element_id");
    ok(
        &f,
        "update-element",
        &[
            "--id",
            &id,
            "--orientation-xyzw",
            "0,0,0.7071067811865476,0.7071067811865476",
            "--translation",
            "100,200,0",
        ],
    );
    ok(
        &f,
        "cut-by-plane",
        &[
            "--id",
            &id,
            "--operation-id",
            RECORD,
            "--point",
            "80,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    let second = ok(
        &f,
        "cut-by-plane",
        &[
            "--id",
            &id,
            "--operation-id",
            LATER,
            "--point",
            "10,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "positive",
        ],
    );
    let before = ok(&f, "inspect-elements", &[]);
    let edited = ok(
        &f,
        "update-cut",
        &[
            "--id",
            &id,
            "--operation-id",
            RECORD,
            "--point",
            "60,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
            "--op-id",
            EDIT,
        ],
    );
    assert_eq!(volume(&edited), 10000.0);
    let old = second.get("element").unwrap();
    let new = edited.get("element").unwrap();
    assert_eq!(old.get("placement"), new.get("placement"));
    assert_eq!(old.get("cut_count"), new.get("cut_count"));
    assert_ne!(before, ok(&f, "inspect-elements", &[]));
}

#[test]
fn original_cut_creation_retries_stay_immutable_after_record_update_removal_and_delete() {
    let f = StoreFixture::new("immutable-cut-create");
    let p = ok(
        &f,
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    let id = text(p.get("plate").unwrap(), "element_id");
    let create_op = "019a7bf0-0000-7000-8000-000000ab0101";
    let args = [
        "--id",
        &id,
        "--operation-id",
        RECORD,
        "--point",
        "80,0,0",
        "--normal",
        "1,0,0",
        "--keep",
        "negative",
        "--op-id",
        create_op,
    ];
    let first = ok(&f, "cut-by-plane", &args);
    assert_eq!(volume(&first), 40000.0);
    let (code, j) = update(&f, &id, RECORD, "60,0,0", EDIT);
    assert_eq!(code, 0, "{j:?}");
    let head = ok(&f, "inspect-elements", &[]);
    let replay = ok(&f, "cut-by-plane", &args);
    assert_eq!(replay.get("revision"), first.get("revision"));
    assert_eq!(volume(&replay), 40000.0);
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    ok(&f, "remove-cut", &["--id", &id, "--operation-id", RECORD]);
    let head = ok(&f, "inspect-elements", &[]);
    assert_eq!(
        ok(&f, "cut-by-plane", &args).get("revision"),
        first.get("revision")
    );
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    ok(&f, "delete-element", &["--id", &id]);
    let head = ok(&f, "inspect-elements", &[]);
    assert_eq!(
        ok(&f, "cut-by-plane", &args).get("revision"),
        first.get("revision")
    );
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
}
