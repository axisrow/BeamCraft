// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected volumes are closed forms: the box minus
// the polygon area, times the thickness.

//! A04 `AddHole` end to end through the real `beamcraft` binary: one through-hole operation
//! record per command, replayed with the plate's other holes, verified by fresh processes on
//! reopen, undone as one unit and copied with fresh record ids.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::process::Command;

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a04-add-hole-tests"),
        }
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        text(&body, "code")
    }
    /// A command run exactly as given, for workspace-level commands and a store given
    /// explicitly.
    fn ok_raw(&self, args: &[&str]) -> Json {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        assert_eq!(output.status.code().unwrap(), 0, "{args:?}: {body:?}");
        body
    }
    fn plate(&self, extra: &[&str]) -> Json {
        let mut args = vec![
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--name",
            "Record plate",
        ];
        args.extend_from_slice(extra);
        self.ok("create-plate", &args)
    }
    /// The head revision as a fresh process sees it, for the one element or the whole model.
    fn inspect(&self, element: Option<&str>) -> Json {
        let mut args = vec![];
        if let Some(element) = element {
            args.extend_from_slice(&["--element", element]);
        }
        self.ok("inspect-elements", &args)
    }
    fn plates(&self) -> Vec<Json> {
        self.inspect(None)
            .get("plates")
            .unwrap()
            .array()
            .unwrap()
            .to_vec()
    }
}
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
/// Signed shoelace area of one reported polygon (mm2).
fn area(hole: &Json) -> f64 {
    let points = hole.array().unwrap();
    let xy: Vec<(f64, f64)> = points
        .iter()
        .map(|p| {
            let pair = p.array().unwrap();
            (pair[0].float().unwrap(), pair[1].float().unwrap())
        })
        .collect();
    let twice: f64 = xy
        .iter()
        .zip(xy.iter().skip(1).chain(xy.iter().take(1)))
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum();
    twice.abs() / 2.0
}

#[test]
fn a_record_hole_gives_the_exact_volume_and_reopens_verified() {
    let f = Fixture::new();
    let steel = text(
        at(
            &f.ok("define-material", &["--density", "7850"]),
            &["definition"],
        ),
        "digest",
    );
    let created = f.plate(&["--material-digest", &steel]);
    let id = text(at(&created, &["plate"]), "element_id");
    assert_eq!(
        num(at(&created, &["plate"]), &["quantities", "volume", "mm3"]),
        5_000_000.0
    );
    let operation_id = "019a7bf0-0000-7000-8000-0000000000a4";
    let added = f.ok(
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            operation_id,
            "--hole-rect",
            "100,100,100,100",
        ],
    );
    assert_eq!(text(&added, "outcome"), "hole_added");
    let plate = at(&added, &["element"]);
    assert_eq!(text(plate, "element_id"), id);
    assert_eq!(num(plate, &["hole_count"]), 1.0);
    assert_eq!(
        at(plate, &["hole_records"]).array().unwrap().len(),
        1,
        "{plate:?}"
    );
    let record = at(plate, &["hole_records"]).array().unwrap()[0].clone();
    assert_eq!(text(&record, "operation_id"), operation_id);
    assert_eq!(num(&record, &["ordinal"]), 0.0);
    // (1000 * 500 - 100 * 100) * 10 = 4,900,000 mm3; * 7850 kg/m3 = 38.465 kg.
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!((volume - 4_900_000.0).abs() <= 1e-6, "{volume}");
    let mass = num(plate, &["quantities", "mass", "kg"]);
    assert!((mass - 38.465).abs() <= 1e-9, "{mass}");

    // A fresh process reopens, re-cuts and re-verifies the stored result.
    let inspected = f.inspect(Some(&id));
    let reopened = &at(&inspected, &["plates"]).array().unwrap()[0];
    assert_eq!(num(reopened, &["hole_count"]), 1.0);
    assert_eq!(num(reopened, &["quantities", "volume", "mm3"]), 4_900_000.0);
    let records = at(reopened, &["hole_records"]).array().unwrap();
    assert_eq!(num(&records[0], &["ordinal"]), 0.0);
}

#[test]
fn a_round_hole_is_the_contract_n_gon_and_its_polygon_volume() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let id = text(at(&created, &["plate"]), "element_id");
    let added = f.ok(
        "add-hole",
        &[
            "--id",
            &id,
            "--hole-round",
            "250,250,100",
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000b0",
        ],
    );
    let plate = at(&added, &["element"]);
    // F06 section 2 (A-2): a 100 mm diameter decides N = 72.
    let holes = at(plate, &["holes_mm"]).array().unwrap();
    assert_eq!(holes.len(), 1);
    let vertices = holes[0].array().unwrap().len();
    assert_eq!(vertices, 72);
    // The stored polygon is the geometry: its exact polygon volume is the quantity.
    let polygon_area = area(&holes[0]);
    let expected = (500_000.0 - polygon_area) * 10.0;
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!((volume - expected).abs() <= 1e-6, "{volume} vs {expected}");
    // The 72-gon of a 100 mm circle covers less than a 100 x 100 square hole.
    assert!(volume > 4_900_000.0, "{volume}");
}

#[test]
fn a_retry_replays_and_a_reused_id_refuses_without_writing() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let id = text(at(&created, &["plate"]), "element_id");
    let args = [
        "--id",
        &id,
        "--operation-id",
        "019a7bf0-0000-7000-8000-0000000000c0",
        "--hole-rect",
        "100,100,100,100",
    ];
    let first = f.ok("add-hole", &args);
    assert_eq!(text(&first, "outcome"), "hole_added");
    let revision = text(at(&first, &["revision"]), "revision_id");
    // The identical command is the recorded result, not a second revision.
    let retry = f.ok("add-hole", &args);
    assert_eq!(text(&retry, "outcome"), "hole_add_replayed");
    assert_eq!(text(at(&retry, &["revision"]), "revision_id"), revision);
    // A different hole under the recorded id is a reuse refusal. (A record id that the
    // ledger never accepted is the engine's DUPLICATE_ID; with one durable ledger a retry
    // of a committed record id always arrives as a replay or a reuse refusal.)
    let code = f.rejected(
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000c0",
            "--hole-rect",
            "700,300,50,50",
        ],
    );
    assert_eq!(code, "OP_ID_REUSE");
    // A hole the kernel refuses writes nothing: the next accepted hole is the second.
    let code = f.rejected(
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000c2",
            "--hole-rect",
            "950,100,100,100",
        ],
    );
    assert_eq!(code, "INVALID_ARGUMENT");
    let second = f.ok(
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000c3",
            "--hole-rect",
            "400,200,50,50",
        ],
    );
    let plate = at(&second, &["element"]);
    assert_eq!(num(plate, &["hole_count"]), 2.0);
    assert_eq!(num(plate, &["quantities", "volume", "mm3"]), 4_875_000.0);
    let records = at(plate, &["hole_records"]).array().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(num(&records[1], &["ordinal"]), 1.0, "ordinals never reuse");
}

#[test]
fn a_member_target_takes_a_member_hole_and_a_polygon_generator_stays_unsupported() {
    let f = Fixture::new();
    let profile = text(
        at(
            &f.ok("define-profile", &["--width", "200", "--height", "300"]),
            &["definition"],
        ),
        "digest",
    );
    let member = f.ok(
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            &profile,
            "--name",
            "Beam B1",
        ],
    );
    let member_id = text(at(&member, &["member"]), "element_id");
    // A plate generator on a member target stays refused (F06 member-hole amendment section 3).
    let code = f.rejected(
        "add-hole",
        &[
            "--id",
            &member_id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000d0",
            "--hole-round",
            "3000,0,50",
        ],
    );
    assert_eq!(code, "UNSUPPORTED_GEOMETRY");
    // A member hole: a through round hole along X_r at the section centre. The member frame of
    // an X-axis beam is X_r = element +Y, Y_r = element +Z, Z_m = +X, so the hole removes
    // polygon area x width (200 mm) from the 200x300x6000 gross.
    let added = f.ok(
        "add-hole",
        &[
            "--id",
            &member_id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000d1",
            "--axis-point",
            "0,0,3000",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "100",
        ],
    );
    let element = at(&added, &["element"]);
    let records = at(element, &["hole_records"]).array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(num(&records[0], &["ordinal"]), 0.0);
    assert_eq!(text(&records[0], "extent"), "through");
    // The 0.05 mm chord bound gives d = 100 seventy-two segments; area x 200 is about
    // 1,568,803 mm3 of the 360,000,000 gross.
    assert!((num(element, &["quantities", "volume", "mm3"]) - 358_431_196.6).abs() <= 10.0);
    // Update keeps the identity and the ordinal and changes the volume.
    let updated = f.ok(
        "update-hole",
        &[
            "--id",
            &member_id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000d1",
            "--axis-point",
            "0,0,3000",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "200",
        ],
    );
    let element = at(&updated, &["element"]);
    let records = at(element, &["hole_records"]).array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        num(&records[0], &["ordinal"]),
        0.0,
        "update keeps the ordinal"
    );
    // d = 200 takes one hundred segments; area x 200 is about 6,279,052 mm3.
    assert!((num(element, &["quantities", "volume", "mm3"]) - 353_720_948.0).abs() <= 10.0);
    // Removal restores the gross volume end to end.
    let removed = f.ok(
        "remove-hole",
        &[
            "--id",
            &member_id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000d1",
        ],
    );
    let element = at(&removed, &["element"]);
    assert!(at(element, &["hole_records"]).array().unwrap().is_empty());
    assert_eq!(
        num(element, &["quantities", "volume", "mm3"]),
        360_000_000.0
    );
    // An absent target is still not found.
    let code = f.rejected(
        "add-hole",
        &[
            "--id",
            "019a7bf0-0000-7000-8000-0000000000d1",
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000d2",
            "--axis-point",
            "0,0,0",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "50",
        ],
    );
    assert_eq!(code, "NOT_FOUND");
}

#[test]
fn a_member_hole_that_no_longer_fits_refuses_and_a_ranged_one_waits_for_the_kernel() {
    let f = Fixture::new();
    let profile = text(
        at(
            &f.ok("define-profile", &["--width", "200", "--height", "300"]),
            &["definition"],
        ),
        "digest",
    );
    let member = f.ok(
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            &profile,
            "--name",
            "Beam B2",
        ],
    );
    let member_id = text(at(&member, &["member"]), "element_id");
    // The transverse coordinate beyond the section (v = 500 > height 300) leaves no fit, the
    // same typed refusal a plate hole gets.
    let code = f.rejected(
        "add-hole",
        &[
            "--id",
            &member_id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000d5",
            "--axis-point",
            "0,500,3000",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "50",
        ],
    );
    assert_eq!(code, "INVALID_ARGUMENT");
    // A range ends the cutter inside the body: the kernel slice G14.1 certifies it later.
    let code = f.rejected(
        "add-hole",
        &[
            "--id",
            &member_id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000d6",
            "--axis-point",
            "0,0,3000",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "50",
            "--from",
            "10",
            "--to",
            "100",
        ],
    );
    assert_eq!(code, "UNSUPPORTED_GEOMETRY");
    // Neither refusal wrote anything: a fit hole is still accepted afterwards.
    let added = f.ok(
        "add-hole",
        &[
            "--id",
            &member_id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000d7",
            "--axis-point",
            "0,0,3000",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "50",
        ],
    );
    assert_eq!(
        at(&added, &["element", "hole_records"])
            .array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn undo_removes_the_record_and_the_volume_returns() {
    let f = Fixture::new();
    let user = "019a7bf0-0000-7000-8000-0000000000e0";
    let created = f.plate(&[]);
    let id = text(at(&created, &["plate"]), "element_id");
    let workspace = f.shared.root.join("ws");
    fs::create_dir(&workspace).unwrap();
    let workspace = workspace.to_str().unwrap().to_owned();
    f.ok(
        "create-workspace",
        &["--workspace-dir", &workspace, "--user-id", user],
    );
    // Edits run against the forked workspace store, as in the undo suite.
    let added = f.ok_raw(&[
        "add-hole",
        "--store",
        &workspace,
        "--id",
        &id,
        "--operation-id",
        "019a7bf0-0000-7000-8000-0000000000e1",
        "--hole-rect",
        "100,100,100,100",
    ]);
    assert_eq!(text(&added, "outcome"), "hole_added");
    let undone = f.ok_raw(&["undo-workspace", "--workspace-dir", &workspace]);
    assert_eq!(text(&undone, "outcome"), "workspace_undone");
    let body = f.ok_raw(&["inspect-elements", "--store", &workspace]);
    let plate = &at(&body, &["plates"]).array().unwrap()[0];
    assert_eq!(num(plate, &["hole_count"]), 0.0);
    assert_eq!(num(plate, &["quantities", "volume", "mm3"]), 5_000_000.0);
    assert_eq!(
        at(plate, &["hole_records"]).array().unwrap().len(),
        0,
        "undo restores the exact previous state"
    );
    // The undone hole can be re-added: the rolled-back record frees its identity.
    let again = f.ok_raw(&[
        "add-hole",
        "--store",
        &workspace,
        "--id",
        &id,
        "--operation-id",
        "019a7bf0-0000-7000-8000-0000000000e2",
        "--hole-rect",
        "100,100,100,100",
    ]);
    assert_eq!(text(&again, "outcome"), "hole_added");
}

#[test]
fn a_copy_takes_fresh_operation_ids_and_the_same_ordinals() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let id = text(at(&created, &["plate"]), "element_id");
    f.ok(
        "add-hole",
        &[
            "--id",
            &id,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000000000f0",
            "--hole-rect",
            "100,100,100,100",
        ],
    );
    let copied = f.ok("copy-element", &["--id", &id]);
    let copy_id = text(at(&copied, &["element"]), "element_id");
    assert_ne!(copy_id, id);
    let plates = f.plates();
    let record_of = |element_id: &str| {
        plates
            .iter()
            .find(|p| text(p, "element_id") == element_id)
            .unwrap()
            .clone()
    };
    let source = record_of(&id);
    let copy = record_of(&copy_id);
    let source_record = at(&source, &["hole_records"]).array().unwrap()[0].clone();
    let copy_record = at(&copy, &["hole_records"]).array().unwrap()[0].clone();
    assert_ne!(
        text(&source_record, "operation_id"),
        text(&copy_record, "operation_id"),
        "a copy's records take fresh ids"
    );
    assert_eq!(
        num(&source_record, &["ordinal"]),
        num(&copy_record, &["ordinal"])
    );
    assert_eq!(
        num(&copy, &["quantities", "volume", "mm3"]),
        num(&source, &["quantities", "volume", "mm3"])
    );
}
