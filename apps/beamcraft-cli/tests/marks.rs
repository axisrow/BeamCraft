// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic model content, MPL-2.0.

//! A09 end to end: `number-assign`, `number-approve` and `marks` through the real CLI
//! binary and committed revisions. A mark is a human-authored value on a plate, member,
//! assembly or weld; approval freezes it; a thickness change withdraws the approval and keeps
//! the value; a copy carries no mark; a damaged record is refused, never reported or replayed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_protocol::{
    json::{self, Json},
    message,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const PLATE: &str = "019a7bf0-0000-7000-8000-000000610001";
const PLATE2: &str = "019a7bf0-0000-7000-8000-000000610002";
const BEAM: &str = "019a7bf0-0000-7000-8000-000000610003";
const ASSEMBLY: &str = "019a7bf0-0000-7000-8000-000000610004";
const WELD: &str = "019a7bf0-0000-7000-8000-000000610005";
const LEVEL: &str = "019a7bf0-0000-7000-8000-000000610006";
const OP_ASSIGN: &str = "019a7bf0-0000-7000-8000-0000006100a1";
const OP_APPROVE: &str = "019a7bf0-0000-7000-8000-0000006100a2";
const OP_REVOKE: &str = "019a7bf0-0000-7000-8000-0000006100a3";
const OP_WITHDRAW: &str = "019a7bf0-0000-7000-8000-0000006100a4";

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a08-tests");
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
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        let store = self.root.join("store").to_str().unwrap().to_owned();
        let mut args = vec![command, "--store", store.as_str()];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        let body = json::decode(&output.stdout, &message::read_limits()).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let before = self.head();
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        // A refusal writes no revision.
        assert_eq!(self.head(), before, "{command} {extra:?}");
        text(&body, "code")
    }
    fn head(&self) -> String {
        text(&self.ok("revision-open", &[]), "revision_id")
    }
    fn plate(&self, id: &str) -> String {
        let body = self.ok(
            "create-plate",
            &[
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--new-id",
                id,
            ],
        );
        text(at(&body, &["plate"]), "element_id")
    }
    fn beam(&self) -> String {
        let profile = text(
            at(
                &self.ok("define-profile", &["--width", "100", "--height", "50"]),
                &["definition"],
            ),
            "digest",
        );
        let body = self.ok(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "3000,0,0",
                "--profile-digest",
                &profile,
                "--new-id",
                BEAM,
            ],
        );
        text(at(&body, &["member"]), "element_id")
    }
    fn assembly(&self) -> String {
        let body = self.ok(
            "create-assembly",
            &["--name", "Frame", "--new-id", ASSEMBLY],
        );
        text(&body, "element_id")
    }
    fn level(&self) -> String {
        let body = self.ok(
            "create-level",
            &["--name", "L1", "--elevation", "0", "--new-id", LEVEL],
        );
        text(&body, "element_id")
    }
    fn weld(&self, main: &str, secondary: &str) -> String {
        let body = self.ok(
            "create-weld",
            &[
                "--name",
                "W1",
                "--new-id",
                WELD,
                "--main",
                main,
                "--secondary",
                secondary,
                "--size",
                "6",
                "--location",
                "workshop",
                "--side",
                "both",
            ],
        );
        text(&body, "element_id")
    }
    fn marks(&self) -> Vec<Json> {
        list(&self.ok("marks", &[]), "marks").to_vec()
    }
    fn mark_of(&self, id: &str) -> (String, String) {
        let found = self.marks().into_iter().find(|m| text(m, "id") == id);
        let found = found.expect("no mark for the element in the head");
        (text(&found, "mark"), text(&found, "status"))
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
fn strings(json: &Json, key: &str) -> Vec<String> {
    list(json, key)
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect()
}
fn groups(change: &Json, key: &str) -> Vec<(String, String)> {
    list(at(change, &["footprint"]), key)
        .iter()
        .map(|e| (text(e, "target"), text(e, "group")))
        .collect()
}
fn assign_args<'a>(id: &'a str, mark: &'a str) -> Vec<&'a str> {
    vec!["--id", id, "--mark", mark]
}

#[test]
fn a_mark_is_assigned_approved_listed_and_replayed() {
    let f = Fixture::new();
    let plate = f.plate(PLATE);
    let beam = f.beam();
    let assembly = f.assembly();
    let weld = f.weld(&plate, &beam);

    let assigned = f.ok("number-assign", &assign_args(&plate, "P/1"));
    assert_eq!(text(&assigned, "outcome"), "mark_changed");
    assert_eq!(text(&assigned, "element_id"), plate);
    // The record lives with its source: only the authored layer changes.
    assert_eq!(strings(&assigned, "changed_layers"), ["authored.usda"]);
    // A mark decision writes only the target's mark group and reads it back.
    let writes = groups(&assigned, "writes");
    assert_eq!(
        writes,
        vec![(plate.clone(), "mark".to_owned())],
        "{writes:?}"
    );
    assert!(groups(&assigned, "reads").contains(&(plate.clone(), "mark".to_owned())));
    assert_eq!(f.mark_of(&plate), ("P/1".to_owned(), "assigned".to_owned()));

    // Every markable kind carries one; the list reports kind and label.
    f.ok("number-assign", &assign_args(&beam, "B/1"));
    f.ok("number-assign", &assign_args(&assembly, "A/1"));
    f.ok("number-assign", &assign_args(&weld, "W/1"));
    let listed: Vec<(String, String, String, String)> = f
        .marks()
        .into_iter()
        .map(|m| {
            (
                text(&m, "id"),
                text(&m, "kind"),
                text(&m, "name"),
                format!("{}/{}", text(&m, "mark"), text(&m, "status")),
            )
        })
        .collect();
    assert_eq!(
        listed,
        vec![
            (
                plate.clone(),
                "plate".into(),
                "Plate".into(),
                "P/1/assigned".into()
            ),
            (
                beam.clone(),
                "beam".into(),
                "beam".into(),
                "B/1/assigned".into()
            ),
            (
                assembly.clone(),
                "assembly".into(),
                "Frame".into(),
                "A/1/assigned".into()
            ),
            (
                weld.clone(),
                "weld".into(),
                "W1".into(),
                "W/1/assigned".into()
            ),
        ],
        "{listed:?}"
    );

    // Approval freezes the value; the list reports it.
    f.ok("number-approve", &["--id", &plate]);
    assert_eq!(f.mark_of(&plate).1, "approved");
    f.ok("number-approve", &["--id", &weld]);
    assert_eq!(f.mark_of(&weld).1, "approved");

    // Assigning a different value over an approved mark is refused and writes nothing; the
    // identical value is a no change.
    let mut over = assign_args(&plate, "P/2");
    over.extend_from_slice(&["--op-id", OP_ASSIGN]);
    assert_eq!(f.rejected("number-assign", &over), "DEPENDENCY_CONFLICT");

    // A retry of the same operation replays; the same id with other content is reuse.
    let with_op = |mark: &'static str| {
        let mut a = assign_args(&beam, mark);
        a.extend_from_slice(&["--op-id", OP_ASSIGN]);
        a
    };
    let first = f.ok("number-assign", &with_op("B/9"));
    assert_eq!(text(&first, "outcome"), "mark_changed");
    let head = f.head();
    let again = f.ok("number-assign", &with_op("B/9"));
    assert_eq!(text(&again, "outcome"), "mark_changed_replayed");
    assert_eq!(f.head(), head);
    assert_eq!(f.rejected("number-assign", &with_op("B/2")), "OP_ID_REUSE");
    assert_eq!(f.mark_of(&beam), ("B/9".to_owned(), "assigned".to_owned()));

    // A reused id is reuse across commands too, not a replay of the recorded state: the assign
    // id retried as a revoke (and, below, a revoke id retried as an assign) is OP_ID_REUSE.
    assert_eq!(
        f.rejected(
            "number-assign",
            &[
                "--id",
                beam.as_str(),
                "--revoke",
                "true",
                "--op-id",
                OP_ASSIGN
            ]
        ),
        "OP_ID_REUSE"
    );

    // An approval replays the same way, under its own operation.
    let approve_op = ["--id", beam.as_str(), "--op-id", OP_APPROVE];
    f.ok("number-approve", &approve_op);
    let replayed = f.ok("number-approve", &approve_op);
    assert_eq!(text(&replayed, "outcome"), "mark_changed_replayed");
    assert_eq!(f.mark_of(&beam).1, "approved");

    // The revoke id retried as an assign of the recorded value is also OP_ID_REUSE.
    let revoke_op = [
        "--id",
        beam.as_str(),
        "--revoke",
        "true",
        "--op-id",
        OP_REVOKE,
    ];
    f.ok("number-assign", &revoke_op);
    assert_eq!(f.mark_of(&beam).1, "assigned");
    let mut as_assign = assign_args(&beam, "B/9");
    as_assign.extend_from_slice(&["--op-id", OP_REVOKE]);
    assert_eq!(f.rejected("number-assign", &as_assign), "OP_ID_REUSE");
    assert_eq!(
        f.rejected("number-approve", &["--id", &beam, "--op-id", OP_REVOKE]),
        "OP_ID_REUSE"
    );
}

#[test]
fn wrong_targets_and_values_are_refused_with_nothing_written() {
    let f = Fixture::new();
    let missing = "019a7bf0-0000-7000-8000-0000006100ff";
    let plate = f.plate(PLATE);
    let beam = f.beam();
    let level = f.level();
    let weld = f.weld(&plate, &beam);
    f.ok("number-assign", &assign_args(&plate, "P/1"));

    // An unknown element is not found; a level is not fabricated and carries no mark.
    assert_eq!(
        f.rejected("number-assign", &assign_args(missing, "P/1")),
        "NOT_FOUND"
    );
    assert_eq!(
        f.rejected("number-assign", &assign_args(&level, "L/1")),
        "UNSUPPORTED_PROPERTY"
    );
    assert_eq!(
        f.rejected("number-approve", &["--id", missing]),
        "NOT_FOUND"
    );
    // One value per model: a duplicate is refused, whatever the kind.
    assert_eq!(
        f.rejected("number-assign", &assign_args(&beam, "P/1")),
        "DUPLICATE_ID"
    );
    assert_eq!(
        f.rejected("number-assign", &assign_args(&weld, "P/1")),
        "DUPLICATE_ID"
    );
    // The value is bounded readable text.
    assert_eq!(
        f.rejected("number-assign", &assign_args(&beam, "")),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("number-assign", &assign_args(&beam, &"x".repeat(65))),
        "LIMIT_EXCEEDED"
    );
    // Approving needs a mark and happens once; revoking needs an approval.
    assert_eq!(
        f.rejected("number-approve", &["--id", &beam]),
        "INVALID_ARGUMENT"
    );
    f.ok("number-approve", &["--id", &plate]);
    assert_eq!(
        f.rejected("number-approve", &["--id", &plate]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("number-assign", &assign_args(&plate, "P/2")),
        "DEPENDENCY_CONFLICT"
    );
    assert_eq!(
        f.rejected("number-assign", &assign_args(&plate, "P/1")),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("number-assign", &["--id", &beam, "--revoke", "true"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.mark_of(&plate).1, "approved");

    // The revoke is the door back: the value is kept, the status falls back, and the value can
    // change again.
    f.ok("number-assign", &["--id", &plate, "--revoke", "true"]);
    assert_eq!(f.mark_of(&plate), ("P/1".to_owned(), "assigned".to_owned()));
    f.ok("number-assign", &assign_args(&plate, "P/9"));
    assert_eq!(f.mark_of(&plate).0, "P/9");
    assert_eq!(f.marks().len(), 1);
}

#[test]
fn a_thickness_change_withdraws_the_approval_and_a_copy_carries_no_mark() {
    let f = Fixture::new();
    let plate = f.plate(PLATE);
    f.ok("number-assign", &assign_args(&plate, "P/1"));
    f.ok("number-approve", &["--id", &plate]);

    // Label and move edits keep the approval.
    f.ok("update-element", &["--id", &plate, "--name", "Gusset"]);
    assert_eq!(f.mark_of(&plate).1, "approved");
    f.ok(
        "update-element",
        &["--id", &plate, "--translation", "10,0,0"],
    );
    assert_eq!(f.mark_of(&plate).1, "approved");
    // A thickness change that restates the current value is no change at all.
    assert_eq!(
        f.rejected("update-element", &["--id", &plate, "--thickness", "10"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.mark_of(&plate).1, "approved");

    // A real thickness change withdraws the approval in the same revision and keeps the value.
    let changed = f.ok(
        "update-element",
        &["--id", &plate, "--thickness", "12", "--op-id", OP_WITHDRAW],
    );
    let writes = groups(&changed, "writes");
    assert!(
        writes.contains(&(plate.clone(), "mark".to_owned())),
        "{writes:?}"
    );
    assert!(writes.contains(&(plate.clone(), "shape_inputs".to_owned())));
    assert_eq!(f.mark_of(&plate), ("P/1".to_owned(), "assigned".to_owned()));

    // The documented replay boundary: the withdrawal leaves the same approved-to-assigned pair
    // a revoke leaves, so a revoke retried under the withdrawal's op id answers replayed (a
    // real revoke right now is INVALID_ARGUMENT); only the stored canonical request of F05
    // tells the two apart.
    assert_eq!(
        f.rejected("number-assign", &["--id", &plate, "--revoke", "true"]),
        "INVALID_ARGUMENT"
    );
    let replayed = f.ok(
        "number-assign",
        &["--id", &plate, "--revoke", "true", "--op-id", OP_WITHDRAW],
    );
    assert_eq!(text(&replayed, "outcome"), "mark_changed_replayed");
    assert_eq!(f.mark_of(&plate).1, "assigned");

    // The person approves again after the edit.
    f.ok("number-approve", &["--id", &plate]);
    assert_eq!(f.mark_of(&plate).1, "approved");

    // A copy is a different part: it carries no mark and the source keeps its own.
    let copied = f.ok("copy-element", &["--id", &plate, "--new-id", PLATE2]);
    assert_eq!(text(&copied, "outcome"), "element_copied");
    assert_eq!(text(at(&copied, &["element"]), "element_id"), PLATE2);
    let marked: Vec<String> = f.marks().into_iter().map(|m| text(&m, "id")).collect();
    assert_eq!(marked, vec![plate.clone()], "{marked:?}");
    assert_eq!(f.mark_of(&plate).0, "P/1");

    // Deleting a marked element removes its mark with it.
    f.ok("delete-element", &["--id", PLATE2]);
    assert_eq!(f.marks().len(), 1);
}

#[test]
fn a_damaged_mark_record_is_refused_not_reported() {
    let f = Fixture::new();
    let plate = f.plate(PLATE);
    let beam = f.beam();
    f.weld(&plate, &beam);
    f.ok("number-assign", &assign_args(&plate, "P/1"));
    f.ok("number-assign", &assign_args(&beam, "B/1"));

    // A half-authored record (the status is gone) is refused before anything reads it.
    f.tamper(|name, text| {
        if name == "authored.usda" {
            let line = "custom token bim:markStatus = \"assigned\"\n";
            assert!(text.contains(line), "{text}");
            text.replace(line, "")
        } else {
            text
        }
    });
    let (code, body) = f.run("marks", &[]);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "INVALID_ARGUMENT")
    );
    let (code, _) = f.run("number-assign", &assign_args(&plate, "P/2"));
    assert_eq!(code, 2);
}

#[test]
fn a_duplicated_mark_in_a_stored_layer_is_refused_and_a_newer_schema_stays_inert() {
    let f = Fixture::new();
    let plate = f.plate(PLATE);
    let beam = f.beam();
    f.ok("number-assign", &assign_args(&plate, "P/1"));
    f.ok("number-assign", &assign_args(&beam, "B/1"));

    // Two elements with one value is outside the model's single namespace.
    f.tamper(|name, text| {
        if name == "authored.usda" {
            text.replacen(
                "custom string bim:mark = \"B/1\"",
                "custom string bim:mark = \"P/1\"",
                1,
            )
        } else {
            text
        }
    });
    let (code, body) = f.run("marks", &[]);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "INVALID_ARGUMENT")
    );

    // Unmarked content of a newer schema version stays inert beside supported content: the
    // read only refuses on the mark rules, never on unknown content.
    let f2 = Fixture::new();
    let plate = f2.plate(PLATE);
    f2.ok("number-assign", &assign_args(&plate, "P/1"));
    let newer = "        def Xform \"E_019a7bf0000070008000000000610099\" (\n            kind = \"component\"\n        )\n        {\n            custom string bim:id = \"019a7bf0-0000-7000-8000-000000610099\"\n            custom token bim:elementKind = \"plate\"\n            custom string bim:schemaVersion = \"0.2.0\"\n            custom token bim:geometryMode = \"parametric\"\n        }\n";
    f2.tamper(|name, text| {
        if name != "authored.usda" {
            return text;
        }
        let open = "def Scope \"Steel\" (\n        kind = \"group\"\n    )\n    {\n";
        let at = text.find(open).unwrap() + open.len();
        format!("{}{newer}{}", &text[..at], &text[at..])
    });
    assert_eq!(
        f2.mark_of(&plate),
        ("P/1".to_owned(), "assigned".to_owned())
    );
    assert_eq!(f2.marks().len(), 1);
}
