// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original models and references, MPL-2.0; existing test-data attribution applies.

//! Actual two-client CLI rebase, including strict inspection and a draft CAS race.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_cli::{
    plate,
    workspace::{
        self,
        rebase::{RebaseRequest, rebase_workspace},
    },
};
use beamcraft_host::{
    revision::{CommitPoint, RevisionInfo, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{
    error::ErrorCode,
    id::{ElementId, OpId},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn id(n: usize) -> String {
    format!("019a7bf0-0000-7000-8000-{n:012x}")
}
fn store(path: &Path) -> RevisionStore {
    RevisionStore::new(BlobStore::new(path).unwrap())
}
fn text<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key).unwrap().string().unwrap()
}
struct Fixture(PathBuf, BTreeMap<&'static str, String>);
impl Fixture {
    fn new(reference: bool) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "rebase-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let paths = ["published", "a", "b", "reference.usda"]
            .into_iter()
            .map(|name| (name, root.join(name).to_str().unwrap().to_owned()))
            .collect();
        let f = Self(root, paths);
        fs::create_dir(&f.0).unwrap();
        fs::create_dir(f.path("published")).unwrap();
        let material = f.ok(&[
            "define-material",
            "--store",
            f.s("published"),
            "--density",
            "7850",
        ]);
        let material = text(material.get("definition").unwrap(), "digest");
        for n in [10, 11] {
            f.ok(&[
                "create-plate",
                "--store",
                f.s("published"),
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--new-id",
                &id(n),
                "--material-digest",
                material,
            ]);
        }
        if reference {
            fs::write(f.path("reference.usda"),b"#usda 1.0\n(defaultPrim=\"Survey\"\nmetersPerUnit=0.001\nupAxis=\"Z\")\ndef Xform \"Survey\" { custom double original:height=125 }\n").unwrap();
            f.ok(&[
                "attach-reference",
                "--store",
                f.s("published"),
                "--source",
                f.s("reference.usda"),
            ]);
        }
        for name in ["a", "b"] {
            f.ok(&[
                "create-workspace",
                "--store",
                f.s("published"),
                "--workspace-dir",
                f.s(name),
                "--user-id",
                &id(3),
            ]);
        }
        f
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn s(&self, name: &str) -> &str {
        self.1.get(name).unwrap()
    }
    fn run(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap(),
        )
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, json) = self.run(args);
        assert_eq!(code, 0, "{args:?}: {json:?}");
        json
    }
    fn head(&self, name: &str) -> RevisionInfo {
        store(&self.path(name)).head().unwrap().unwrap()
    }
    fn update(&self, name: &str, n: usize, patch: &[&str]) {
        let path = self.path(name);
        let mut args = vec!["update-element", "--store", path.to_str().unwrap(), "--id"];
        let element = id(n);
        args.push(&element);
        args.extend(patch);
        self.ok(&args);
    }
    fn publish(&self, name: &str) {
        self.ok(&[
            "publish-workspace",
            "--store",
            self.s("published"),
            "--workspace-dir",
            self.s(name),
        ]);
    }
    fn rebase(&self, n: usize, accepted: &RevisionInfo, draft: &RevisionInfo) -> (i32, Json) {
        self.run(&[
            "rebase-workspace",
            "--store",
            self.s("published"),
            "--workspace-dir",
            self.s("b"),
            "--op-id",
            &id(n),
            "--expected-head",
            accepted.revision().as_str(),
            "--expected-draft",
            draft.revision().as_str(),
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn independent_edits_rebase_without_publishing_and_replay_after_a_later_edit() {
    for same_part in [false, true] {
        let f = Fixture::new(!same_part);
        f.update("a", 10, &["--name", "remote name"]);
        f.update(
            "b",
            if same_part { 10 } else { 11 },
            if same_part {
                &["--translation", "12,34,56"]
            } else {
                &["--name", "local name"]
            },
        );
        let original = f.head("b");
        f.publish("a");
        let accepted = f.head("published");
        let old_meta = workspace::read_meta(&f.path("b")).unwrap();
        let (code, result) = f.rebase(500, &accepted, &original);
        assert_eq!(code, 0, "{result:?}");
        assert_eq!(text(&result, "outcome"), "workspace_rebased");
        let candidate = f.head("b");
        assert_eq!(candidate.sequence(), original.sequence() + 1);
        assert_eq!(
            f.head("published").manifest_digest(),
            accepted.manifest_digest()
        );
        let meta = workspace::read_meta(&f.path("b")).unwrap();
        assert_eq!(meta.synced_sequence, old_meta.synced_sequence);
        assert_eq!(meta.base_manifest, accepted.manifest_digest());
        assert_eq!(meta.rebase_sequence, candidate.sequence());
        let report = plate::inspect(&store(&f.path("b"))).unwrap();
        let a = report
            .plates
            .iter()
            .find(|p| p.authored.id == ElementId::parse(&id(10)).unwrap())
            .unwrap();
        assert_eq!(a.authored.name, "remote name");
        assert_eq!(a.quantities.volume_mm3, 5_000_000.0);
        assert_eq!(a.quantities.mass_kg, Some(39.25));
        if same_part {
            assert_eq!(a.authored.placement.translation_mm, [12.0, 34.0, 56.0]);
        } else {
            assert_eq!(
                report
                    .plates
                    .iter()
                    .find(|p| p.authored.id == ElementId::parse(&id(11)).unwrap())
                    .unwrap()
                    .authored
                    .name,
                "local name"
            );
        }
        f.update("b", 11, &["--name", "later local"]);
        let later = f.head("b");
        let (code, replay) = f.rebase(500, &accepted, &original);
        assert_eq!(code, 0, "{replay:?}");
        assert_eq!(text(&replay, "outcome"), "workspace_rebase_replayed");
        assert_eq!(
            text(replay.get("revision").unwrap(), "revision_id"),
            candidate.revision().as_str()
        );
        assert_eq!(f.head("b").manifest_digest(), later.manifest_digest());
        let (code, reused) = f.rebase(500, &accepted, &later);
        assert_eq!(code, 2);
        assert_eq!(text(&reused, "code"), "OP_ID_REUSE");
        f.publish("b");
        let final_report = plate::inspect(&store(&f.path("published"))).unwrap();
        assert_eq!(final_report.plates.len(), 2);
        if !same_part {
            let published = store(&f.path("published")).open().unwrap();
            let draft = store(&f.path("b")).open().unwrap();
            for entry in accepted
                .manifest()
                .content()
                .entries()
                .iter()
                .filter(|e| e.role() == beamcraft_host::revision::LayerRole::External)
            {
                assert_eq!(
                    published.layer_bytes(entry.name()).unwrap(),
                    draft.layer_bytes(entry.name()).unwrap()
                );
                assert_eq!(
                    published.layer_bytes(entry.name()).unwrap(),
                    fs::read(f.path("reference.usda")).unwrap()
                );
            }
        }
    }
}

#[test]
fn semantic_conflicts_include_indirect_reads_and_intermediate_reverted_writes() {
    for case in ["equal-label", "shape", "copy-source", "reverted-label"] {
        let f = Fixture::new(false);
        if case == "copy-source" {
            f.ok(&[
                "copy-element",
                "--store",
                f.s("b"),
                "--id",
                &id(10),
                "--new-id",
                &id(12),
            ]);
        } else {
            f.update(
                "b",
                10,
                if case == "shape" {
                    &["--thickness", "20"]
                } else {
                    &["--name", "same"]
                },
            );
        }
        f.update(
            "a",
            10,
            if case == "shape" {
                &["--thickness", "30"]
            } else {
                &["--name", "same"]
            },
        );
        if case == "reverted-label" {
            f.update("a", 10, &["--name", "Plate"]);
        }
        f.publish("a");
        let accepted = f.head("published");
        let draft = f.head("b");
        let metadata = fs::read(f.path("b").join("workspace.json")).unwrap();
        let (code, result) = f.rebase(500, &accepted, &draft);
        assert_eq!(code, 2, "{case}: {result:?}");
        assert_eq!(text(&result, "code"), "DEPENDENCY_CONFLICT");
        let diagnostics = result.get("diagnostics").unwrap().array().unwrap();
        assert!(!diagnostics.is_empty());
        assert!(diagnostics.iter().all(|d| text(d, "target") == id(10)));
        assert_eq!(
            f.head("published").manifest_digest(),
            accepted.manifest_digest()
        );
        assert_eq!(f.head("b").manifest_digest(), draft.manifest_digest());
        assert_eq!(
            fs::read(f.path("b").join("workspace.json")).unwrap(),
            metadata
        );
    }
}

#[test]
fn absent_evidence_and_wrong_actor_do_not_become_independence() {
    let f = Fixture::new(false);
    f.update("b", 10, &["--name", "temporary"]);
    f.ok(&["undo-workspace", "--workspace-dir", f.s("b")]);
    f.update("a", 11, &["--name", "remote"]);
    f.publish("a");
    let accepted = f.head("published");
    let draft = f.head("b");
    let (code, result) = f.rebase(500, &accepted, &draft);
    assert_eq!(code, 2, "{result:?}");
    assert_eq!(text(&result, "code"), "DEPENDENCY_UNAVAILABLE");
    let (code, result) = f.run(&[
        "rebase-workspace",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.s("b"),
        "--op-id",
        &id(501),
        "--expected-head",
        accepted.revision().as_str(),
        "--expected-draft",
        draft.revision().as_str(),
        "--actor-id",
        &id(99),
    ]);
    assert_eq!(code, 2);
    assert_eq!(text(&result, "code"), "PERMISSION_DENIED");
    assert!(
        store(&f.path("b"))
            .changeset_claim(&Uuid7::parse(&id(501)).unwrap())
            .unwrap()
            .is_none()
    );
    assert_eq!(f.head("b").manifest_digest(), draft.manifest_digest());
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
    f.update("a", 11, &["--name", "a later accepted head"]);
    f.publish("a");
    let (code, retry) = f.rebase(500, &accepted, &draft);
    assert_eq!(code, 2);
    assert_eq!(text(&retry, "code"), "BASE_REVISION_MISMATCH");
}

#[test]
fn accepted_head_is_pinned_while_a_concurrent_local_edit_wins_the_draft_cas() {
    let f = Fixture::new(false);
    f.update("b", 11, &["--name", "local"]);
    f.update("a", 10, &["--name", "remote"]);
    f.publish("a");
    let accepted = f.head("published");
    let draft = f.head("b");
    let old = workspace::read_meta(&f.path("b")).unwrap();
    let mut raced = false;
    let mut hook = |point| {
        if point == CommitPoint::Checked && !raced {
            raced = true;
            let (code, result) = f.run(&[
                "update-element",
                "--store",
                f.s("published"),
                "--id",
                &id(10),
                "--name",
                "cannot publish under the guard",
            ]);
            assert_eq!(code, 3, "{result:?}");
            f.update("b", 11, &["--name", "concurrent local"]);
        }
        Ok(())
    };
    let result = rebase_workspace(
        &store(&f.path("published")),
        &store(&f.path("b")),
        &f.path("b"),
        &RebaseRequest {
            op_id: OpId::parse(&id(500)).unwrap(),
            actor: Uuid7::parse(&id(3)).unwrap(),
            expected_head: accepted.revision().clone(),
            expected_draft: draft.revision().clone(),
        },
        &mut hook,
    );
    assert_eq!(result.err().unwrap().code, ErrorCode::BaseRevisionMismatch);
    assert!(raced);
    assert_eq!(workspace::read_meta(&f.path("b")).unwrap(), old);
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
    assert_eq!(
        plate::inspect(&store(&f.path("b")))
            .unwrap()
            .plates
            .iter()
            .find(|p| p.authored.id == ElementId::parse(&id(11)).unwrap())
            .unwrap()
            .authored
            .name,
        "concurrent local"
    );
}
