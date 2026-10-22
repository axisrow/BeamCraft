// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, members and assemblies, MPL-2.0. The independent
// `beamcraft-usd-oracle` process is used as the external test reader: it is not a dependency
// and shares no code with the layer engine.

//! A09 end to end through the real CLI binary: `numbering-approve` stores the approved marks as
//! authored data through the normal revision path, `numbering-status` and `numbering-preview` read
//! them back, a stale token, overlapping series and an empty model are refused with nothing
//! written, fabrication edits make the approval stale while a move and a rename do not, and an
//! undo restores the earlier marks.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const USER: &str = "01890a5d-ac96-774b-b9cc-aaaaaaaaaaaa";

struct Fixture {
    root: PathBuf,
}

struct Scene {
    plates: Vec<String>,
    beams: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a09-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        Self { root }
    }
    fn store(&self) -> String {
        self.root.join("store").to_str().unwrap().to_owned()
    }
    fn dir(&self, name: &str) -> String {
        self.root.join(name).to_str().unwrap().to_owned()
    }
    fn run_in(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        let mut args = vec![command, "--store", store];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.run_in(&self.store(), command, extra)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn ok_in(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run_in(store, command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    /// Runs a command that must be refused and returns its code; no revision may be written.
    fn refused(&self, command: &str, extra: &[&str]) -> String {
        let before = self.layers();
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        assert_eq!(text(&body, "outcome"), "rejected");
        assert_eq!(self.layers(), before, "{command} {extra:?} wrote");
        text(&body, "code")
    }
    fn preview(&self, extra: &[&str]) -> Json {
        self.ok("numbering-preview", extra)
    }
    fn token(&self, extra: &[&str]) -> String {
        text(&self.preview(extra), "preview_token")
    }
    fn approve(&self, token: &str, extra: &[&str]) -> Json {
        let mut args = vec!["--preview-token", token];
        args.extend_from_slice(extra);
        self.ok("numbering-approve", &args)
    }
    fn status(&self) -> Json {
        self.ok("numbering-status", &[])
    }
    fn marks(&self) -> BTreeMap<String, String> {
        list(&self.status(), "marks")
            .iter()
            .map(|m| (text(m, "id"), text(m, "mark")))
            .collect()
    }
    fn state(&self) -> String {
        text(at(&self.status(), &["approval"]), "state")
    }
    fn layers(&self) -> Vec<(String, String)> {
        match self.run("revision-open", &[]) {
            (0, body) => list(&body, "layers")
                .iter()
                .map(|l| (text(l, "name"), text(l, "digest")))
                .collect(),
            _ => Vec::new(),
        }
    }
    fn head(&self) -> String {
        text(&self.ok("revision-open", &[]), "revision_id")
    }
    fn plate(&self, material: &str, thickness: &str, extra: &[&str]) -> String {
        let mut args = vec![
            "--width",
            "400",
            "--length",
            "300",
            "--thickness",
            thickness,
            "--material-digest",
            material,
        ];
        args.extend_from_slice(extra);
        text(
            at(&self.ok("create-plate", &args), &["plate"]),
            "element_id",
        )
    }
    fn beam(&self, profile: &str, material: &str, end: &str, translation: &str) -> String {
        text(
            at(
                &self.ok(
                    "create-member",
                    &[
                        "--start",
                        "0,0,0",
                        "--end",
                        end,
                        "--profile-digest",
                        profile,
                        "--material-digest",
                        material,
                        "--translation",
                        translation,
                    ],
                ),
                &["member"],
            ),
            "element_id",
        )
    }
    /// Plates 0, 1 (identical, one moved), 2 (other material), 3 (a hole), 4 (thicker); beams 0, 1
    /// (identical, one moved), 2 (other length).
    fn scene(&self) -> Scene {
        let material = |name: &str| {
            text(
                at(
                    &self.ok(
                        "define-material",
                        &["--density", "7850", "--name", name, "--grade", name],
                    ),
                    &["definition"],
                ),
                "digest",
            )
        };
        let steel = material("S355");
        let other_steel = material("S275");
        let profile = text(
            at(
                &self.ok("define-profile", &["--width", "100", "--height", "50"]),
                &["definition"],
            ),
            "digest",
        );
        let plates = vec![
            self.plate(&steel, "20", &[]),
            self.plate(&steel, "20", &["--translation", "1000,0,0"]),
            self.plate(&other_steel, "20", &[]),
            self.plate(&steel, "20", &["--hole-round", "100,100,40"]),
            self.plate(&steel, "25", &[]),
        ];
        let beams = vec![
            self.beam(&profile, &steel, "3000,0,0", "0,0,0"),
            self.beam(&profile, &steel, "3000,0,0", "0,500,0"),
            self.beam(&profile, &steel, "3500,0,0", "0,1000,0"),
        ];
        Scene { plates, beams }
    }
    /// Commits a head-based revision with edited layer text and no content verification, the way
    /// a layer edited outside BeamCraft would arrive through `revision-commit`.
    fn tamper(&self, edit: impl Fn(&str, String) -> String) {
        let store = RevisionStore::new(BlobStore::new(self.root.join("store")).unwrap());
        let head = store.open().unwrap();
        let content = head.manifest().content();
        let mut ids = IdSource::system();
        let mut entries = Vec::new();
        for entry in content.entries() {
            let original = String::from_utf8(head.layer_bytes(entry.name()).unwrap()).unwrap();
            let edited = edit(entry.name(), original);
            store.blobs().publish(edited.as_bytes()).unwrap();
            entries.push(
                LayerEntry::from_bytes(entry.name(), entry.role(), edited.as_bytes()).unwrap(),
            );
        }
        let draft = RevisionDraft {
            project: content.project().clone(),
            workspace: content.workspace().clone(),
            revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            workspace_generation: content.workspace_generation() + 1,
            schema: content.schema(),
            root_layer: content.root_layer().to_owned(),
            entries,
        };
        store
            .commit_with(
                &draft,
                &ExpectedBase::Revision(head.info().revision().clone()),
                None,
                &mut NoFault,
            )
            .unwrap();
    }
    fn authored_text(&self) -> String {
        let store = RevisionStore::new(BlobStore::new(self.root.join("store")).unwrap());
        let head = store.open().unwrap();
        String::from_utf8(head.layer_bytes("authored.usda").unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}
fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
fn number(json: &Json, key: &str) -> u64 {
    json.get(key).unwrap().unsigned().unwrap()
}
fn is_null(json: &Json, key: &str) -> bool {
    matches!(json.get(key), Ok(Json::Null))
}
fn strings(json: &Json, key: &str) -> Vec<String> {
    list(json, key)
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect()
}

#[test]
fn approval_stores_one_mark_per_group_as_authored_data_and_reads_back() {
    let f = Fixture::new();
    let s = f.scene();
    // Two assemblies with the same content: A-1 for both.
    let mut frames = Vec::new();
    for (name, plate, beam) in [("Frame 1", 0, 0), ("Frame 2", 1, 1)] {
        let id = text(&f.ok("create-assembly", &["--name", name]), "element_id");
        for part in [&s.plates[plate], &s.beams[beam]] {
            f.ok("assembly-attach", &["--id", part, "--assembly-id", &id]);
        }
        frames.push(id);
    }
    assert_eq!(f.state(), "not_approved");
    let inspected = f.ok("plate-inspect", &[]);
    let derived_before = f
        .layers()
        .into_iter()
        .find(|(n, _)| n == "derived.usda")
        .unwrap();
    let before_head = f.head();
    let token = f.token(&[]);
    let approved = f.approve(&token, &[]);
    assert_eq!(text(&approved, "outcome"), "numbering_approved");
    // 5 plates, 3 beams and 2 assemblies carry a mark; 4 plate, 2 beam and 1 assembly groups.
    assert_eq!(number(&approved, "marks"), 10);
    assert_eq!(number(&approved, "groups"), 7);
    assert_ne!(text(&approved, "revision_id"), before_head);
    // Only the authored layer changes: a mark touches no derived body.
    assert_eq!(strings(&approved, "changed_layers"), ["authored.usda"]);
    let derived_after = f
        .layers()
        .into_iter()
        .find(|(n, _)| n == "derived.usda")
        .unwrap();
    assert_eq!(derived_after, derived_before);
    let footprint = at(&approved, &["footprint"]);
    assert_eq!(number(footprint, "writes"), 10);
    assert_eq!(text(at(&approved, &["approval"]), "state"), "current");
    assert_eq!(
        text(at(&approved, &["approval"]), "approved_token"),
        token,
        "the approval records the token it approved"
    );

    // Equivalent parts share a mark; the others differ.
    let marks = f.marks();
    let mark = |id: &String| marks.get(id).unwrap().as_str();
    assert_eq!(mark(&s.plates[0]), "PL1");
    assert_eq!(mark(&s.plates[1]), "PL1");
    assert_eq!(mark(&s.plates[2]), "PL2");
    assert_eq!(mark(&s.plates[3]), "PL3");
    assert_eq!(mark(&s.plates[4]), "PL4");
    assert_eq!(mark(&s.beams[0]), "B1");
    assert_eq!(mark(&s.beams[1]), "B1");
    assert_eq!(mark(&s.beams[2]), "B2");
    assert_eq!(mark(&frames[0]), "A-1");
    assert_eq!(mark(&frames[1]), "A-1");
    let distinct: std::collections::BTreeSet<_> = marks.values().collect();
    assert_eq!(distinct.len(), 7);

    // The status lists series and number separately, for a reader that composes its own text.
    let status = f.status();
    let first = list(&status, "marks")
        .iter()
        .find(|m| text(m, "id") == s.plates[4])
        .unwrap();
    assert_eq!(text(first, "series"), "PL");
    assert_eq!(number(first, "number"), 4);
    assert_eq!(text(first, "kind"), "plate");

    // The preview reads the approval back: current, and every group holds its proposed position.
    let p = f.preview(&["--previous", &token]);
    assert!(matches!(p.get("stale").unwrap(), Json::Bool(false)));
    assert_eq!(text(at(&p, &["approval"]), "state"), "current");
    assert_eq!(number(at(&p, &["approval"]), "marked"), 10);
    assert!(
        list(&p, "groups")
            .iter()
            .all(|g| text(g, "stored") == "same")
    );
    assert_eq!(
        text(at(&p, &["approval", "approved_settings"]), "plate_series"),
        "PL:1"
    );

    // The marks are three plain properties of the element prim in the authored layer.
    let authored = f.authored_text();
    assert_eq!(
        authored
            .matches("custom string steel:numbering:series")
            .count(),
        10
    );
    assert_eq!(
        authored
            .matches("custom int steel:numbering:number")
            .count(),
        10
    );
    assert_eq!(
        authored
            .matches("custom string steel:numbering:basis")
            .count(),
        10
    );
    // plate-inspect still lists the same plates with the same bodies.
    let after = f.ok("plate-inspect", &[]);
    assert_eq!(
        list(&after, "plates").len(),
        list(&inspected, "plates").len()
    );
    for (a, b) in list(&after, "plates")
        .iter()
        .zip(list(&inspected, "plates"))
    {
        assert_eq!(text(a, "mesh_digest"), text(b, "mesh_digest"));
    }
}

#[test]
fn a_stale_token_overlapping_series_and_an_empty_model_are_refused_with_nothing_written() {
    let f = Fixture::new();
    // A store with no revision at all, then one with only a material.
    assert_eq!(
        f.run("numbering-approve", &["--preview-token", &"0".repeat(64)])
            .0,
        2
    );
    f.ok("define-material", &["--density", "7850"]);
    let empty_token = f.token(&[]);
    assert_eq!(
        f.refused("numbering-approve", &["--preview-token", &empty_token]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.state(), "not_approved");

    let s = f.scene();
    let token = f.token(&[]);
    // Another part edit after the preview: the token is no longer the proposal.
    f.ok(
        "update-element",
        &["--id", &s.plates[1], "--thickness", "30"],
    );
    assert_eq!(
        f.refused("numbering-approve", &["--preview-token", &token]),
        "STALE_WORKSPACE"
    );
    assert_eq!(f.state(), "not_approved");
    // The token of other settings is not the proposal either.
    let current = f.token(&[]);
    assert_eq!(
        f.refused(
            "numbering-approve",
            &["--preview-token", &current, "--compare-name", "true"]
        ),
        "STALE_WORKSPACE"
    );
    // Overlapping series: the beams would take the plates' positions.
    let overlap = ["--beam-series", "PL:1"];
    let overlapping = f.token(&overlap);
    assert_eq!(
        f.refused(
            "numbering-approve",
            &["--preview-token", &overlapping, "--beam-series", "PL:1"]
        ),
        "INVALID_ARGUMENT"
    );
    // Series values the engine refuses.
    for bad in ["--plate-series", "--assembly-series"] {
        assert_eq!(
            f.refused(
                "numbering-approve",
                &["--preview-token", &current, bad, "TOOLONG_PREFIX_OVER_16:1"]
            ),
            "INVALID_ARGUMENT"
        );
    }
    assert_eq!(f.state(), "not_approved");
    // The proposal that is still the proposal is approved.
    assert_eq!(
        text(&f.approve(&current, &[]), "outcome"),
        "numbering_approved"
    );
}

#[test]
fn approving_again_is_unchanged_and_the_same_operation_replays() {
    let f = Fixture::new();
    let s = f.scene();
    let token = f.token(&[]);
    let op = "0199e7a0-0000-7000-8000-00000000a091";
    let first = f.approve(&token, &["--op-id", op]);
    assert_eq!(text(&first, "outcome"), "numbering_approved");
    let head = f.head();
    let marks = f.marks();
    // The same operation id with the same command is a replay of the same revision.
    let again = f.approve(&token, &["--op-id", op]);
    assert_eq!(text(&again, "outcome"), "numbering_approved_replayed");
    assert_eq!(f.head(), head);
    assert_eq!(
        text(at(&again, &["revision"]), "revision_id"),
        text(at(&first, &["revision"]), "revision_id")
    );
    // The same operation id with other settings is reuse.
    let other = f.token(&["--plate-series", "P:10"]);
    assert_eq!(
        f.refused(
            "numbering-approve",
            &[
                "--preview-token",
                &other,
                "--plate-series",
                "P:10",
                "--op-id",
                op
            ]
        ),
        "OP_ID_REUSE"
    );
    // A new operation id over the same approved numbering writes nothing and renumbers nothing.
    let layers = f.layers();
    let unchanged = f.approve(&token, &[]);
    assert_eq!(text(&unchanged, "outcome"), "numbering_approved_unchanged");
    assert!(is_null(&unchanged, "footprint"));
    assert!(strings(&unchanged, "changed_layers").is_empty());
    assert_eq!(
        (f.head(), f.layers(), f.marks()),
        (head.clone(), layers, marks.clone())
    );
    // A move changes nothing that approving reads: still unchanged.
    f.ok(
        "update-element",
        &["--id", &s.plates[0], "--translation", "5,5,5"],
    );
    let after_move = f.token(&[]);
    assert_eq!(after_move, token);
    assert_eq!(
        text(&f.approve(&after_move, &[]), "outcome"),
        "numbering_approved_unchanged"
    );
    assert_eq!(f.marks(), marks);
    // The replay of the first operation still names its own revision after later edits.
    let replay = f.approve(&token, &["--op-id", op]);
    assert_eq!(text(&replay, "outcome"), "numbering_approved_replayed");
}

#[test]
fn fabrication_changes_make_the_approval_stale_and_a_move_or_a_rename_do_not() {
    let f = Fixture::new();
    let s = f.scene();
    let token = f.token(&[]);
    f.approve(&token, &[]);
    assert_eq!(f.state(), "current");
    // A move, a rename and a read-only view of the model do not.
    f.ok(
        "update-element",
        &["--id", &s.plates[0], "--translation", "5,5,5"],
    );
    f.ok(
        "update-element",
        &["--id", &s.plates[1], "--name", "Renamed"],
    );
    f.ok("plate-inspect", &[]);
    f.preview(&["--tolerance", "5"]);
    assert_eq!(f.state(), "current");
    let marks = f.marks();

    // A thickness edit does, and the report names the part.
    f.ok(
        "update-element",
        &["--id", &s.plates[1], "--thickness", "30"],
    );
    let status = f.status();
    let approval = at(&status, &["approval"]);
    assert_eq!(text(approval, "state"), "stale");
    assert!(!is_null(approval, "reason"));
    assert_eq!(strings(approval, "differing_ids")[0], s.plates[1]);
    // The preview reports the same, and the stored marks are still there to read.
    let p = f.preview(&[]);
    assert_eq!(text(at(&p, &["approval"]), "state"), "stale");
    assert_eq!(f.marks(), marks);
    // Re-approving renumbers from the rules: the part that left its group has a mark of its own
    // and the part that stayed keeps PL1.
    let renewed = f.token(&[]);
    assert_ne!(renewed, token);
    f.approve(&renewed, &[]);
    assert_eq!(f.state(), "current");
    let marks = f.marks();
    assert_eq!(marks[&s.plates[0]], "PL1");
    assert_ne!(marks[&s.plates[1]], marks[&s.plates[0]]);

    // A material edit, a copy and a delete each make it stale too.
    let other_steel = text(
        at(
            &f.ok("define-material", &["--density", "7850", "--name", "S460"]),
            &["definition"],
        ),
        "digest",
    );
    let renewed = f.token(&[]);
    f.approve(&renewed, &[]);
    f.ok(
        "update-element",
        &["--id", &s.beams[0], "--material-digest", &other_steel],
    );
    assert_eq!(f.state(), "stale");
    f.approve(&f.token(&[]), &[]);
    assert_eq!(f.state(), "current");
    let copy = f.ok("copy-element", &["--id", &s.beams[2]]);
    assert_eq!(f.state(), "stale");
    assert_eq!(number(at(&f.status(), &["approval"]), "unmarked"), 1);
    f.approve(&f.token(&[]), &[]);
    // The copy joined the group of its source and renumbered nothing.
    let marks = f.marks();
    let copy_id = text(at(&copy, &["element"]), "element_id");
    assert_eq!(marks[&copy_id], marks[&s.beams[2]]);
    f.ok("delete-element", &["--id", &s.beams[2]]);
    assert_eq!(f.state(), "stale");
}

#[test]
fn an_undo_of_an_approval_restores_the_earlier_marks() {
    let f = Fixture::new();
    let published = f.store();
    let steel = text(
        at(
            &f.ok("define-material", &["--density", "7850"]),
            &["definition"],
        ),
        "digest",
    );
    for _ in 0..2 {
        f.plate(&steel, "10", &[]);
    }
    let dir = f.dir("ws");
    f.ok_in(
        &published,
        "workspace-create",
        &["--workspace-dir", &dir, "--user-id", USER],
    );
    let history = |op: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([op, "--workspace-dir", &dir])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{op}");
        beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap()
    };
    let layers = || {
        list(
            at(&history("workspace-history"), &["draft_head", "manifest"]),
            "layers",
        )
        .iter()
        .map(|l| (text(l, "name"), text(l, "digest")))
        .collect::<Vec<_>>()
    };
    let marks = || -> BTreeMap<String, String> {
        list(&f.ok_in(&dir, "numbering-status", &[]), "marks")
            .iter()
            .map(|m| (text(m, "id"), text(m, "mark")))
            .collect()
    };
    let before = layers();
    let token = text(&f.ok_in(&dir, "numbering-preview", &[]), "preview_token");
    f.ok_in(&dir, "numbering-approve", &["--preview-token", &token]);
    let approved = layers();
    assert_ne!(approved, before);
    let first = marks();
    assert_eq!(first.len(), 2);
    assert!(first.values().all(|m| m == "PL1"));

    history("workspace-undo");
    assert_eq!(layers(), before, "undo restores the layers byte for byte");
    assert!(marks().is_empty());
    let undone = f.ok_in(&dir, "numbering-status", &[]);
    assert_eq!(text(at(&undone, &["approval"]), "state"), "not_approved");
    history("workspace-redo");
    assert_eq!(layers(), approved);
    assert_eq!(marks(), first);
    // The published store never saw the approval.
    let published_status = f.ok_in(&published, "numbering-status", &[]);
    assert_eq!(number(&published_status, "count"), 0);
}

#[test]
fn damaged_and_oversized_input_is_refused() {
    let f = Fixture::new();
    f.scene();
    let token = f.token(&[]);
    f.approve(&token, &[]);
    assert_eq!(f.state(), "current");
    // Bad arguments.
    for args in [
        vec!["--preview-token", "abc"],
        vec!["--preview-token", &"G".repeat(64)],
        vec!["--preview-token", &"a".repeat(4097)],
        vec!["--preview-token", &token, "--preview-token", &token],
        vec!["--preview-token", &token, "--previous", &token],
        vec!["--preview-token", &token, "--offset", "1"],
        vec!["--plate-series", "PL:1"],
        vec!["--preview-token", &token, "--plate-series", "PL"],
        vec!["--preview-token", &token, "--tolerance", "NaN"],
        vec!["--preview-token", &token, "--tolerance", "5000"],
        vec!["--preview-token", &token, "--compare-name", "maybe"],
    ] {
        assert_eq!(
            f.refused("numbering-approve", &args),
            "INVALID_ARGUMENT",
            "{args:?}"
        );
    }
    assert_eq!(
        f.refused(
            "numbering-approve",
            &["--preview-token", &token, "--op-id", "not-an-id"]
        ),
        "INVALID_ID"
    );
    for args in [
        vec!["--limit", "0"],
        vec!["--limit", "257"],
        vec!["--offset", "x"],
        vec!["--preview-token", &token],
    ] {
        assert_eq!(
            f.refused("numbering-status", &args),
            "INVALID_ARGUMENT",
            "{args:?}"
        );
    }
    // Damaged records in the model are refused, never read as "unnumbered".
    let before = f.layers();
    for (from, to) in [
        // A partial record.
        (
            "custom string steel:numbering:basis",
            "custom string steel:numbering:other",
        ),
        // A wrong type.
        (
            "custom int steel:numbering:number = 1",
            "custom double steel:numbering:number = 1",
        ),
        // A number outside the range.
        (
            "custom int steel:numbering:number = 1",
            "custom int steel:numbering:number = -4",
        ),
        // An oversized record.
        (
            "custom string steel:numbering:series = \"PL\"",
            &format!(
                "custom string steel:numbering:series = \"{}\"",
                "X".repeat(40)
            ),
        ),
    ] {
        f.tamper(|name, text| {
            if name == "authored.usda" {
                text.replacen(from, to, 1)
            } else {
                text
            }
        });
        for command in ["numbering-status", "numbering-preview", "plate-inspect"] {
            let (code, body) = f.run(command, &[]);
            assert_eq!(code, 2, "{command} {to}: {body:?}");
        }
        let (code, body) = f.run("numbering-approve", &["--preview-token", &token]);
        assert_eq!(code, 2, "{to}: {body:?}");
        // Put the model back for the next case: the tamper commits a new head, so rewind the text.
        f.tamper(|name, text| {
            if name == "authored.usda" {
                text.replacen(to, from, 1)
            } else {
                text
            }
        });
        assert_eq!(f.state(), "current", "{to}");
    }
    // Every tamper was undone byte for byte.
    assert_eq!(f.layers(), before);
}

/// The independent reader re-reads the stored marks from the committed layer text.
#[test]
fn the_reader_reads_the_stored_marks() {
    let f = Fixture::new();
    let s = f.scene();
    f.approve(&f.token(&[]), &[]);
    let layers = f.root.join("layers");
    fs::create_dir(&layers).unwrap();
    let opened = f.ok("revision-open", &[]);
    for entry in list(&opened, "layers") {
        let digest = text(entry, "digest");
        fs::copy(
            f.root
                .join("store")
                .join("blobs")
                .join(&digest[..2])
                .join(&digest),
            layers.join(text(entry, "name")),
        )
        .unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft-usd-oracle"))
        .arg("marks")
        .arg(layers.join("root.usda"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = beamcraft_json::decode(
        String::from_utf8_lossy(&output.stdout).trim().as_bytes(),
        &Limits::DEFAULT,
    )
    .unwrap();
    // The reader and the CLI agree on every mark: series, number and the approval record.
    let status = f.status();
    let basis = text(at(&status, &["approval"]), "approved_token");
    for m in list(&status, "marks") {
        let seen = list(&report, &text(m, "id"));
        assert_eq!(seen[0].string().unwrap(), text(m, "series"));
        assert_eq!(seen[1].unsigned().unwrap(), number(m, "number"));
        assert!(seen[2].string().unwrap().contains(&basis));
    }
    assert_eq!(list(&status, "marks").len(), 8);
    assert!(report.get(&s.plates[0]).is_ok());
}
