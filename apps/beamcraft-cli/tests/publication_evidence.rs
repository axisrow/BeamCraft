// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic commands, stores and malicious payloads, MPL-2.0.

//! C03.4: exact operation dependencies survive both publication paths; a sender cannot narrow
//! a captured source interval or substitute another output. Legacy data stays conservative.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_cli::command_evidence::{CapturedFootprint, read};
use beamcraft_engine::footprint::{Entry, Footprint, Group, Target};
use beamcraft_host::revision::{
    EvidencePin, ExpectedBase, NoFault, RevisionDraft, RevisionStore, Uuid7,
};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::changeset::{
    Access, Changeset, Dependency, DependencyGroup, DependencyTarget,
};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const USER: &str = "019a7bf0-0000-7000-8000-000000000099";
fn element(n: usize) -> ElementId {
    ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
}
fn text<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key).unwrap().string().unwrap()
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "publication-evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        let f = Self(root);
        for n in 1..=2 {
            f.ok(&[
                "create-plate",
                "--store",
                &f.path("published"),
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--new-id",
                &element(n).to_string(),
            ]);
        }
        f.ok(&[
            "workspace-create",
            "--store",
            &f.path("published"),
            "--workspace-dir",
            &f.path("draft"),
            "--user-id",
            USER,
        ]);
        f
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().into()
    }
    fn store(&self, name: &str) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.0.join(name)).unwrap())
    }
    fn run(&self, args: &[&str]) -> (i32, Json) {
        let out = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let json = beamcraft_json::decode(&out.stdout, &Limits::DEFAULT).unwrap();
        (out.status.code().unwrap(), json)
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, json) = self.run(args);
        assert_eq!(code, 0, "{args:?}: {json:?}");
        json
    }
    fn rename(&self, name: &str) {
        self.ok(&[
            "update-element",
            "--store",
            &self.path("draft"),
            "--id",
            &element(1).to_string(),
            "--name",
            name,
        ]);
    }
    fn create(&self) -> (ContentDigest, Changeset) {
        let result = self.ok(&[
            "changeset-create",
            "--store",
            &self.path("published"),
            "--workspace-dir",
            &self.path("draft"),
        ]);
        let digest = text(result.get("changeset").unwrap(), "digest")
            .parse()
            .unwrap();
        let stored =
            beamcraft_cli::changeset::read_changeset(&self.store("draft"), &digest).unwrap();
        (digest, stored.changeset)
    }
    fn put(&self, changeset: &Changeset) -> ContentDigest {
        self.store("draft")
            .blobs()
            .publish(&changeset.encode().unwrap())
            .unwrap()
            .0
    }
    fn apply(&self, digest: ContentDigest) -> (i32, Json) {
        self.run(&[
            "changeset-apply",
            "--store",
            &self.path("published"),
            "--workspace-dir",
            &self.path("draft"),
            "--changeset",
            &digest.to_string(),
            "--actor-id",
            USER,
        ])
    }
    fn head(&self) -> ContentDigest {
        self.store("published")
            .head()
            .unwrap()
            .unwrap()
            .manifest_digest()
    }
    fn raw(&self, n: usize, footprint: Option<Footprint>, malformed: bool) {
        let store = self.store("draft");
        let head = store.open().unwrap();
        let c = head.manifest().content();
        let op = Uuid7::parse(&element(100 + n).to_string()).unwrap();
        let pin = if let Some(footprint) = footprint {
            let bytes = if malformed {
                b"unsupported evidence\n".to_vec()
            } else {
                CapturedFootprint {
                    op_id: OpId::parse(op.as_str()).unwrap(),
                    fingerprint: Sha256::digest(b"synthetic raw input").unwrap(),
                    footprint,
                }
                .encode()
                .unwrap()
            };
            Some(
                EvidencePin::new(store.blobs().publish(&bytes).unwrap().0, bytes.len() as u64)
                    .unwrap(),
            )
        } else {
            None
        };
        let draft = RevisionDraft {
            project: c.project().clone(),
            workspace: c.workspace().clone(),
            revision: Uuid7::parse(&element(200 + n).to_string()).unwrap(),
            op,
            workspace_generation: c.workspace_generation() + 1,
            schema: c.schema(),
            root_layer: c.root_layer().into(),
            entries: c.entries().to_vec(),
        };
        store
            .commit_with_evidence(
                &draft,
                &ExpectedBase::Revision(head.info().revision().clone()),
                pin,
                None,
                &mut NoFault,
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_workspace_publication_retains_exact_reads_and_its_immutable_request() {
    let f = Fixture::new();
    f.rename("First");
    f.rename("Second");
    let source = f.store("draft").head().unwrap().unwrap().manifest_digest();
    f.ok(&[
        "workspace-publish",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("draft"),
    ]);
    let proof = read(&f.store("published").open().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(proof.fingerprint, source);
    assert_eq!(
        proof.footprint,
        Footprint::for_update(element(1), true, false, false, false, None).unwrap()
    );
    assert!(
        !proof
            .footprint
            .reads()
            .iter()
            .any(|e| e.target == Target::Element(element(2)))
    );
    let head = f.head();
    f.ok(&[
        "workspace-publish",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("draft"),
    ]);
    assert_eq!(f.head(), head);
}

#[test]
fn changeset_publication_carries_copy_source_reads_without_unrelated_model_reads() {
    let f = Fixture::new();
    f.ok(&[
        "copy-element",
        "--store",
        &f.path("draft"),
        "--id",
        &element(1).to_string(),
        "--new-id",
        &element(3).to_string(),
    ]);
    let (digest, changeset) = f.create();
    assert!(
        !changeset
            .dependencies
            .iter()
            .any(|d| d.target == DependencyTarget::Element(element(2)))
    );
    assert!(changeset.dependencies.contains(&Dependency {
        target: DependencyTarget::Element(element(1)),
        group: DependencyGroup::ShapeInputs,
        access: Access::Read
    }));
    let (code, json) = f.apply(digest);
    assert_eq!(code, 0, "{json:?}");
    let proof = read(&f.store("published").open().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(proof.fingerprint, digest);
    assert_eq!(
        proof.footprint,
        Footprint::for_copy(element(1), element(3), None, None).unwrap()
    );
}

#[test]
fn narrowed_dependencies_or_an_incomplete_source_interval_never_publish() {
    for variant in ["narrow", "widen", "reorder", "omit", "identity", "output"] {
        let f = Fixture::new();
        f.rename("First");
        f.rename("Second");
        let (_, mut changeset) = f.create();
        match variant {
            "narrow" => changeset
                .dependencies
                .retain(|d| !(d.group == DependencyGroup::Lifecycle && d.access == Access::Read)),
            "widen" => {
                changeset.dependencies.push(Dependency {
                    target: DependencyTarget::Element(element(2)),
                    group: DependencyGroup::Label,
                    access: Access::Read,
                });
                changeset.dependencies.sort();
            }
            "reorder" => changeset.operations.reverse(),
            "omit" => {
                changeset.operations.remove(0);
            }
            "identity" => {
                changeset.operations.first_mut().unwrap().op_id =
                    OpId::parse(&element(901).to_string()).unwrap()
            }
            _ => changeset.root_layer = "authored.usda".into(),
        }
        let forged = f.put(&changeset);
        let before = f.head();
        let (code, json) = f.apply(forged);
        assert_eq!(
            (code, text(&json, "code")),
            (2, "INTEGRITY_ERROR"),
            "{variant}: {json:?}"
        );
        assert_eq!(f.head(), before);
        let host_id = Uuid7::parse(&changeset.changeset_id.to_string()).unwrap();
        assert_eq!(
            f.store("published").changeset_claim(&host_id).unwrap(),
            Some(forged)
        );
    }
}

#[test]
fn legacy_intervals_keep_conservative_reads_and_explicitly_absent_destination_evidence() {
    for legacy_v1 in [false, true] {
        let f = Fixture::new();
        f.raw(1, None, false);
        f.rename("After legacy");
        let (mut digest, mut changeset) = f.create();
        assert!(changeset.dependencies.iter().any(|d| {
            d.target == DependencyTarget::Element(element(2)) && d.access == Access::Read
        }));
        if legacy_v1 {
            changeset
                .dependencies
                .retain(|d| d.group != DependencyGroup::Mark);
            assert!(
                changeset
                    .encode()
                    .unwrap()
                    .starts_with(b"BeamCraft/C01/Changeset/v1\0")
            );
            digest = f.put(&changeset);
        }
        let (code, json) = f.apply(digest);
        assert_eq!(code, 0, "{legacy_v1}: {json:?}");
        assert_eq!(read(&f.store("published").open().unwrap()).unwrap(), None);
    }
}

#[test]
fn an_absent_legacy_record_does_not_hide_a_later_malformed_record() {
    let f = Fixture::new();
    f.raw(1, None, false);
    f.raw(
        2,
        Some(Footprint::for_update(element(1), true, false, false, false, None).unwrap()),
        true,
    );
    let before = f.head();
    let (code, json) = f.run(&[
        "workspace-publish",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("draft"),
    ]);
    assert_eq!((code, text(&json, "code")), (2, "INTEGRITY_ERROR"));
    assert_eq!(f.head(), before);
}

#[test]
fn numbering_dependencies_survive_publication_without_reinterpreting_the_command() {
    let f = Fixture::new();
    let preview = f.ok(&["numbering-preview", "--store", &f.path("draft")]);
    f.ok(&[
        "numbering-approve",
        "--store",
        &f.path("draft"),
        "--preview-token",
        text(&preview, "preview_token"),
    ]);
    let (digest, changeset) = f.create();
    assert!(
        changeset
            .encode()
            .unwrap()
            .starts_with(b"BeamCraft/C01/Changeset/v1\0")
    );
    assert!(
        changeset
            .dependencies
            .iter()
            .any(|d| d.group == DependencyGroup::Label && d.access == Access::Write)
    );
    let (code, json) = f.apply(digest);
    assert_eq!(code, 0, "{json:?}");
    let captured = read(&f.store("published").open().unwrap())
        .unwrap()
        .unwrap();
    assert!(
        captured
            .footprint
            .writes()
            .iter()
            .any(|entry| entry.group == Group::Label)
    );
    // A09 currently declares label writes. Publication must carry the actual admitted command
    // footprint, rather than substitute the separate engine for_mark helper's contract.
    assert_eq!(
        captured.footprint,
        read(&f.store("draft").open().unwrap())
            .unwrap()
            .unwrap()
            .footprint
    );
}

#[test]
fn raw_aggregate_entry_work_is_bounded_even_when_entries_repeat() {
    let entries: Vec<_> = (0..MAX_FOOTPRINT_ENTRIES / 2)
        .map(|n| Entry {
            target: Target::Element(element(n + 1)),
            group: Group::Lifecycle,
        })
        .collect();
    let half = Footprint::from_entries(&entries, &[]).unwrap();
    for extra in [false, true] {
        let f = Fixture::new();
        f.raw(1, Some(half.clone()), false);
        f.raw(2, Some(half.clone()), false);
        if extra {
            f.raw(
                3,
                Some(Footprint::from_entries(&entries[..1], &[]).unwrap()),
                false,
            );
        }
        let before = f.head();
        let (code, json) = f.run(&[
            "workspace-publish",
            "--store",
            &f.path("published"),
            "--workspace-dir",
            &f.path("draft"),
        ]);
        if extra {
            assert_eq!((code, text(&json, "code")), (2, "LIMIT_EXCEEDED"));
            assert_eq!(f.head(), before);
        } else {
            assert_eq!(code, 0, "{json:?}");
            assert_eq!(
                read(&f.store("published").open().unwrap()).unwrap(),
                None,
                "unverified raw records cannot acquire precise provenance"
            );
        }
    }
}
