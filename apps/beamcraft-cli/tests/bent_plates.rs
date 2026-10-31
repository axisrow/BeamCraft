// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected volumes are the closed forms of the
// mid-surface invariance (the thickness times the developed blank area), never production math.

//! A10 end to end through the real `beamcraft` binary: `bend-plate` appends a fold or a roll
//! record to a stored plate, the bent body and its quantities are rebuilt in the same
//! transaction, the unfolded outline stays the authored contour, the refusals change nothing,
//! and a copy and the workspace undo keep the records exact.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support;

use beamcraft_json::Json;
use std::process::Command;

const FOLD_OP: &str = "019a7bf0-0000-7000-8000-0000000000a1";
const ROLL_OP: &str = "019a7bf0-0000-7000-8000-0000000000a2";
const OVERLAP_OP: &str = "019a7bf0-0000-7000-8000-0000000000a3";
const UNDO_OP: &str = "019a7bf0-0000-7000-8000-0000000000a4";
const FOLD_HOST_OP: &str = "019a7bf0-0000-7000-8000-0000000000b1";
const ROLL_HOST_OP: &str = "019a7bf0-0000-7000-8000-0000000000b2";

fn ok(f: &support::StoreFixture, command: &str, extra: &[&str]) -> Json {
    let (code, body) = f.run(command, extra);
    assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
    body
}

fn refused(f: &support::StoreFixture, command: &str, extra: &[&str]) {
    let before = ok(f, "open-revision", &[]);
    let (code, body) = f.run(command, extra);
    assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
    assert_eq!(
        before,
        ok(f, "open-revision", &[]),
        "{command} changed the store"
    );
}

fn text(j: &Json, key: &str) -> String {
    j.get(key).unwrap().string().unwrap().to_owned()
}

fn at<'a>(j: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(j, |j, k| j.get(k).unwrap())
}

fn num(j: &Json, path: &[&str]) -> f64 {
    at(j, path)
        .float()
        .unwrap_or_else(|error| panic!("path {path:?} is not a number in {j:?}: {error:?}"))
}

fn list<'a>(j: &'a Json, key: &str) -> &'a [Json] {
    j.get(key)
        .unwrap_or_else(|_| panic!("key {key} missing from {j:?}"))
        .array()
        .unwrap_or_else(|error| panic!("key {key} is not an array in {j:?}: {error:?}"))
}

fn plates(f: &support::StoreFixture) -> Vec<Json> {
    list(&ok(f, "inspect-elements", &[]), "plates").to_vec()
}

fn by_id<'a>(plates: &'a [Json], id: &str) -> &'a Json {
    plates
        .iter()
        .find(|p| text(p, "element_id") == id)
        .unwrap_or_else(|| panic!("plate {id} is not listed"))
}

fn volume_mm3(plate: &Json) -> f64 {
    num(plate, &["quantities", "volume", "mm3"])
}

fn create(f: &support::StoreFixture, along: &str, across: &str, name: &str) -> String {
    let created = ok(
        f,
        "create-plate",
        &[
            "--width",
            along,
            "--length",
            across,
            "--thickness",
            "2",
            "--name",
            name,
        ],
    );
    text(at(&created, &["plate"]), "element_id")
}

fn bend(f: &support::StoreFixture, id: &str, extra: &[&str]) -> Json {
    let mut args = vec!["--id", id];
    args.extend_from_slice(extra);
    ok(f, "bend-plate", &args)
}

#[test]
fn a_fold_records_the_bend_and_keeps_the_unfold() {
    let f = support::StoreFixture::new("a10-fold");
    let id = create(&f, "1000", "2000", "Folded");

    // The blank is the developed mid-surface outline; one right-angle fold on r 8 through the
    // middle keeps the volume at t x blank area = 2 x 1000 x 2000.
    let applied = bend(
        &f,
        &id,
        &[
            "--operation-id",
            FOLD_OP,
            "--op-id",
            FOLD_HOST_OP,
            "--kind",
            "fold",
            "--point-a",
            "0,1000,0",
            "--point-b",
            "1000,1000,0",
            "--radius",
            "8",
            "--angle-rad",
            "1.5707963267948966",
        ],
    );
    assert_eq!(text(&applied, "outcome"), "bend_applied");
    let listed = plates(&f);
    let plate = by_id(&listed, &id);
    let volume = volume_mm3(plate);
    assert!((volume - 4_000_000.0).abs() <= 8_000.0, "volume {volume}");
    // The unfold is the authored contour: the inspection reports it as the outline source and
    // the bend record carries its parameters.
    assert_eq!(text(plate, "outline"), "contour");
    assert_eq!(list(plate, "contour_points_mm").len(), 4);
    let bends = list(plate, "bends");
    assert_eq!(bends.len(), 1);
    assert_eq!(text(&bends[0], "bend"), "fold");
    assert_eq!(text(&bends[0], "operation_id"), FOLD_OP);
    assert!(bends[0].get("angle_rad").is_ok());

    // A retry with the same operation identity replays the recorded result and writes nothing.
    let replay = bend(
        &f,
        &id,
        &[
            "--operation-id",
            FOLD_OP,
            "--op-id",
            FOLD_HOST_OP,
            "--kind",
            "fold",
            "--point-a",
            "0,1000,0",
            "--point-b",
            "1000,1000,0",
            "--radius",
            "8",
            "--angle-rad",
            "1.5707963267948966",
        ],
    );
    assert_eq!(text(&replay, "outcome"), "bend_replayed");

    // A second fold whose bend region overlaps the first is refused with the store unchanged.
    refused(
        &f,
        "bend-plate",
        &[
            "--id",
            &id,
            "--operation-id",
            OVERLAP_OP,
            "--kind",
            "fold",
            "--point-a",
            "0,1001,0",
            "--point-b",
            "1000,1001,0",
            "--radius",
            "8",
            "--angle-rad",
            "1.5707963267948966",
        ],
    );

    // One command is one undo unit: a personal workspace holds the draft, the bend is applied
    // in it, and the undo rolls the bent body back to the flat plate.
    let draft = f.root.join("draft");
    std::fs::create_dir(&draft).unwrap();
    let draft = draft.to_str().unwrap().to_owned();
    ok(&f, "create-workspace", &["--workspace-dir", &draft]);
    let in_draft = |command: &str, extra: &[&str]| {
        let (code, body) = support::StoreFixture::run_in(&draft, command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    };
    let draft_id = text(
        at(
            &in_draft(
                "create-plate",
                &[
                    "--width",
                    "1000",
                    "--length",
                    "2000",
                    "--thickness",
                    "2",
                    "--name",
                    "Draft",
                ],
            ),
            &["plate"],
        ),
        "element_id",
    );
    in_draft(
        "bend-plate",
        &[
            "--id",
            &draft_id,
            "--operation-id",
            UNDO_OP,
            "--kind",
            "fold",
            "--point-a",
            "0,1000,0",
            "--point-b",
            "1000,1000,0",
            "--radius",
            "8",
            "--angle-rad",
            "1.5707963267948966",
        ],
    );
    let bent = in_draft("inspect-elements", &[]);
    let bent_volume = volume_mm3(by_id(list(&bent, "plates"), &draft_id));
    assert!(
        (bent_volume - 4_000_000.0).abs() <= 8_000.0,
        "bent draft volume {bent_volume}"
    );
    // The undo takes no store: it rolls the draft workspace itself.
    let mut undo = Command::new(env!("CARGO_BIN_EXE_beamcraft"));
    undo.args(["undo-workspace", "--workspace-dir", draft.as_str()]);
    let undo = undo.output().unwrap();
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stdout)
    );
    let flat = in_draft("inspect-elements", &[]);
    let flat_plate = by_id(list(&flat, "plates"), &draft_id);
    assert_eq!(list(flat_plate, "bends").len(), 0);
    let flat_volume = volume_mm3(flat_plate);
    assert!(
        (flat_volume - 4_000_000.0).abs() <= 1e-6,
        "flat volume {flat_volume}"
    );
}

#[test]
fn a_roll_records_the_cylinder_and_a_copy_keeps_it() {
    let f = support::StoreFixture::new("a10-roll");
    let id = create(&f, "1000", "20", "Rolled");

    // The 20 mm developed length wraps the r 8 cylinder over 20/9 rad; the volume stays
    // t x blank area = 2 x 1000 x 20.
    bend(
        &f,
        &id,
        &[
            "--operation-id",
            ROLL_OP,
            "--op-id",
            ROLL_HOST_OP,
            "--kind",
            "roll",
            "--point-a",
            "0,0,0",
            "--point-b",
            "1000,0,0",
            "--radius",
            "8",
        ],
    );
    let rolled_list = plates(&f);
    let plate = by_id(&rolled_list, &id);
    let volume = volume_mm3(plate);
    assert!((volume - 40_000.0).abs() <= 600.0, "volume {volume}");
    let bends = list(plate, "bends");
    assert_eq!(bends.len(), 1);
    assert_eq!(text(&bends[0], "bend"), "roll");

    // A roll never joins a plate that already carries a bend.
    refused(
        &f,
        "bend-plate",
        &[
            "--id",
            &id,
            "--operation-id",
            OVERLAP_OP,
            "--kind",
            "roll",
            "--point-a",
            "0,0,0",
            "--point-b",
            "1000,0,0",
            "--radius",
            "9",
        ],
    );

    // A copy carries the same record with a fresh identity and the same bent body.
    let copy = ok(&f, "copy-element", &["--id", &id, "--name", "Rolled copy"]);
    let copy_id = text(at(&copy, &["element"]), "element_id");
    let copied_list = plates(&f);
    let copied = by_id(&copied_list, &copy_id);
    let copy_bends = list(copied, "bends");
    assert_eq!(copy_bends.len(), 1);
    assert_eq!(text(&copy_bends[0], "bend"), "roll");
    assert_ne!(text(&copy_bends[0], "operation_id"), ROLL_OP);
    let copy_volume = volume_mm3(copied);
    assert!(
        (copy_volume - volume).abs() <= 1.0,
        "{copy_volume} vs {volume}"
    );
}
