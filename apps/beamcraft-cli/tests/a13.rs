// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic fixtures, MPL-2.0. Expected volumes are closed forms.

//! A13 fabrication operations end to end through the real `beamcraft` binary: line, polygon
//! and part cuts, fitting an end to another part, chamfers, split and combine, each as an
//! A-2 operation record or an atomic state change. Every accepted command rebuilds the mesh
//! and the quantities in the same transaction, a fresh process reopens and re-verifies, the
//! saved layer text carries the record as plain values, and the workspace undo takes each
//! operation back (and a split or combine restores both elements).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    calls: std::cell::Cell<u32>,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a13-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            crate::support::boot_nonce(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        Self {
            root,
            calls: std::cell::Cell::new(0),
        }
    }
    fn store(&self) -> String {
        self.root.join("store").to_str().unwrap().to_owned()
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.run_in(&self.store(), command, extra)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        let index = self.calls.get();
        self.calls.set(index + 1);
        assert_eq!(code, 0, "call #{index} {command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        text(&body, "code")
    }
    fn plate(&self, extra: &[&str]) -> Json {
        let mut args = vec!["--width", "1000", "--length", "500", "--thickness", "10"];
        args.extend_from_slice(extra);
        self.ok("create-plate", &args)
    }
    /// A plate whose full arguments the caller gives (a tool plate with its own dimensions).
    fn tool_plate(&self, extra: &[&str]) -> Json {
        self.ok("create-plate", extra)
    }
    /// A `200 x 300` rectangular profile digest, the one straight-member section of the tests.
    fn beam_profile(&self) -> String {
        let reply = self.ok("define-profile", &["--width", "200", "--height", "300"]);
        text(at(&reply, &["definition"]), "digest")
    }
    fn volume_of(&self, reply: &Json, key: &str) -> f64 {
        num(at(reply, &[key]), &["quantities", "volume", "mm3"])
    }
    fn inspected(&self) -> Json {
        self.ok("inspect-elements", &[])
    }
    fn authored_text(&self, revision: &Json) -> String {
        let layers = at(revision, &["revision", "manifest", "layers"])
            .array()
            .unwrap();
        let layer = layers
            .iter()
            .find(|l| text(l, "name") == "authored.usda")
            .unwrap();
        let digest = text(layer, "digest");
        let path = self
            .root
            .join("store")
            .join("blobs")
            .join(&digest[..2])
            .join(&digest);
        String::from_utf8(fs::read(path).unwrap()).unwrap()
    }
    /// A personal workspace over its own store copy, for the undo acceptance.
    fn draft(&self) -> String {
        let path = self
            .root
            .join(format!("draft-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        let draft = path.to_str().unwrap().to_owned();
        let (code, body) = self.run("create-workspace", &["--workspace-dir", &draft]);
        assert_eq!(code, 0, "create-workspace: {body:?}");
        draft
    }
    fn run_in(&self, workspace: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        let mut args = vec![command, "--store", workspace];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        assert!(
            output.status.code().is_some() || !output.stderr.is_empty(),
            "{command} was killed"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            eprintln!("child stderr [{command}]: {stderr}");
        }
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap_or(3), body)
    }
    fn ok_in(&self, workspace: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run_in(workspace, command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn history(&self, workspace: &str, command: &str) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([command, "--workspace-dir", workspace])
            .output()
            .unwrap();
        assert!(output.status.success(), "{command}");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(json: &Json, key: &str) -> String {
    let value = json
        .get(key)
        .unwrap_or_else(|_| panic!("{key} missing from {json:?}"));
    value.string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
/// The volume of the one plate an `inspect-elements` reply lists.
fn first_plate_volume(inspected: &Json) -> f64 {
    let plates = at(inspected, &["plates"]).array().unwrap();
    num(&plates[0], &["quantities", "volume", "mm3"])
}

const OP_A: &str = "01b00000-0000-7000-8000-0000000000a1";
const OP_B: &str = "01b00000-0000-7000-8000-0000000000a2";

#[test]
fn a_line_cut_trims_the_plate_reopens_and_undoes() {
    let f = Fixture::new();
    let steel = {
        let reply = f.ok("define-material", &["--density", "7850"]);
        text(at(&reply, &["definition"]), "digest")
    };
    let created = f.plate(&["--material-digest", &steel]);
    let element = text(at(&created, &["plate"]), "element_id");
    let cut = f.ok(
        "cut-by-line",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--point-a",
            "0,250,0",
            "--point-b",
            "1000,250,0",
            "--axis",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(text(&cut, "outcome"), "a13_applied");
    assert_eq!(num(&cut, &["record", "ordinal"]), 0.0);
    // The plane contains the line and Z: exactly one half is kept.
    let volume = f.volume_of(&cut, "element");
    assert!((volume - 2_500_000.0).abs() <= 1e-6, "{volume}");
    let mass = num(at(&cut, &["element"]), &["quantities", "mass", "kg"]);
    assert!((mass - 2_500_000.0 * 7850.0 / 1e9).abs() <= 1e-9, "{mass}");
    // A retry with the same operation id and content is a replay; changed content under the
    // accepted operation id is a reuse refusal, and a new command reusing the record identity
    // is a duplicate.
    let again = f.ok(
        "cut-by-line",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--point-a",
            "0,250,0",
            "--point-b",
            "1000,250,0",
            "--axis",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(text(&again, "outcome"), "a13_replayed");
    let kind = f.rejected(
        "cut-by-line",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--point-a",
            "0,250,0",
            "--point-b",
            "1000,250,0",
            "--axis",
            "0,1,1",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(kind, "OP_ID_REUSE");
    let kind = f.rejected(
        "cut-by-line",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--point-a",
            "0,250,0",
            "--point-b",
            "1000,250,0",
            "--axis",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(kind, "DUPLICATE_ID");
    // A parallel axis never reaches the model, and nothing was published by any refusal.
    let kind = f.rejected(
        "cut-by-line",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_B,
            "--point-a",
            "0,250,0",
            "--point-b",
            "1000,250,0",
            "--axis",
            "1,0,0",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(kind, "INVALID_ARGUMENT");
    let inspected = f.inspected();
    assert_eq!(first_plate_volume(&inspected), volume);

    // The workspace undo takes the cut back; redo reapplies it. The draft cut is its own
    // operation (a fresh record identity and line), so undo has a real command to revert.
    let draft = f.draft();
    f.ok_in(
        &draft,
        "cut-by-line",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_B,
            "--op-id",
            OP_B,
            "--point-a",
            "0,100,0",
            "--point-b",
            "1000,100,0",
            "--axis",
            "0,0,1",
            "--keep",
            "negative",
        ],
    );
    let (_, before) = f.run_in(&draft, "inspect-elements", &[]);
    assert!((first_plate_volume(&before) - 1_500_000.0).abs() <= 1e-6);
    f.history(&draft, "undo-workspace");
    let (_, undone) = f.run_in(&draft, "inspect-elements", &[]);
    // The undo reverts only the draft's own cut; the head's record stays.
    assert!((first_plate_volume(&undone) - 2_500_000.0).abs() <= 1e-6);
    f.history(&draft, "redo-workspace");
    let (_, redone) = f.run_in(&draft, "inspect-elements", &[]);
    assert!((first_plate_volume(&redone) - 1_500_000.0).abs() <= 1e-6);
}

#[test]
fn a_polygon_cut_is_a_cut_record_not_a_hole() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    let cut = f.ok(
        "cut-by-polygon",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--polygon",
            "100,100;100,200;200,200;200,100",
        ],
    );
    let volume = f.volume_of(&cut, "element");
    assert!((volume - 4_900_000.0).abs() <= 1e-6, "{volume}");
    // The record is stored as a through cut of kind polygon_cut, beside the hole vocabulary.
    let authored = f.authored_text(&cut);
    assert!(
        authored.contains("steel:hole:kinds = [\"polygon_cut\"]"),
        "{authored}"
    );
    // A polygon crossing the outline is refused by the replay and publishes nothing.
    let kind = f.rejected(
        "cut-by-polygon",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_B,
            "--polygon",
            "900,100;900,200;1100,200;1100,100",
        ],
    );
    assert_eq!(kind, "INVALID_ARGUMENT");
    let inspected = f.inspected();
    assert_eq!(first_plate_volume(&inspected), volume);

    // The undo returns the solid plate; the draft cut is its own operation.
    let draft = f.draft();
    f.ok_in(
        &draft,
        "cut-by-polygon",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_B,
            "--polygon",
            "300,100;300,200;400,200;400,100",
        ],
    );
    f.history(&draft, "undo-workspace");
    let (_, undone) = f.run_in(&draft, "inspect-elements", &[]);
    // The undo reverts only the draft's own cut; the head's record stays.
    assert!((first_plate_volume(&undone) - 4_900_000.0).abs() <= 1e-6);
}

#[test]
fn a_part_cut_maps_the_tool_prism_and_refuses_the_rest() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    let tool_created = f.tool_plate(&[
        "--width",
        "100",
        "--length",
        "100",
        "--thickness",
        "20",
        "--translation",
        "150,150,-5",
        "--new-id",
        "01b00000-0000-7000-8000-0000000000d1",
    ]);
    let tool = text(at(&tool_created, &["plate"]), "element_id");
    let cut = f.ok(
        "cut-by-part",
        &["--id", &element, "--operation-id", OP_A, "--tool", &tool],
    );
    let volume = f.volume_of(&cut, "element");
    assert!((volume - 4_900_000.0).abs() <= 1e-6, "{volume}");
    // A tool that does not reach past both faces is refused and publishes nothing.
    let shallow = f.tool_plate(&[
        "--width",
        "100",
        "--length",
        "100",
        "--thickness",
        "20",
        "--translation",
        "150,150,5",
        "--new-id",
        "01b00000-0000-7000-8000-0000000000d2",
    ]);
    let shallow = text(at(&shallow, &["plate"]), "element_id");
    let kind = f.rejected(
        "cut-by-part",
        &["--id", &element, "--operation-id", OP_B, "--tool", &shallow],
    );
    assert_eq!(kind, "UNSUPPORTED_GEOMETRY");
    let inspected = f.inspected();
    assert_eq!(first_plate_volume(&inspected), volume);
}

#[test]
fn a_fit_trims_the_end_flush_with_the_tool_face() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    // The tool's near face lies at z = 5; the midpoint side is kept, so half the plate stays.
    let tool_created = f.tool_plate(&[
        "--width",
        "200",
        "--length",
        "200",
        "--thickness",
        "20",
        "--translation",
        "100,100,5",
        "--new-id",
        "01b00000-0000-7000-8000-0000000000e1",
    ]);
    let tool = text(at(&tool_created, &["plate"]), "element_id");
    let fit = f.ok(
        "fit-end-to-part",
        &["--id", &element, "--operation-id", OP_A, "--tool", &tool],
    );
    let volume = f.volume_of(&fit, "element");
    assert!((volume - 2_500_000.0).abs() <= 1e-6, "{volume}");
    // An unknown tool is a not-found result.
    let kind = f.rejected(
        "fit-end-to-part",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_B,
            "--tool",
            "01b00000-0000-7000-8000-0000000000ff",
        ],
    );
    assert_eq!(kind, "NOT_FOUND");
}

#[test]
fn a_chamfer_removes_the_corner_triangle() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    let cut = f.ok(
        "chamfer-part",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--vertex-index",
            "0",
            "--first-distance",
            "100",
            "--second-distance",
            "100",
        ],
    );
    let volume = f.volume_of(&cut, "element");
    assert!((volume - 4_950_000.0).abs() <= 1e-6, "{volume}");
    // A distance past the edge is refused.
    let kind = f.rejected(
        "chamfer-part",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_B,
            "--vertex-index",
            "0",
            "--first-distance",
            "500",
            "--second-distance",
            "100",
        ],
    );
    assert_eq!(kind, "INVALID_ARGUMENT");
}

#[test]
fn a_split_makes_two_halves_and_undo_restores_one_element() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    let new_id = "01b00000-0000-7000-8000-0000000000f1";
    let split = f.ok(
        "split-part",
        &[
            "--id",
            &element,
            "--new-id",
            new_id,
            "--new-name",
            "Other half",
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--point",
            "250,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    // Both halves exist with the exact volumes; together they keep the full plate.
    let kept_volume = f.volume_of(&split, "kept");
    let created_volume = f.volume_of(&split, "created");
    assert!((kept_volume - 1_250_000.0).abs() <= 1e-6, "{kept_volume}");
    assert!(
        (created_volume - 3_750_000.0).abs() <= 1e-6,
        "{created_volume}"
    );
    let inspected = f.inspected();
    assert_eq!(at(&inspected, &["plates"]).array().unwrap().len(), 2);
    // An identity already in the project is refused.
    let kind = f.rejected(
        "split-part",
        &[
            "--id",
            &element,
            "--new-id",
            new_id,
            "--new-name",
            "Again",
            "--operation-id",
            OP_B,
            "--point",
            "250,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    assert_eq!(kind, "DUPLICATE_ID");
    // A retry of the accepted split under its operation id replays and names both halves.
    let again = f.ok(
        "split-part",
        &[
            "--id",
            &element,
            "--new-id",
            new_id,
            "--new-name",
            "Other half",
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--point",
            "250,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    assert_eq!(text(&again, "outcome"), "a13_replayed");
    assert_eq!(f.volume_of(&again, "kept"), kept_volume);
    assert_eq!(f.volume_of(&again, "created"), created_volume);
    // A retry that renames the created half is changed content under the accepted operation
    // id: a reuse refusal, never a silent replay under the old label.
    let kind = f.rejected(
        "split-part",
        &[
            "--id",
            &element,
            "--new-id",
            new_id,
            "--new-name",
            "Renamed half",
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--point",
            "250,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    assert_eq!(kind, "OP_ID_REUSE");
    // The undo removes the created half of the draft's own split and restores the two halves;
    // the draft splits the kept half again at a fresh plane with its own identities.
    let draft = f.draft();
    let draft_new_id = "01b00000-0000-7000-8000-0000000000f2";
    let draft_split = f.ok_in(
        &draft,
        "split-part",
        &[
            "--id",
            &element,
            "--new-id",
            draft_new_id,
            "--new-name",
            "Quarter",
            "--operation-id",
            OP_B,
            "--point",
            "100,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    let quarter_volume = f.volume_of(&draft_split, "kept");
    assert!(
        (quarter_volume - 500_000.0).abs() <= 1e-6,
        "{quarter_volume}"
    );
    f.history(&draft, "undo-workspace");
    let (_, undone) = f.run_in(&draft, "inspect-elements", &[]);
    assert_eq!(at(&undone, &["plates"]).array().unwrap().len(), 2);
}

#[test]
fn a_combine_extends_the_survivor_and_undo_restores_both() {
    let f = Fixture::new();
    let profile = f.beam_profile();
    let first = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            &profile,
            "--new-id",
            "01b00000-0000-7000-8000-0000000000c1",
        ],
    );
    let survivor = text(at(&first, &["member"]), "element_id");
    let second = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "4000,0,0",
            "--profile-digest",
            &profile,
            "--translation",
            "5000,0,0",
            "--new-id",
            "01b00000-0000-7000-8000-0000000000c2",
        ],
    );
    let consumed = text(at(&second, &["member"]), "element_id");
    // Members that do not overlap are refused and publish nothing.
    let third = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "4000,0,0",
            "--profile-digest",
            &profile,
            "--translation",
            "10000,0,0",
            "--new-id",
            "01b00000-0000-7000-8000-0000000000c3",
        ],
    );
    let apart = text(at(&third, &["member"]), "element_id");
    let kind = f.rejected(
        "combine-parts",
        &[
            "--survivor",
            &survivor,
            "--consumed",
            &apart,
            "--operation-id",
            OP_B,
        ],
    );
    assert_eq!(kind, "INVALID_ARGUMENT");
    let inspected = f.inspected();
    assert_eq!(at(&inspected, &["members"]).array().unwrap().len(), 3);

    // The combine is one revision: 9000 mm of a 200 x 300 section, 540,000,000 mm3, and the
    // undo brings the consumed member back with its own volume.
    let draft = f.draft();
    let combined = f.ok_in(
        &draft,
        "combine-parts",
        &[
            "--survivor",
            &survivor,
            "--consumed",
            &consumed,
            "--operation-id",
            OP_A,
        ],
    );
    let volume = f.volume_of(&combined, "element");
    assert!((volume - 540_000_000.0).abs() <= 1e-6, "{volume}");
    // The consumed member is gone; the refused "apart" member stays.
    let (_, combined_view) = f.run_in(&draft, "inspect-elements", &[]);
    assert_eq!(at(&combined_view, &["members"]).array().unwrap().len(), 2);
    f.history(&draft, "undo-workspace");
    let (_, undone) = f.run_in(&draft, "inspect-elements", &[]);
    let members = at(&undone, &["members"]).array().unwrap();
    assert_eq!(members.len(), 3);
    let volumes: Vec<f64> = members
        .iter()
        .map(|m| num(m, &["quantities", "volume", "mm3"]))
        .collect();
    assert!(
        volumes.iter().any(|v| (*v - 360_000_000.0).abs() <= 1e-6),
        "{volumes:?}"
    );
    assert!(
        volumes.iter().any(|v| (*v - 240_000_000.0).abs() <= 1e-6),
        "{volumes:?}"
    );
}

#[test]
fn an_a13_cut_rides_the_instance_group_with_the_fresh_body() {
    let f = Fixture::new();
    let steel = {
        let reply = f.ok("define-material", &["--density", "7850"]);
        text(at(&reply, &["definition"]), "digest")
    };
    let created = f.plate(&[
        "--material-digest",
        &steel,
        "--name",
        "Definition",
        "--new-id",
        "019a7bf0-0000-7000-8000-00000000b001",
    ]);
    let definition = text(at(&created, &["plate"]), "element_id");
    for (n, offset) in [
        ("019a7bf0-0000-7000-8000-00000000b002", "1100,0,0"),
        ("019a7bf0-0000-7000-8000-00000000b003", "2200,0,0"),
    ] {
        let copied = f.ok(
            "copy-instance",
            &[
                "--id",
                &definition,
                "--new-id",
                n,
                "--name",
                "Instance",
                "--offset",
                offset,
            ],
        );
        assert_eq!(text(&copied, "outcome"), "element_copied");
    }
    let group = [
        "019a7bf0-0000-7000-8000-00000000b001",
        "019a7bf0-0000-7000-8000-00000000b002",
        "019a7bf0-0000-7000-8000-00000000b003",
    ];
    // The line cut halves every member of the group: the fresh body and quantities ride with
    // the shared records, so no instance keeps the stale pre-cut mesh or volume.
    let cut = f.ok(
        "cut-by-line",
        &[
            "--id",
            &definition,
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--point-a",
            "0,250,0",
            "--point-b",
            "1000,250,0",
            "--axis",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(text(&cut, "outcome"), "a13_applied");
    let inspected = f.inspected();
    let plates = at(&inspected, &["plates"]).array().unwrap();
    assert_eq!(plates.len(), 3);
    for id in group {
        let member = plates.iter().find(|p| text(p, "element_id") == id).unwrap();
        let volume = num(member, &["quantities", "volume", "mm3"]);
        assert!((volume - 2_500_000.0).abs() <= 1e-6, "{id}: {volume}");
    }
    let mesh = text(&plates[0], "mesh_digest");
    for member in &plates[1..] {
        assert_eq!(text(member, "mesh_digest"), mesh);
    }
}

#[test]
fn retired_a13_cut_identities_are_never_reused_across_removal() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    let first = f.ok(
        "cut-by-line",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--point-a",
            "0,250,0",
            "--point-b",
            "1000,250,0",
            "--axis",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(num(&first, &["record", "ordinal"]), 0.0);
    // Removing the record retires its identity and its ordinal.
    let removal_op = "01b00000-0000-7000-8000-0000000000b1";
    let removed = f.ok(
        "remove-cut",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--op-id",
            removal_op,
        ],
    );
    assert!(
        (f.volume_of(&removed, "element") - 5_000_000.0).abs() <= 1e-6,
        "{removed:?}"
    );
    // The retired record id is refused for a fresh A13 record...
    let kind = f.rejected(
        "cut-by-line",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--point-a",
            "0,100,0",
            "--point-b",
            "1000,100,0",
            "--axis",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(kind, "DUPLICATE_ID");
    // ...and the new record takes the next ordinal of the persistent counter, never a reuse.
    let next = f.ok(
        "cut-by-line",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_B,
            "--op-id",
            OP_B,
            "--point-a",
            "0,100,0",
            "--point-b",
            "1000,100,0",
            "--axis",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(num(&next, &["record", "ordinal"]), 1.0);
}

#[test]
fn a_polygon_cut_takes_the_persistent_hole_counter_and_refuses_retired_ids() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    let hole_op = "01b00000-0000-7000-8000-0000000000c1";
    f.ok(
        "add-hole",
        &[
            "--id",
            &element,
            "--operation-id",
            hole_op,
            "--hole-rect",
            "100,100,100,100",
        ],
    );
    let remove_op = "01b00000-0000-7000-8000-0000000000c2";
    f.ok(
        "remove-hole",
        &[
            "--id",
            &element,
            "--operation-id",
            hole_op,
            "--op-id",
            remove_op,
        ],
    );
    // The retired hole identity is refused to a polygon cut as well: one record vocabulary,
    // one identity space.
    let kind = f.rejected(
        "cut-by-polygon",
        &[
            "--id",
            &element,
            "--operation-id",
            hole_op,
            "--polygon",
            "100,100;100,200;200,200;200,100",
        ],
    );
    assert_eq!(kind, "DUPLICATE_ID");
    // ...and the polygon cut takes the next ordinal of the persistent hole counter, so no
    // wall id of a removed record ever comes back.
    let cut = f.ok(
        "cut-by-polygon",
        &[
            "--id",
            &element,
            "--operation-id",
            OP_A,
            "--polygon",
            "100,100;100,200;200,200;200,100",
        ],
    );
    let authored = f.authored_text(&cut);
    assert!(authored.contains("steel:hole:ordinals = [1]"), "{authored}");
}

/// The volume of the one member an `inspect-elements` reply lists.
fn first_member_volume(inspected: &Json) -> f64 {
    let members = at(inspected, &["members"]).array().unwrap();
    num(&members[0], &["quantities", "volume", "mm3"])
}

/// The volume of the named member in an `inspect-elements` reply.
fn member_volume(inspected: &Json, id: &str) -> f64 {
    let members = at(inspected, &["members"]).array().unwrap();
    let member = members
        .iter()
        .find(|m| text(m, "element_id") == id)
        .unwrap_or_else(|| panic!("{id} missing from the inspection"));
    num(member, &["quantities", "volume", "mm3"])
}

#[test]
fn a_member_polygon_cut_slots_an_shs_end_to_end() {
    let f = Fixture::new();
    // A square hollow section with sharp corners (a zero radius): the wall set is all flat
    // faces, which the certified member replay serves. A filleted hollow section (a
    // hot-rolled RHS with r > 0) stays refused with the standing G14.2 wall-mix refusal;
    // `steel::engine::member_hole` regression-tests that refusal.
    let profile = text(
        at(
            &f.ok(
                "define-profile",
                &[
                    "--family", "rhs", "--h", "100", "--b", "100", "--t", "4", "--r", "0",
                ],
            ),
            &["definition"],
        ),
        "digest",
    );
    let created = f.ok(
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
            "--new-id",
            "01b00000-0000-7000-8000-0000000000f1",
        ],
    );
    let member = text(at(&created, &["member"]), "element_id");
    let gross = first_member_volume(&f.inspected());

    // The member form: a 20 x 20 slot through both walls normal to element Z, through the
    // whole body (the section direction the records compose along). Both openings sit clear
    // of the section corners, so the certified removed volume is the exact parallel-wall
    // identity, slot area x 2 x t = 3200 mm3.
    let cut = f.ok(
        "cut-by-polygon",
        &[
            "--id",
            &member,
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--axis-point",
            "0,0,3000",
            "--axis-direction",
            "0,1,0",
            "--polygon",
            "-10,-10;-10,10;10,10;10,-10",
        ],
    );
    assert_eq!(text(&cut, "outcome"), "a13_applied");
    let net = f.volume_of(&cut, "element");
    assert!((net - (gross - 3200.0)).abs() <= 0.001, "{net} vs {gross}");

    // The record is a member polygon cut: the eleventh record attribute carries the kind, and
    // the inspect response names it.
    let authored = f.authored_text(&cut);
    assert!(
        authored.contains("steel:mhole:kinds = [\"polygon_cut\"]"),
        "{authored}"
    );
    let records = at(at(&cut, &["element"]), &["hole_records"])
        .array()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(at(&records[0], &["ordinal"]).unsigned().unwrap(), 0);
    assert_eq!(text(&records[0], "kind"), "polygon_cut");
    assert_eq!(text(&records[0], "extent"), "through");

    // A retry is a replay; a removal through the hole command is refused: the record is a
    // polygon cut, not a hole.
    let again = f.ok(
        "cut-by-polygon",
        &[
            "--id",
            &member,
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--axis-point",
            "0,0,3000",
            "--axis-direction",
            "0,1,0",
            "--polygon",
            "-10,-10;-10,10;10,10;10,-10",
        ],
    );
    assert_eq!(text(&again, "outcome"), "a13_replayed");
    let kind = f.rejected("remove-hole", &["--id", &member, "--operation-id", OP_A]);
    assert_eq!(kind, "INVALID_ARGUMENT");

    // A retired record identity stays retired: a hole removed from this member cannot name
    // the member polygon cut that follows it (one project-wide record identity space).
    f.ok(
        "add-hole",
        &[
            "--id",
            &member,
            "--operation-id",
            "01b00000-0000-7000-8000-0000000000f7",
            "--axis-point",
            "0,0,5000",
            "--axis-direction",
            "0,1,0",
            "--diameter",
            "40",
        ],
    );
    f.ok(
        "remove-hole",
        &[
            "--id",
            &member,
            "--operation-id",
            "01b00000-0000-7000-8000-0000000000f7",
        ],
    );
    let kind = f.rejected(
        "cut-by-polygon",
        &[
            "--id",
            &member,
            "--operation-id",
            "01b00000-0000-7000-8000-0000000000f7",
            "--axis-point",
            "0,0,5000",
            "--axis-direction",
            "0,1,0",
            "--polygon",
            "-10,-10;-10,10;10,10;10,-10",
        ],
    );
    assert_eq!(kind, "DUPLICATE_ID");

    // The undo returns the head member; the redo restores both cuts (the draft's own slot at
    // another axis position composes with the head's along the same section direction).
    let draft = f.draft();
    f.ok_in(
        &draft,
        "cut-by-polygon",
        &[
            "--id",
            &member,
            "--operation-id",
            OP_B,
            "--axis-point",
            "0,0,4000",
            "--axis-direction",
            "0,1,0",
            "--polygon",
            "-10,-10;-10,10;10,10;10,-10",
        ],
    );
    f.history(&draft, "undo-workspace");
    let (_, undone) = f.run_in(&draft, "inspect-elements", &[]);
    assert!((first_member_volume(&undone) - (gross - 3200.0)).abs() <= 0.001);
    f.history(&draft, "redo-workspace");
    let (_, redone) = f.run_in(&draft, "inspect-elements", &[]);
    assert!((first_member_volume(&redone) - (gross - 6400.0)).abs() <= 0.001);
}

#[test]
fn a_member_polygon_cut_refuses_the_wrong_forms_and_coexists_with_a_hole() {
    let f = Fixture::new();
    let profile = f.beam_profile();
    let created = f.ok(
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
            "--new-id",
            "01b00000-0000-7000-8000-0000000000f2",
        ],
    );
    let member = text(at(&created, &["member"]), "element_id");
    // The plate form on a member is an argument error, not the old unsupported-geometry
    // refusal; an axial direction stays unsupported; the member form on a plate is the
    // mirrored argument error.
    let kind = f.rejected(
        "cut-by-polygon",
        &[
            "--id",
            &member,
            "--operation-id",
            OP_A,
            "--polygon",
            "-50,-100;-50,100;50,100;50,-100",
        ],
    );
    assert_eq!(kind, "INVALID_ARGUMENT");
    let kind = f.rejected(
        "cut-by-polygon",
        &[
            "--id",
            &member,
            "--operation-id",
            OP_A,
            "--axis-point",
            "0,0,3000",
            "--axis-direction",
            "0,0,1",
            "--polygon",
            "-50,-100;-50,100;50,100;50,-100",
        ],
    );
    assert_eq!(kind, "UNSUPPORTED_GEOMETRY");

    // A hand-placed round hole joins the same record list and replays with the cut: both
    // records run along the same section direction (the one the records compose along), the
    // hole first, the cut beside it.
    let added = f.ok(
        "add-hole",
        &[
            "--id",
            &member,
            "--operation-id",
            "01b00000-0000-7000-8000-0000000000f3",
            "--axis-point",
            "0,0,4500",
            "--axis-direction",
            "0,1,0",
            "--diameter",
            "40",
        ],
    );
    let holed = f.volume_of(&added, "element");
    let records = at(at(&added, &["element"]), &["hole_records"])
        .array()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(text(&records[0], "kind"), "hole");

    // The cut: a 20 x 20 slot through the solid section, composing with the hole. The cutter
    // crosses the 300 mm section in one chord, so the certified removed volume is the exact
    // parallel-wall identity, slot area x 300 = 120,000 mm3.
    let cut = f.ok(
        "cut-by-polygon",
        &[
            "--id",
            &member,
            "--operation-id",
            OP_A,
            "--axis-point",
            "0,0,3000",
            "--axis-direction",
            "0,1,0",
            "--polygon",
            "-10,-10;-10,10;10,10;10,-10",
        ],
    );
    let net = f.volume_of(&cut, "element");
    assert!(
        (net - (holed - 120_000.0)).abs() <= 0.001,
        "{net} vs {holed}"
    );

    let records = at(at(&cut, &["element"]), &["hole_records"])
        .array()
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(text(&records[0], "kind"), "hole");
    assert_eq!(text(&records[1], "kind"), "polygon_cut");

    // A fresh process reopens and re-verifies the two records.
    let inspected = f.inspected();
    let members = at(&inspected, &["members"]).array().unwrap();
    assert_eq!(num(&members[0], &["quantities", "volume", "mm3"]), net,);

    // The oblique cut on its own member: a 100 x 50 slot through the solid section at 45
    // degrees in the section plane (the footprint's transverse band must stay inside both
    // crossed faces). Both crossed walls are parallel, so the certified removed volume is the
    // exact parallel-wall identity, slot area x 200 / cos(45 deg) = 1,414,213.5624 mm3.
    let second = f.ok(
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,2000,0",
            "--end",
            "6000,2000,0",
            "--profile-digest",
            &profile,
            "--new-id",
            "01b00000-0000-7000-8000-0000000000f5",
        ],
    );
    let other = text(at(&second, &["member"]), "element_id");
    let gross_other = member_volume(&f.inspected(), &other);
    let oblique = f.ok(
        "cut-by-polygon",
        &[
            "--id",
            &other,
            "--operation-id",
            OP_B,
            "--axis-point",
            "0,0,3000",
            "--axis-direction",
            "1,1,0",
            "--polygon",
            "-50,-25;-50,25;50,25;50,-25",
        ],
    );
    let removed = 5000.0 * 200.0 / (0.5f64.sqrt());
    let net_other = f.volume_of(&oblique, "element");
    assert!(
        (net_other - (gross_other - removed)).abs() <= 0.01,
        "{net_other} vs {gross_other}"
    );

    // The member form on a plate target is refused symmetrically.
    let plate = text(at(&f.plate(&[]), &["plate"]), "element_id");
    let kind = f.rejected(
        "cut-by-polygon",
        &[
            "--id",
            &plate,
            "--operation-id",
            "01b00000-0000-7000-8000-0000000000f6",
            "--axis-point",
            "0,0,0",
            "--axis-direction",
            "1,0,0",
            "--polygon",
            "100,100;100,200;200,200;200,100",
        ],
    );
    assert_eq!(kind, "INVALID_ARGUMENT");
}

#[test]
fn a_member_polygon_cut_slots_a_chs_and_undoes() {
    let f = Fixture::new();
    let profile = text(
        at(
            &f.ok(
                "define-profile",
                &["--family", "chs", "--d", "168", "--t", "5"],
            ),
            &["definition"],
        ),
        "digest",
    );
    let created = f.ok(
        "create-member",
        &[
            "--kind",
            "column",
            "--start",
            "0,0,0",
            "--end",
            "0,0,3000",
            "--profile-digest",
            &profile,
            "--new-id",
            "01b00000-0000-7000-8000-0000000000f4",
        ],
    );
    let member = text(at(&created, &["member"]), "element_id");
    let gross = first_member_volume(&f.inspected());

    // The slot lives in a personal draft (one member-hole record per tube: a faceted wall's
    // band walls keep the standing re-cut refusal of the G14.2 note, so the head stays gross
    // and the draft carries the one cut). The faceted wall strips are tilted against the
    // cutter, so the certified removed volume is the clip pieces' moment enclosure around the
    // naive area x 2 x t = 1440 mm3; the assertion proves material left through both walls of
    // one certified replay.
    let draft = f.draft();
    let cut = f.ok_in(
        &draft,
        "cut-by-polygon",
        &[
            "--id",
            &member,
            "--operation-id",
            OP_A,
            "--axis-point",
            "0,0,1500",
            "--axis-direction",
            "0,1,0",
            "--polygon",
            "-6,-6;-6,6;6,6;6,-6",
        ],
    );
    assert_eq!(text(&cut, "outcome"), "a13_applied");
    let net = f.volume_of(&cut, "element");
    assert!((gross - net - 1440.0).abs() <= 0.001, "{net} vs {gross}");
    let records = at(at(&cut, &["element"]), &["hole_records"])
        .array()
        .unwrap();
    assert_eq!(text(&records[0], "kind"), "polygon_cut");

    // The undo returns the gross tube; the redo reproduces the certified cut.
    f.history(&draft, "undo-workspace");
    let (_, undone) = f.run_in(&draft, "inspect-elements", &[]);
    assert!((first_member_volume(&undone) - gross).abs() <= 0.001);
    f.history(&draft, "redo-workspace");
    let (_, redone) = f.run_in(&draft, "inspect-elements", &[]);
    assert!((first_member_volume(&redone) - net).abs() <= 0.001);
}

#[test]
fn a_part_cut_slots_a_member_at_the_brace_angle_and_refuses_the_axial_tool() {
    let f = Fixture::new();
    // A square hollow section member along element X (200 x 200 x 10, exact walls), the
    // gross 7600 mm2 x 3000 mm.
    let profile = f.ok(
        "define-profile",
        &[
            "--family", "rhs", "--h", "200", "--b", "200", "--t", "10", "--r", "0",
        ],
    );
    let digest = text(at(&profile, &["definition"]), "digest");
    let created = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &digest,
            "--new-id",
            "01b00000-0000-7000-8000-0000000000e1",
        ],
    );
    let member = text(at(&created, &["member"]), "element_id");
    let gross = num(&created, &["member", "quantities", "volume", "mm3"]);
    assert!((gross - 22_800_000.0).abs() <= 1e-6, "{gross}");
    // The knife plate: a 40 x 40 contour with a 500 mm band, its normal leaning along the
    // member at 45 degrees (element (sqrt2/2, sqrt2/2, 0)), centred on the member axis at
    // mid-length. In the member frame the cut direction is (sqrt2/2, 0, sqrt2/2): it
    // crosses the two u-walls and runs parallel to the v-walls.
    let tool_created = f.tool_plate(&[
        "--width",
        "40",
        "--length",
        "40",
        "--thickness",
        "500",
        "--translation",
        "1500,0,28.284271247461902",
        "--orientation-xyzw",
        "-0.5,0.5,0,0.7071067811865476",
        "--new-id",
        "01b00000-0000-7000-8000-0000000000e2",
    ]);
    let tool = text(at(&tool_created, &["plate"]), "element_id");
    let cut = f.ok(
        "cut-by-part",
        &["--id", &member, "--operation-id", OP_A, "--tool", &tool],
    );
    // The net is the certified removed column: two wall crossings of
    // `A x t / |n . d|` = `1600 x 10 x sqrt2` each.
    let net = f.volume_of(&cut, "element");
    let expected = 22_800_000.0 - 32_000.0 * 2.0f64.sqrt();
    assert!((net - expected).abs() <= 1e-6, "{net} vs {expected}");
    // The record rides the member's own hole vocabulary as a part cut.
    let authored = f.authored_text(&cut);
    assert!(
        authored.contains("steel:mhole:kinds = [\"part_cut\"]"),
        "{authored}"
    );
    // A fresh process reopens and re-verifies.
    let inspected = f.inspected();
    let members = at(&inspected, &["members"]).array().unwrap();
    let reopened = members
        .iter()
        .find(|m| text(m, "element_id") == member)
        .unwrap();
    let reopened_volume = num(reopened, &["quantities", "volume", "mm3"]);
    assert!(
        (reopened_volume - expected).abs() <= 1e-6,
        "{reopened_volume}"
    );
    // The purely axial tool (its normal along the member axis) is the unsupported geometry.
    let axial = f.tool_plate(&[
        "--width",
        "40",
        "--length",
        "40",
        "--thickness",
        "500",
        "--translation",
        "-176.7766952966369,-176.7766952966369,1500",
        "--orientation-xyzw",
        "0,0.7071067811865476,0,0.7071067811865476",
        "--new-id",
        "01b00000-0000-7000-8000-0000000000e3",
    ]);
    let axial = text(at(&axial, &["plate"]), "element_id");
    let kind = f.rejected(
        "cut-by-part",
        &["--id", &member, "--operation-id", OP_B, "--tool", &axial],
    );
    assert_eq!(kind, "UNSUPPORTED_GEOMETRY");
    // The undo takes a fresh member's slot back: the head member already carries the
    // oblique slot, and a body cut obliquely once serves no further member-hole replay
    // (the documented kernel limit), so the draft cuts its own member and undoes that.
    let draft = f.draft();
    let fresh = f.ok_in(
        &draft,
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &digest,
            "--new-id",
            "01b00000-0000-7000-8000-0000000000e6",
        ],
    );
    let fresh_member = text(at(&fresh, &["member"]), "element_id");
    f.ok_in(
        &draft,
        "cut-by-part",
        &[
            "--id",
            &fresh_member,
            "--operation-id",
            OP_B,
            "--tool",
            &tool,
        ],
    );
    f.history(&draft, "undo-workspace");
    let (_, undone) = f.run_in(&draft, "inspect-elements", &[]);
    let members = at(&undone, &["members"]).array().unwrap();
    let restored = members
        .iter()
        .find(|m| text(m, "element_id") == fresh_member)
        .unwrap();
    let restored_volume = num(restored, &["quantities", "volume", "mm3"]);
    assert!((restored_volume - gross).abs() <= 1e-6, "{restored_volume}");
}

#[test]
fn an_oblique_part_cut_leans_against_a_plate_and_keeps_the_volume() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    // A tool leaning 45 degrees against the plate: its normal (sqrt2/2, 0, sqrt2/2), the
    // band still passing both faces, the mapped openings inside the outline.
    let tool_created = f.tool_plate(&[
        "--width",
        "100",
        "--length",
        "100",
        "--thickness",
        "30",
        "--translation",
        "454.0380592228744,200,29.748737341529165",
        "--orientation-xyzw",
        "0,0.3826834323650898,0,0.9238795325112867",
        "--new-id",
        "01b00000-0000-7000-8000-0000000000e4",
    ]);
    let tool = text(at(&tool_created, &["plate"]), "element_id");
    let cut = f.ok(
        "cut-by-part",
        &["--id", &element, "--operation-id", OP_A, "--tool", &tool],
    );
    // The tilted prism's horizontal section is the mapped outline at every height: it
    // removes `area x thickness` where the area is the along-axis projection's, stretched
    // by the lean.
    let net = f.volume_of(&cut, "element");
    let expected = 5_000_000.0 - 100_000.0 * 2.0f64.sqrt();
    assert!((net - expected).abs() <= 1e-6, "{net}");
    // The record stores the leaning axis and the part_cut kind.
    let authored = f.authored_text(&cut);
    assert!(
        authored.contains("steel:hole:kinds = [\"part_cut\"]"),
        "{authored}"
    );
    assert!(authored.contains("steel:hole:axisDirections"), "{authored}");
    // A steep tool lying in the plate's plane never crosses a face.
    let flat = f.tool_plate(&[
        "--width",
        "100",
        "--length",
        "100",
        "--thickness",
        "30",
        "--translation",
        "400,200,5",
        "--orientation-xyzw",
        "0,1,0,0",
        "--new-id",
        "01b00000-0000-7000-8000-0000000000e5",
    ]);
    let flat = text(at(&flat, &["plate"]), "element_id");
    let kind = f.rejected(
        "cut-by-part",
        &["--id", &element, "--operation-id", OP_B, "--tool", &flat],
    );
    assert_eq!(kind, "UNSUPPORTED_GEOMETRY");
}
