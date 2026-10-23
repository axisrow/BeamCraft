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

#[path = "rebase/conflicts.rs"]
mod conflicts;

#[path = "rebase/compensation_proof.rs"]
mod compensation_proof;
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
                "reference-attach",
                "--store",
                f.s("published"),
                "--source",
                f.s("reference.usda"),
            ]);
        }
        for name in ["a", "b"] {
            f.ok(&[
                "workspace-create",
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
            "workspace-publish",
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
fn hostile_contexts_metadata_and_pinned_blobs_refuse_without_a_partial_rebase() {
    use beamcraft_host::revision::{ExpectedBase, NoFault, RevisionDraft};
    use beamcraft_model::digest::Sha256;

    for case in [
        "project",
        "schema",
        "workspace",
        "base-manifest",
        "base-blob",
        "evidence",
        "metadata-version",
        "metadata-cap",
        "metadata-cap-plus-one",
    ] {
        let f = Fixture::new(false);
        f.update("a", 10, &["--name", "accepted"]);
        f.update("b", 11, &["--name", "unsent"]);
        f.publish("a");
        let draft = f.head("b");
        let original_accepted = f.head("published");
        let mut accepted_path = f.path("published");
        if ["project", "schema", "workspace"].contains(&case) {
            // A different store with otherwise real, identical layer blobs is not this draft's
            // accepted project/basis. Do not let matching visible model content authorize it.
            let published = store(&f.path("published"));
            let old = published.open().unwrap();
            let c = old.manifest().content();
            accepted_path = f.path("wrong-store");
            fs::create_dir(&accepted_path).unwrap();
            let wrong = store(&accepted_path);
            for entry in c.entries() {
                wrong
                    .blobs()
                    .publish(&old.layer_bytes(entry.name()).unwrap())
                    .unwrap();
            }
            wrong
                .commit_with(
                    &RevisionDraft {
                        project: if case == "project" {
                            Uuid7::parse(&id(900)).unwrap()
                        } else {
                            c.project().clone()
                        },
                        workspace: if case == "workspace" {
                            Uuid7::parse(&id(901)).unwrap()
                        } else {
                            c.workspace().clone()
                        },
                        schema: if case == "schema" {
                            Sha256::digest(b"unsupported schema").unwrap()
                        } else {
                            c.schema()
                        },
                        revision: Uuid7::parse(&id(902)).unwrap(),
                        op: Uuid7::parse(&id(903)).unwrap(),
                        workspace_generation: c.workspace_generation() + 1,
                        root_layer: c.root_layer().into(),
                        entries: c.entries().to_vec(),
                    },
                    &ExpectedBase::Initial,
                    None,
                    &mut NoFault,
                )
                .unwrap();
        }
        let metadata_path = f.path("b").join("workspace.json");
        let meta = workspace::read_meta(&f.path("b")).unwrap();
        let mut damaged_blob = None;
        match case {
            "metadata-version" => {
                let original = fs::read_to_string(&metadata_path).unwrap();
                fs::write(
                    &metadata_path,
                    original.replacen("workspace 1", "workspace 999", 1),
                )
                .unwrap();
            }
            "metadata-cap" => fs::write(&metadata_path, vec![b'x'; 4096]).unwrap(),
            "metadata-cap-plus-one" => fs::write(&metadata_path, vec![b'x'; 4097]).unwrap(),
            "base-manifest" | "base-blob" | "evidence" => {
                let (name, digest) = if case == "evidence" {
                    (
                        "b",
                        draft
                            .manifest()
                            .content()
                            .operation_evidence()
                            .unwrap()
                            .digest(),
                    )
                } else if case == "base-manifest" {
                    ("published", meta.base_manifest)
                } else {
                    let base = store(&f.path("published"))
                        .open_at(
                            store(&f.path("published"))
                                .sequence_of(&meta.base_revision)
                                .unwrap()
                                .unwrap(),
                        )
                        .unwrap();
                    (
                        "published",
                        base.manifest()
                            .content()
                            .entries()
                            .iter()
                            .find(|e| e.name() == plate::AUTHORED_LAYER)
                            .unwrap()
                            .digest(),
                    )
                };
                let digest = digest.to_string();
                let prefix: String = digest.chars().take(2).collect();
                let path = f.path(name).join("blobs").join(prefix).join(digest);
                fs::write(&path, b"truncated and not the pinned digest").unwrap();
                damaged_blob = Some(path);
            }
            _ => (),
        }
        let accepted = store(&accepted_path).head().unwrap().unwrap();
        let bytes = fs::read(&metadata_path).unwrap();
        let (code, result) = f.run(&[
            "rebase-workspace",
            "--store",
            accepted_path.to_str().unwrap(),
            "--workspace-dir",
            f.s("b"),
            "--op-id",
            &id(500),
            "--expected-head",
            accepted.revision().as_str(),
            "--expected-draft",
            draft.revision().as_str(),
        ]);
        assert_ne!(code, 0, "{case}: {result:?}");
        assert_ne!(
            text(&result, "code"),
            "DEPENDENCY_CONFLICT",
            "{case}: malformed input is not a semantic conflict"
        );
        assert_eq!(
            f.head("b").manifest_digest(),
            draft.manifest_digest(),
            "{case}"
        );
        assert_eq!(
            f.head("published").manifest_digest(),
            original_accepted.manifest_digest(),
            "{case}"
        );
        assert_eq!(
            store(&accepted_path)
                .head()
                .unwrap()
                .unwrap()
                .manifest_digest(),
            accepted.manifest_digest(),
            "{case}"
        );
        assert_eq!(fs::read(&metadata_path).unwrap(), bytes, "{case}");
        assert!(!f.path("b").join("rebase.pending").exists(), "{case}");
        if let Some(path) = damaged_blob {
            assert_eq!(
                fs::read(path).unwrap(),
                b"truncated and not the pinned digest"
            );
        }
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
    f.ok(&["workspace-undo", "--workspace-dir", f.s("b")]);
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

#[test]
fn both_clients_new_parts_and_immutable_materials_survive_rebase() {
    let f = Fixture::new(false);
    for (name, n, density) in [("a", 12, "8000"), ("b", 13, "7000")] {
        let material = f.ok(&[
            "define-material",
            "--store",
            f.s(name),
            "--density",
            density,
            "--name",
            name,
        ]);
        let digest = text(material.get("definition").unwrap(), "digest");
        f.ok(&[
            "create-plate",
            "--store",
            f.s(name),
            "--width",
            "10",
            "--length",
            "20",
            "--thickness",
            "3",
            "--new-id",
            &id(n),
            "--name",
            name,
            "--material-digest",
            digest,
        ]);
    }
    let original = f.head("b");
    f.publish("a");
    let accepted = f.head("published");
    let (code, result) = f.rebase(500, &accepted, &original);
    assert_eq!(code, 0, "{result:?}");
    f.publish("b");
    let report = plate::inspect(&store(&f.path("published"))).unwrap();
    assert_eq!(report.plates.len(), 4);
    assert_eq!(report.definitions.len(), 3);
    for (n, mass) in [(12, 0.0048), (13, 0.0042)] {
        let part = report
            .plates
            .iter()
            .find(|p| p.authored.id == ElementId::parse(&id(n)).unwrap())
            .unwrap();
        assert_eq!(part.quantities.volume_mm3, 600.0);
        assert!((part.quantities.mass_kg.unwrap() - mass).abs() < 1e-12);
    }
}

#[test]
fn repeated_rebase_retains_unsent_ids_and_undo_stops_at_the_latest_combined_basis() {
    use beamcraft_cli::workspace::rebase_receipt::RebaseReceipt;
    let f = Fixture::new(false);
    f.update("b", 11, &["--name", "local"]);
    let original = f.head("b");
    f.update("a", 10, &["--name", "accepted first"]);
    f.publish("a");
    let first = f.head("published");
    assert_eq!(f.rebase(500, &first, &original).0, 0);
    let combined = f.head("b");
    f.update("a", 10, &["--name", "accepted second"]);
    f.publish("a");
    let second = f.head("published");
    let (code, result) = f.rebase(501, &second, &combined);
    assert_eq!(code, 0, "{result:?}");
    let latest = f.head("b");
    let receipt = RebaseReceipt::decode(
        &store(&f.path("b"))
            .open()
            .unwrap()
            .operation_evidence()
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt.operations.len(), 2);
    assert_eq!(receipt.operations[0].result, original.manifest_digest());
    assert_eq!(receipt.operations[1].result, combined.manifest_digest());
    assert_eq!(f.rebase(500, &first, &original).0, 0);
    assert_eq!(f.head("b").manifest_digest(), latest.manifest_digest());
    let (code, refused) = f.run(&["workspace-undo", "--workspace-dir", f.s("b")]);
    assert_eq!(code, 2, "{refused:?}");
    f.update("b", 11, &["--name", "after combined basis"]);
    f.ok(&["workspace-undo", "--workspace-dir", f.s("b")]);
    f.publish("b");
    let report = plate::inspect(&store(&f.path("published"))).unwrap();
    for (n, name) in [(10, "accepted second"), (11, "local")] {
        assert_eq!(
            report
                .plates
                .iter()
                .find(|p| p.authored.id == ElementId::parse(&id(n)).unwrap())
                .unwrap()
                .authored
                .name,
            name
        );
    }
}

#[test]
fn an_unpublished_weld_conflicts_with_deleting_its_linked_part() {
    let f = Fixture::new(false);
    f.ok(&[
        "create-weld",
        "--name",
        "Local weld",
        "--store",
        f.s("b"),
        "--new-id",
        &id(12),
        "--main",
        &id(10),
        "--secondary",
        &id(11),
        "--size",
        "6",
        "--location",
        "workshop",
        "--side",
        "both",
    ]);
    f.ok(&["delete-element", "--store", f.s("a"), "--id", &id(10)]);
    f.publish("a");
    let accepted = f.head("published");
    let draft = f.head("b");
    let metadata = fs::read(f.path("b").join("workspace.json")).unwrap();
    let (code, result) = f.rebase(500, &accepted, &draft);
    assert_eq!(code, 2, "{result:?}");
    assert_eq!(text(&result, "code"), "DEPENDENCY_CONFLICT");
    assert!(
        result
            .get("diagnostics")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .any(|d| text(d, "target") == id(10) && text(d, "group") == "lifecycle")
    );
    assert_eq!(f.head("b").manifest_digest(), draft.manifest_digest());
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
    assert_eq!(
        fs::read(f.path("b").join("workspace.json")).unwrap(),
        metadata
    );
}

#[test]
fn an_interrupted_runtime_request_retries_once_and_recovers_beyond_a_later_edit() {
    use beamcraft_host::revision::Injected;
    for point in [CommitPoint::Checked, CommitPoint::Switched] {
        let f = Fixture::new(false);
        f.update("b", 11, &["--name", "local"]);
        let original = f.head("b");
        f.update("a", 10, &["--name", "remote"]);
        f.publish("a");
        let accepted = f.head("published");
        let mut reached = false;
        let mut hook = |seen| {
            if seen == point {
                reached = true;
                Err(Injected)
            } else {
                Ok(())
            }
        };
        let attempted = rebase_workspace(
            &store(&f.path("published")),
            &store(&f.path("b")),
            &f.path("b"),
            &RebaseRequest {
                op_id: OpId::parse(&id(500)).unwrap(),
                actor: Uuid7::parse(&id(3)).unwrap(),
                expected_head: accepted.revision().clone(),
                expected_draft: original.revision().clone(),
            },
            &mut hook,
        );
        let code = attempted.err().unwrap().code;
        assert!(reached);
        if point == CommitPoint::Switched {
            assert_eq!(code, ErrorCode::OutcomeUnknown);
            f.update("b", 11, &["--name", "later local"]);
        }
        let (code, result) = f.rebase(500, &accepted, &original);
        assert_eq!(code, 0, "{result:?}");
        assert_eq!(
            text(&result, "outcome"),
            if point == CommitPoint::Switched {
                "workspace_rebase_replayed"
            } else {
                "workspace_rebased"
            }
        );
        assert_eq!(
            f.head("b").sequence(),
            original.sequence() + if point == CommitPoint::Switched { 2 } else { 1 }
        );
        assert_eq!(
            f.head("published").manifest_digest(),
            accepted.manifest_digest()
        );
        let expected = if point == CommitPoint::Switched {
            "later local"
        } else {
            "local"
        };
        assert_eq!(
            plate::inspect(&store(&f.path("b")))
                .unwrap()
                .plates
                .iter()
                .find(|p| p.authored.id == ElementId::parse(&id(11)).unwrap())
                .unwrap()
                .authored
                .name,
            expected
        );
    }
}
