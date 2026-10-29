// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! A04 in-place hole edits preserve record identities, legacy holes and request evidence.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use support::StoreFixture;
const RECORD: &str = "019a7bf0-0000-7000-8000-000000ad0001";
const EDIT: &str = "019a7bf0-0000-7000-8000-000000ad0002";
const LATER: &str = "019a7bf0-0000-7000-8000-000000ad0003";
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
        &[
            "--width",
            "100",
            "--length",
            "100",
            "--thickness",
            "10",
            "--hole-rect",
            "70,70,10,10",
        ],
    );
    text(p.get("plate").unwrap(), "element_id")
}
fn edit(f: &StoreFixture, id: &str, rect: &str, op: &str) -> (i32, Json) {
    f.run(
        "update-hole",
        &[
            "--id",
            id,
            "--operation-id",
            RECORD,
            "--hole-rect",
            rect,
            "--op-id",
            op,
        ],
    )
}
#[test]
fn replacement_preserves_other_holes_and_replays_the_accepted_revision_after_later_edits() {
    let f = StoreFixture::new("hole-record-updates");
    let id = plate(&f);
    let a = ok(
        &f,
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            RECORD,
            "--hole-rect",
            "10,10,10,10",
        ],
    );
    assert_eq!(volume(a.get("element").unwrap()), 98000.0);
    let (code, e) = edit(&f, &id, "10,10,20,10", EDIT);
    assert_eq!(code, 0, "{e:?}");
    assert_eq!(volume(e.get("element").unwrap()), 97000.0);
    let (code, later) = edit(&f, &id, "10,10,25,10", LATER);
    assert_eq!(code, 0, "{later:?}");
    assert_eq!(volume(later.get("element").unwrap()), 96500.0);
    let head = ok(&f, "inspect-elements", &[]);
    let add_replay = ok(
        &f,
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            RECORD,
            "--hole-rect",
            "10,10,10,10",
        ],
    );
    assert_eq!(add_replay.get("revision"), a.get("revision"));
    assert_eq!(volume(add_replay.get("element").unwrap()), 98000.0);
    assert_eq!(ok(&f, "inspect-elements", &[]), head);

    let (code, replay) = edit(&f, &id, "10,10,20,10", EDIT);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(text(&replay, "outcome"), "hole_update_replayed");
    assert_eq!(replay.get("revision"), e.get("revision"));
    assert_eq!(volume(replay.get("element").unwrap()), 97000.0);
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
    assert_ne!(edit(&f, &id, "10,10,21,10", EDIT).0, 0);
    for rect in [
        "-10,0,5,5",
        "10,10,-1,10",
        "70,70,10,10",
        "0,0,10000000000,10",
    ] {
        assert_ne!(
            edit(&f, &id, rect, "019a7bf0-0000-7000-8000-000000ad0010").0,
            0
        );
        assert_eq!(ok(&f, "inspect-elements", &[]), head);
    }
    ok(&f, "delete-element", &["--id", &id]);
    let head = ok(&f, "inspect-elements", &[]);
    assert_eq!(edit(&f, &id, "10,10,20,10", EDIT).0, 0);
    assert_eq!(ok(&f, "inspect-elements", &[]), head);
}
#[test]
fn linked_holes_edit_and_undo_as_one_unit_and_keep_each_instance_placement() {
    let f = StoreFixture::new("linked-hole-updates");
    let id = plate(&f);
    ok(
        &f,
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            RECORD,
            "--hole-rect",
            "10,10,10,10",
        ],
    );
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
    let before = run("inspect-elements", &[]);
    run(
        "update-hole",
        &[
            "--id",
            &instance,
            "--operation-id",
            RECORD,
            "--hole-rect",
            "10,10,20,10",
            "--op-id",
            EDIT,
        ],
    );
    let after = run("inspect-elements", &[]);
    let vols = |j: &Json| {
        j.get("plates")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(volume)
            .collect::<Vec<_>>()
    };
    let poses = |j: &Json| {
        j.get("plates")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|p| p.get("placement").unwrap().clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(vols(&after), vec![97000.0; 2]);
    assert_eq!(poses(&after), poses(&before));
    run("undo-workspace", &[]);
    assert_eq!(vols(&run("inspect-elements", &[])), vols(&before));
    run("redo-workspace", &[]);
    assert_eq!(vols(&run("inspect-elements", &[])), vols(&after));
}
#[test]
fn missing_record_and_oversized_or_ill_wound_polygons_refuse_without_a_revision() {
    let f = StoreFixture::new("hole-update-refusals");
    let id = plate(&f);
    let before = ok(&f, "inspect-elements", &[]);
    assert_ne!(edit(&f, &id, "10,10,10,10", EDIT).0, 0);
    assert_ne!(
        f.run("update-hole", &["--id", &id, "--hole-rect", "10,10,10,10"])
            .0,
        0
    );
    assert_ne!(
        f.run(
            "update-hole",
            &[
                "--id",
                &id,
                "--operation-id",
                RECORD,
                "--polygon",
                "10,10;20,10;20,20;10,20"
            ]
        )
        .0,
        0
    );
    let oversized = "1,1;".repeat(1100);
    assert_ne!(
        f.run(
            "update-hole",
            &[
                "--id",
                &id,
                "--operation-id",
                RECORD,
                "--polygon",
                &oversized
            ]
        )
        .0,
        0
    );
    assert_eq!(ok(&f, "inspect-elements", &[]), before);
}
