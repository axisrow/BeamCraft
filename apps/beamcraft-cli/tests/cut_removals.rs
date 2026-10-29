// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Explicit cut retirement prevents semantic identity reuse; real process/undo regressions.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use support::StoreFixture;
const RECORD: &str = "019a7bf0-0000-7000-8000-000000ac0001";
const REMOVE: &str = "019a7bf0-0000-7000-8000-000000ac0002";
const NEXT: &str = "019a7bf0-0000-7000-8000-000000ac0003";
fn ok(f: &StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
fn amount(p: &Json) -> f64 {
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
    let j = ok(
        f,
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    text(j.get("plate").unwrap(), "element_id")
}
fn cut(f: &StoreFixture, id: &str, record: &str) -> Json {
    ok(
        f,
        "cut-by-plane",
        &[
            "--id",
            id,
            "--operation-id",
            record,
            "--point",
            "80,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    )
}
fn remove(f: &StoreFixture, id: &str) -> (i32, Json) {
    f.run(
        "remove-cut",
        &["--id", id, "--operation-id", RECORD, "--op-id", REMOVE],
    )
}
#[test]
fn removing_the_last_cut_never_reuses_its_ordinal_or_record_uuid() {
    let f = StoreFixture::new("retired-cuts");
    let id = plate(&f);
    let first = cut(&f, &id, RECORD);
    assert_eq!(
        first
            .get("record")
            .unwrap()
            .get("ordinal")
            .unwrap()
            .unsigned()
            .unwrap(),
        0
    );
    let (code, removed) = remove(&f, &id);
    assert_eq!(code, 0, "{removed:?}");
    assert_eq!(amount(removed.get("element").unwrap()), 50000.0);
    let again = cut(&f, &id, NEXT);
    assert_eq!(
        again
            .get("record")
            .unwrap()
            .get("ordinal")
            .unwrap()
            .unsigned()
            .unwrap(),
        1
    );
    let head = ok(&f, "inspect-elements", &[]);
    let (code, replay) = remove(&f, &id);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(replay.get("revision"), removed.get("revision"));
    assert_eq!(amount(replay.get("element").unwrap()), 50000.0);
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    let (code, bad) = f.run(
        "cut-by-plane",
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
        ],
    );
    assert_ne!(code, 0, "{bad:?}");
    assert_eq!(text(&bad, "code"), "DUPLICATE_ID");
    let (code, bad) = f.run(
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            RECORD,
            "--hole-round",
            "10,10,2",
        ],
    );
    assert_ne!(code, 0, "{bad:?}");
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    let other = plate(&f);
    let (code, bad) = remove(&f, &other);
    assert_ne!(code, 0, "{bad:?}");
    assert_eq!(text(&bad, "code"), "OP_ID_REUSE");
    ok(&f, "delete-element", &["--id", &id]);
    let head = ok(&f, "inspect-elements", &[]);
    assert_eq!(remove(&f, &id).0, 0);
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
}
#[test]
fn removal_through_a_linked_instance_is_one_undo_unit_and_preserves_the_group() {
    let f = StoreFixture::new("shared-cut-retirement");
    let id = plate(&f);
    cut(&f, &id, RECORD);
    let copied = ok(&f, "copy-instance", &["--id", &id, "--offset", "200,0,0"]);
    let instance = text(copied.get("element").unwrap(), "element_id");
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
        "remove-cut",
        &[
            "--id",
            &instance,
            "--operation-id",
            RECORD,
            "--op-id",
            REMOVE,
        ],
    );
    let after = run("inspect-elements", &[]);
    let volumes = |j: &Json| {
        j.get("plates")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(amount)
            .collect::<Vec<_>>()
    };
    assert_eq!(volumes(&after), vec![50000.0; 2]);
    run("undo-workspace", &[]);
    assert_eq!(volumes(&run("inspect-elements", &[])), volumes(&before));
    run("redo-workspace", &[]);
    assert_eq!(volumes(&run("inspect-elements", &[])), volumes(&after));
    let next = run(
        "cut-by-plane",
        &[
            "--id",
            &id,
            "--operation-id",
            NEXT,
            "--point",
            "60,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    assert_eq!(
        next.get("record")
            .unwrap()
            .get("ordinal")
            .unwrap()
            .unsigned()
            .unwrap(),
        1
    );
    assert_eq!(volumes(&run("inspect-elements", &[])), vec![30000.0; 2]);
}
#[test]
fn unknown_record_and_incompatible_options_refuse_without_writing() {
    let f = StoreFixture::new("cut-removal-refusals");
    let id = plate(&f);
    let before = ok(&f, "inspect-elements", &[]);
    let (code, j) = remove(&f, &id);
    assert_ne!(code, 0, "{j:?}");
    assert_eq!(text(&j, "code"), "NOT_FOUND");
    assert_eq!(ok(&f, "inspect-elements", &[]), before);
    let (code, j) = f.run(
        "remove-cut",
        &["--id", &id, "--operation-id", RECORD, "--point", "1,0,0"],
    );
    assert_ne!(code, 0, "{j:?}");
    assert_eq!(ok(&f, "inspect-elements", &[]), before);
}
