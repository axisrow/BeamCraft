// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected values are closed forms.

//! P07 end to end: durable undo and redo of a workspace draft. Every step is one new verified
//! revision whose layers are byte for byte those of the restored state, so identity, mesh and
//! stored quantities come back together; the stacks survive a process restart (every call here is
//! a fresh process) and a crash between an intent and its commit.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::Path;
use std::process::Command;

const USER: &str = "01890a5d-ac96-774b-b9cc-aaaaaaaaaaaa";

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("p07-tests"),
        };
        fs::create_dir(fixture.shared.root.join("published")).unwrap();
        fixture
    }
    fn dir(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn run(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.run(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }
    fn code(&self, args: &[&str]) -> (i32, String) {
        let (code, body) = self.run(args);
        assert_ne!(code, 0, "{args:?}: {body:?}");
        (code, text(&body, "code"))
    }
    /// A published store with one plate (hole, steel) and a forked workspace.
    fn forked(&self) -> (String, String, String) {
        let published = self.dir("published");
        let steel = text(
            at(
                &self.ok(&[
                    "define-material",
                    "--store",
                    &published,
                    "--density",
                    "7850",
                ]),
                &["definition"],
            ),
            "digest",
        );
        let created = self.ok(&[
            "create-plate",
            "--store",
            &published,
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--material-digest",
            &steel,
            "--hole-rect",
            "100,100,100,100",
        ]);
        let id = text(at(&created, &["plate"]), "element_id");
        let dir = self.dir("ws");
        self.ok(&[
            "create-workspace",
            "--store",
            &published,
            "--workspace-dir",
            &dir,
            "--user-id",
            USER,
        ]);
        (published, dir, id)
    }
    fn edit(&self, dir: &str, args: &[&str]) -> Json {
        let mut all = vec![args[0], "--store", dir];
        all.extend_from_slice(&args[1..]);
        self.ok(&all)
    }
    fn history(&self, dir: &str) -> Json {
        self.ok(&["inspect-workspace-history", "--workspace-dir", dir])
    }
    fn layers(&self, dir: &str) -> Vec<(String, String)> {
        at(&self.history(dir), &["draft_head", "manifest", "layers"])
            .array()
            .unwrap()
            .iter()
            .map(|l| (text(l, "name"), text(l, "digest")))
            .collect()
    }
    fn units(&self, dir: &str) -> (String, String) {
        let h = self.history(dir);
        (text(&h, "undo_units"), text(&h, "redo_units"))
    }
    fn plates(&self, dir: &str) -> Vec<Json> {
        at(&self.ok(&["inspect-elements", "--store", dir]), &["plates"])
            .array()
            .unwrap()
            .to_vec()
    }
}

#[test]
fn undo_after_a_rebase_boundary_restores_only_later_edits_and_keeps_the_combined_basis() {
    use beamcraft_cli::workspace::{WorkspaceState, read_meta};
    let f = Fixture::new();
    let (published, dir, id) = f.forked();
    let accepted = f.ok(&["open-revision", "--store", &published]);
    f.edit(
        &dir,
        &["update-element", "--id", &id, "--name", "Original draft"],
    );
    f.edit(
        &dir,
        &["update-element", "--id", &id, "--name", "Combined basis"],
    );
    // Isolate the undo boundary with real revisions; runtime merge is separate acceptance.
    let old = read_meta(Path::new(&dir)).unwrap();
    let mut rebased = old.clone();
    rebased.rebase_sequence = 3;
    WorkspaceState::open(Path::new(&dir))
        .unwrap()
        .compare_and_swap(Some(&old), &rebased)
        .unwrap();
    let layers = f.layers(&dir);
    assert_eq!(f.units(&dir), ("0".into(), "0".into()));
    assert_eq!(
        f.code(&["undo-workspace", "--workspace-dir", &dir]),
        (2, "INVALID_ARGUMENT".into())
    );
    assert_eq!(f.layers(&dir), layers);
    f.edit(
        &dir,
        &["update-element", "--id", &id, "--name", "Later local edit"],
    );
    f.ok(&["undo-workspace", "--workspace-dir", &dir]);
    assert_eq!(text(&f.plates(&dir)[0], "name"), "Combined basis");
    assert_eq!(f.units(&dir), ("0".into(), "1".into()));
    assert_eq!(
        f.code(&["undo-workspace", "--workspace-dir", &dir]),
        (2, "INVALID_ARGUMENT".into())
    );
    f.ok(&["redo-workspace", "--workspace-dir", &dir]);
    assert_eq!(text(&f.plates(&dir)[0], "name"), "Later local edit");
    assert_eq!(read_meta(Path::new(&dir)).unwrap().rebase_sequence, 3);
    assert_eq!(f.ok(&["open-revision", "--store", &published]), accepted);
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn find<'a>(plates: &'a [Json], id: &str) -> &'a Json {
    plates.iter().find(|p| text(p, "element_id") == id).unwrap()
}

#[test]
fn undo_and_redo_restore_a_copy_and_a_change_atomically() {
    let f = Fixture::new();
    let (published, dir, id) = f.forked();
    let initial = f.layers(&dir);
    assert_eq!(f.units(&dir), ("0".into(), "0".into()));

    // A copy is one unit: undo removes it and redo brings back the same element.
    let copied = f.edit(&dir, &["copy-element", "--id", &id, "--name", "Copy"]);
    let copy_id = text(at(&copied, &["element"]), "element_id");
    let copy_mesh = text(at(&copied, &["element"]), "mesh_digest");
    let after_copy = f.layers(&dir);
    assert_ne!(after_copy, initial);
    assert_eq!(f.units(&dir), ("1".into(), "0".into()));

    let undone = f.ok(&["undo-workspace", "--workspace-dir", &dir]);
    assert_eq!(text(&undone, "outcome"), "workspace_undone");
    assert_eq!(f.plates(&dir).len(), 1);
    assert_eq!(f.layers(&dir), initial);
    assert_eq!(f.units(&dir), ("0".into(), "1".into()));

    let redone = f.ok(&["redo-workspace", "--workspace-dir", &dir]);
    assert_eq!(text(&redone, "outcome"), "workspace_redone");
    let plates = f.plates(&dir);
    assert_eq!(plates.len(), 2);
    let copy = find(&plates, &copy_id);
    assert_eq!(text(copy, "mesh_digest"), copy_mesh);
    assert_eq!(text(copy, "name"), "Copy");
    assert_eq!(f.layers(&dir), after_copy);
    assert_eq!(f.units(&dir), ("1".into(), "0".into()));

    // A shape change: undo restores the thickness, the volume, the mass and their freshness.
    let before_change = f.layers(&dir);
    f.edit(&dir, &["update-element", "--id", &id, "--thickness", "20"]);
    let changed = f.plates(&dir);
    assert_eq!(
        num(find(&changed, &id), &["quantities", "volume", "mm3"]),
        9_800_000.0
    );
    f.ok(&["undo-workspace", "--workspace-dir", &dir]);
    let restored = f.plates(&dir);
    let source = find(&restored, &id);
    assert_eq!(num(source, &["quantities", "volume", "mm3"]), 4_900_000.0);
    assert_eq!(num(source, &["quantities", "mass", "kg"]), 38.465);
    for quantity in ["volume", "mass"] {
        assert_eq!(
            text(at(source, &["stored_quantities", quantity]), "freshness"),
            "current"
        );
    }
    assert_eq!(f.layers(&dir), before_change);

    // A new edit after an undo discards the redo stack.
    assert_eq!(f.units(&dir), ("1".into(), "1".into()));
    f.edit(&dir, &["update-element", "--id", &id, "--name", "Renamed"]);
    assert_eq!(f.units(&dir), ("2".into(), "0".into()));
    let (_, code) = f.code(&["redo-workspace", "--workspace-dir", &dir]);
    assert_eq!(code, "INVALID_ARGUMENT");

    // Undo reaches back to the fork and not before.
    for _ in 0..2 {
        f.ok(&["undo-workspace", "--workspace-dir", &dir]);
    }
    let (_, code) = f.code(&["undo-workspace", "--workspace-dir", &dir]);
    assert_eq!(code, "INVALID_ARGUMENT");
    assert_eq!(f.units(&dir), ("0".into(), "2".into()));
    assert_eq!(f.layers(&dir), initial);

    // The published store was never touched by any of this.
    let published_plates = f.ok(&["inspect-elements", "--store", &published]);
    assert_eq!(at(&published_plates, &["plates"]).array().unwrap().len(), 1);
}

#[test]
fn an_undone_draft_publishes_the_restored_state() {
    let f = Fixture::new();
    let (published, dir, id) = f.forked();
    f.edit(&dir, &["update-element", "--id", &id, "--name", "Draft"]);
    f.ok(&["undo-workspace", "--workspace-dir", &dir]);
    let sent = f.ok(&[
        "publish-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &dir,
    ]);
    assert_eq!(text(&sent, "outcome"), "workspace_published");
    let plates = at(
        &f.ok(&["inspect-elements", "--store", &published]),
        &["plates"],
    )
    .array()
    .unwrap()
    .to_vec();
    assert_eq!(plates.len(), 1);
    assert_ne!(text(&plates[0], "name"), "Draft");
}

#[test]
fn replay_reuse_and_an_interrupted_step_are_handled() {
    let f = Fixture::new();
    let (_, dir, id) = f.forked();
    f.edit(&dir, &["update-element", "--id", &id, "--name", "One"]);
    f.edit(&dir, &["update-element", "--id", &id, "--name", "Two"]);

    let step = ["--op-id", "01890a5d-ac96-774b-b9cc-0000000000a1"];
    let mut undo = vec!["undo-workspace", "--workspace-dir", &dir];
    undo.extend_from_slice(&step);
    let first = f.ok(&undo);
    assert_eq!(text(&first, "outcome"), "workspace_undone");
    let replay = f.ok(&undo);
    assert_eq!(text(&replay, "outcome"), "workspace_undo_replayed");
    assert_eq!(f.units(&dir), ("1".into(), "1".into()));
    let mut redo = vec!["redo-workspace", "--workspace-dir", &dir];
    redo.extend_from_slice(&step);
    assert_eq!(f.code(&redo).1, "OP_ID_REUSE");

    // A crash after the intent was written and before the commit: the intent alone changes
    // nothing, and the same operation finishes it.
    let head = text(at(&f.history(&dir), &["draft_head"]), "revision_id");
    let revision = "01890a5d-ac96-774b-b9cc-0000000000b2";
    let op = "01890a5d-ac96-774b-b9cc-0000000000b3";
    let log = Path::new(&dir).join("undo.log");
    let mut text_log = fs::read_to_string(&log).unwrap();
    text_log.push_str(&format!("undo {revision} {op} {head}\n"));
    fs::write(&log, &text_log).unwrap();
    assert_eq!(f.units(&dir), ("1".into(), "1".into()));
    let finished = f.ok(&["undo-workspace", "--workspace-dir", &dir, "--op-id", op]);
    assert_eq!(text(&finished, "outcome"), "workspace_undone");
    assert_eq!(text(at(&finished, &["revision"]), "revision_id"), revision);
    assert_eq!(f.units(&dir), ("0".into(), "2".into()));

    // An intent recorded against another head cannot be resumed.
    let stale_op = "01890a5d-ac96-774b-b9cc-0000000000c4";
    let mut text_log = fs::read_to_string(&log).unwrap();
    text_log.push_str(&format!(
        "redo 01890a5d-ac96-774b-b9cc-0000000000c5 {stale_op} 01890a5d-ac96-774b-b9cc-0000000000c6\n"
    ));
    fs::write(&log, &text_log).unwrap();
    let (_, code) = f.code(&[
        "redo-workspace",
        "--workspace-dir",
        &dir,
        "--op-id",
        stale_op,
    ]);
    assert_eq!(code, "OP_ID_REUSE");
    assert_eq!(f.units(&dir), ("0".into(), "2".into()));
}

#[test]
fn bad_inputs_are_rejected_without_writing() {
    let f = Fixture::new();
    let (_, dir, _) = f.forked();
    assert_eq!(f.code(&["undo-workspace"]).1, "INVALID_ARGUMENT");
    assert_eq!(
        f.code(&["undo-workspace", "--workspace-dir", &dir, "--op-id", "x"])
            .1,
        "INVALID_ID"
    );
    assert_eq!(
        f.code(&["undo-workspace", "--workspace-dir", &dir, "--store", &dir])
            .1,
        "INVALID_ARGUMENT"
    );
    // A directory that is not a workspace, and a damaged log, are backend errors.
    let empty = f.dir("empty");
    fs::create_dir(&empty).unwrap();
    assert_eq!(
        f.code(&["inspect-workspace-history", "--workspace-dir", &empty])
            .0,
        3
    );
    let log = Path::new(&dir).join("undo.log");
    fs::write(&log, "garbage\n").unwrap();
    assert_eq!(
        f.code(&["inspect-workspace-history", "--workspace-dir", &dir])
            .0,
        3
    );
    // An intent whose revision was never committed is ignored.
    fs::write(
        &log,
        "undo 01890a5d-ac96-774b-b9cc-0000000000b2 01890a5d-ac96-774b-b9cc-0000000000b3 \
         01890a5d-ac96-774b-b9cc-0000000000b4\n",
    )
    .unwrap();
    assert_eq!(f.units(&dir), ("0".into(), "0".into()));
}

#[test]
fn undo_restores_a_move_and_a_deleted_element() {
    let f = Fixture::new();
    let (_, dir, id) = f.forked();
    let initial = f.layers(&dir);
    f.edit(
        &dir,
        &["update-element", "--id", &id, "--translation", "5,6,7"],
    );
    let moved = f.layers(&dir);
    f.edit(&dir, &["delete-element", "--id", &id]);
    assert!(f.plates(&dir).is_empty());

    f.ok(&["undo-workspace", "--workspace-dir", &dir]);
    let plates = f.plates(&dir);
    assert_eq!(plates.len(), 1);
    assert_eq!(text(&plates[0], "element_id"), id);
    assert_eq!(f.layers(&dir), moved);
    f.ok(&["undo-workspace", "--workspace-dir", &dir]);
    assert_eq!(f.layers(&dir), initial);
    f.ok(&["redo-workspace", "--workspace-dir", &dir]);
    f.ok(&["redo-workspace", "--workspace-dir", &dir]);
    assert!(f.plates(&dir).is_empty());
}
