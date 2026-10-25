// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and beams, MPL-2.0. Expected values are closed forms.

//! P05 end to end: `copy-element` makes an independent element with the source's authored inputs
//! and the same body. The source is never written; editing the copy later rebuilds only the
//! copy's mesh and quantities.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p05-tests");
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
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}");
        text(&body, "code")
    }
    /// Commits a head-based revision with changed layer text and no content verification, the way
    /// a layer edited outside BeamCraft would arrive.
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
    fn definition(&self, command: &str, extra: &[&str]) -> String {
        text(self.ok(command, extra).get("definition").unwrap(), "digest")
    }
    fn plate(&self, extra: &[&str]) -> Json {
        let mut args = vec!["--width", "1000", "--length", "500", "--thickness", "10"];
        args.extend_from_slice(extra);
        self.ok("create-plate", &args)
    }
    fn plates(&self) -> Vec<Json> {
        at(&self.ok("inspect-elements", &[]), &["plates"])
            .array()
            .unwrap()
            .to_vec()
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
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn find<'a>(elements: &'a [Json], id: &str) -> &'a Json {
    elements
        .iter()
        .find(|e| text(e, "element_id") == id)
        .unwrap()
}
fn translation(element: &Json) -> Vec<f64> {
    at(element, &["placement", "translation_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}

#[test]
fn a_plate_copy_is_independent_and_shares_the_body() {
    let f = Fixture::new();
    let steel = f.definition("define-material", &["--density", "7850"]);
    let created = f.plate(&[
        "--material-digest",
        &steel,
        "--hole-rect",
        "100,100,100,100",
        "--translation",
        "10,20,30",
    ]);
    let source = text(at(&created, &["plate"]), "element_id");
    let before = f.plates();
    let source_before = find(&before, &source).clone();

    let copied = f.ok(
        "copy-element",
        &[
            "--id",
            &source,
            "--name",
            "Plate copy",
            "--offset",
            "1000,0,-5",
        ],
    );
    assert_eq!(text(&copied, "outcome"), "element_copied");
    let copy = at(&copied, &["element"]);
    let copy_id = text(copy, "element_id");
    assert_ne!(copy_id, source);
    assert_eq!(text(copy, "name"), "Plate copy");
    assert_eq!(translation(copy), [1010.0, 20.0, 25.0]);
    // The body is the source's: the same mesh digest, volume and mass.
    assert_eq!(
        text(copy, "mesh_digest"),
        text(&source_before, "mesh_digest")
    );
    assert_eq!(
        num(copy, &["quantities", "volume", "mm3"]),
        num(&source_before, &["quantities", "volume", "mm3"])
    );
    assert_eq!(num(copy, &["quantities", "mass", "kg"]), 38.465);

    // The footprint writes only the copy's identity and reads the source.
    let footprint = at(&copied, &["footprint"]);
    let writes = at(footprint, &["writes"]).array().unwrap();
    assert!(writes.iter().all(|e| text(e, "target") == copy_id));
    let reads = at(footprint, &["reads"]).array().unwrap();
    assert!(
        reads
            .iter()
            .any(|e| text(e, "target") == source && text(e, "group") == "shape_inputs")
    );

    // The source reads back unchanged, and both stored records are current.
    let after = f.plates();
    assert_eq!(after.len(), 2);
    assert_eq!(find(&after, &source), &source_before);
    let stored = at(find(&after, &copy_id), &["stored_quantities"]).clone();
    assert_eq!(text(at(&stored, &["volume"]), "freshness"), "current");
    assert_eq!(text(at(&stored, &["mass"]), "freshness"), "current");

    // Editing only the copy builds a new mesh for it; the source keeps its inputs and records.
    let edited = f.ok("update-element", &["--id", &copy_id, "--thickness", "20"]);
    assert_ne!(
        text(at(&edited, &["element"]), "mesh_digest"),
        text(&source_before, "mesh_digest")
    );
    let last = f.plates();
    assert_eq!(find(&last, &source), &source_before);
    assert_eq!(
        num(find(&last, &copy_id), &["quantities", "volume", "mm3"]),
        9_800_000.0
    );
}

#[test]
fn a_member_copy_keeps_the_pinned_profile_and_material() {
    let f = Fixture::new();
    let profile = f.definition("define-profile", &["--width", "200", "--height", "300"]);
    let steel = f.definition("define-material", &["--density", "7850"]);
    let created = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            &profile,
            "--material-digest",
            &steel,
        ],
    );
    let source = text(at(&created, &["member"]), "element_id");
    let copied = f.ok(
        "copy-element",
        &["--id", &source, "--offset", "0,500,0", "--name", "Beam 2"],
    );
    let copy = at(&copied, &["element"]);
    assert_eq!(text(copy, "profile_digest"), profile);
    assert_eq!(text(copy, "material_digest"), steel);
    assert_eq!(translation(copy), [0.0, 500.0, 0.0]);
    assert_eq!(
        text(copy, "mesh_digest"),
        text(at(&created, &["member"]), "mesh_digest")
    );
    // The profile is a read in the footprint, never a write.
    let footprint = at(&copied, &["footprint"]);
    assert!(
        at(footprint, &["reads"])
            .array()
            .unwrap()
            .iter()
            .any(|e| text(e, "target") == profile)
    );
    assert!(
        at(footprint, &["writes"])
            .array()
            .unwrap()
            .iter()
            .all(|e| text(e, "target") != profile)
    );
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(at(&inspected, &["members"]).array().unwrap().len(), 2);
}

#[test]
fn a_copy_is_refused_replayed_or_rejected_by_the_ledger() {
    let f = Fixture::new();
    let created = f.plate(&[]);
    let source = text(at(&created, &["plate"]), "element_id");
    let unknown = "019a7bf0-0000-7000-8000-0000000000aa";
    assert_eq!(f.rejected("copy-element", &["--id", unknown]), "NOT_FOUND");
    assert_eq!(
        f.rejected("copy-element", &["--id", &source, "--new-id", &source]),
        "DUPLICATE_ID"
    );
    assert_eq!(
        f.rejected("copy-element", &["--id", &source, "--offset", "NaN,0,0"]),
        "NON_FINITE_NUMBER"
    );
    assert_eq!(
        f.rejected(
            "copy-element",
            &["--id", &source, "--offset", "2000000000,0,0"]
        ),
        "LIMIT_EXCEEDED"
    );
    assert_eq!(
        f.rejected("copy-element", &["--id", &source, "--name", ""]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.plates().len(), 1);

    let op = "019a7bf0-0000-7000-8000-0000000000b1";
    let new_id = "019a7bf0-0000-7000-8000-0000000000b2";
    let args = [
        "--id", &source, "--new-id", new_id, "--op-id", op, "--offset", "100,0,0",
    ];
    let first = f.ok("copy-element", &args);
    assert_eq!(text(&first, "outcome"), "element_copied");
    // The same operation is a replay: no new element and the same revision.
    let again = f.ok("copy-element", &args);
    assert_eq!(text(&again, "outcome"), "element_copy_replayed");
    assert_eq!(
        text(at(&again, &["revision"]), "revision_id"),
        text(at(&first, &["revision"]), "revision_id")
    );
    assert_eq!(f.plates().len(), 2);
    // The same operation with other content is OP_ID_REUSE; a fresh operation that reuses the
    // copy's identity is DUPLICATE_ID.
    assert_eq!(
        f.rejected(
            "copy-element",
            &[
                "--id", &source, "--new-id", new_id, "--op-id", op, "--offset", "7,0,0"
            ]
        ),
        "OP_ID_REUSE"
    );
    assert_eq!(
        f.rejected("copy-element", &["--id", &source, "--new-id", new_id]),
        "DUPLICATE_ID"
    );
    assert_eq!(f.plates().len(), 2);
}

#[test]
fn a_copy_refuses_element_links_it_cannot_carry_over() {
    let f = Fixture::new();
    let holder = text(at(&f.plate(&[]), &["plate"]), "element_id");
    let target = text(at(&f.plate(&[]), &["plate"]), "element_id");
    // The holder gets a link to the other plate, as a later command or another tool would add.
    let hex = |id: &str| id.replace('-', "");
    f.tamper(|name, mut text| {
        if name == "authored.usda" {
            let marker = format!("\"PRT_{}\"", hex(&holder));
            let start = text.find(&marker).unwrap();
            let brace = start + text[start..].find('{').unwrap() + 1;
            text.insert_str(
                brace,
                &format!(
                    "\ncustom rel bim:test:supports = </World/Steel/Parts/PRT_{}>\n",
                    hex(&target)
                ),
            );
        }
        text
    });
    let before = f.plates().len();
    assert_eq!(
        f.rejected("copy-element", &["--id", &holder]),
        "DEPENDENCY_CONFLICT"
    );
    assert_eq!(f.plates().len(), before);
    // The link target holds no link itself, so it can still be copied.
    f.ok("copy-element", &["--id", &target]);
    assert_eq!(f.plates().len(), before + 1);
}
