// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, stores and metadata, MPL-2.0.

//! C03.5: the accepted base pin and the local published prefix are different coordinates.
//! These tests construct that boundary explicitly; they do not claim to exercise a rebase API.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_host::revision::{RevisionInfo, RevisionStore};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::changeset::Changeset;
use beamcraft_model::digest::{ContentDigest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const USER: &str = "019a7bf0-0000-7000-8000-000000000099";
const ONE: &str = "019a7bf0-0000-7000-8000-000000000001";
const TWO: &str = "019a7bf0-0000-7000-8000-000000000002";
fn text<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key).unwrap().string().unwrap()
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "changeset-base-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        let f = Self(root);
        for (id, name) in [(ONE, "Base one"), (TWO, "Base two")] {
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
                id,
                "--name",
                name,
            ]);
        }
        f.ok(&[
            "create-workspace",
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
    fn rename(&self, store: &str, id: &str, name: &str) {
        self.ok(&[
            "update-element",
            "--store",
            &self.path(store),
            "--id",
            id,
            "--name",
            name,
        ]);
    }
    fn head(&self, store: &str) -> RevisionInfo {
        self.store(store).head().unwrap().unwrap()
    }
    fn pin(&self, revision: &str, manifest: ContentDigest) {
        let path = self.0.join("draft/workspace.json");
        let old = beamcraft_cli::workspace::read_meta(&self.0.join("draft")).unwrap();
        let bytes = fs::read_to_string(&path)
            .unwrap()
            .replace(
                &format!("base_revision={}", old.base_revision),
                &format!("base_revision={revision}"),
            )
            .replace(
                &format!("base_manifest={}", old.base_manifest),
                &format!("base_manifest={manifest}"),
            );
        fs::write(path, bytes).unwrap();
    }
    fn create_args(&self) -> Vec<String> {
        vec![
            "create-changeset".into(),
            "--store".into(),
            self.path("published"),
            "--workspace-dir".into(),
            self.path("draft"),
        ]
    }
    fn create(&self) -> (ContentDigest, Changeset) {
        let args = self.create_args();
        let json = self.ok(&args.iter().map(String::as_str).collect::<Vec<_>>());
        let digest = text(json.get("changeset").unwrap(), "digest")
            .parse()
            .unwrap();
        let changeset = beamcraft_cli::changeset::read_changeset(&self.store("draft"), &digest)
            .unwrap()
            .changeset;
        (digest, changeset)
    }
    fn names(&self, store: &str) -> Vec<String> {
        let json = self.ok(&["inspect-elements", "--store", &self.path(store)]);
        let mut names: Vec<_> = json
            .get("plates")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|plate| text(plate, "name").to_owned())
            .collect();
        names.sort();
        names
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_new_accepted_base_does_not_mark_unsent_operations_published_or_use_old_layer_digests() {
    let f = Fixture::new();
    let old = f.head("published");
    f.rename("draft", ONE, "Mine");
    f.rename("published", TWO, "Theirs");
    let accepted = f.head("published");
    // Materialize a complete combined draft with real authoring commands, then set the new
    // accepted pin while retaining the old local prefix. Runtime rebase is tested separately.
    f.rename("draft", TWO, "Theirs");
    f.pin(accepted.revision().as_str(), accepted.manifest_digest());
    assert_ne!(
        old.manifest().content().entries(),
        accepted.manifest().content().entries()
    );
    let (digest, changeset) = f.create();
    assert_eq!(
        changeset.base_revision.to_string(),
        accepted.revision().as_str()
    );
    assert_eq!(changeset.base_manifest, accepted.manifest_digest());
    assert_eq!(
        changeset.operations.len(),
        2,
        "all original unsent operations stay unsent"
    );
    assert_eq!(
        beamcraft_cli::workspace::read_meta(&f.0.join("draft"))
            .unwrap()
            .synced_sequence,
        1
    );
    for layer in &changeset.layers {
        let base = accepted
            .manifest()
            .content()
            .entries()
            .iter()
            .find(|entry| entry.name() == layer.name)
            .unwrap();
        assert_eq!(
            layer.base,
            Some(base.digest()),
            "{} must use the accepted pin",
            layer.name
        );
    }
    f.ok(&[
        "apply-changeset",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("draft"),
        "--changeset",
        &digest.to_string(),
        "--actor-id",
        USER,
    ]);
    assert_eq!(f.names("published"), ["Mine", "Theirs"]);
    assert_eq!(f.names("draft"), ["Mine", "Theirs"]);
}

#[test]
fn a_stale_workspace_packages_the_pinned_historical_base_instead_of_the_current_head() {
    let f = Fixture::new();
    let pinned = f.head("published");
    f.rename("draft", ONE, "Unsent");
    f.rename("published", TWO, "Accepted later");
    let before = f.head("published");
    let (digest, changeset) = f.create();
    assert_eq!(changeset.base_manifest, pinned.manifest_digest());
    for layer in &changeset.layers {
        assert_eq!(
            layer.base,
            pinned
                .manifest()
                .content()
                .entries()
                .iter()
                .find(|entry| entry.name() == layer.name)
                .map(|entry| entry.digest())
        );
    }
    let (code, json) = f.run(&[
        "apply-changeset",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("draft"),
        "--changeset",
        &digest.to_string(),
        "--actor-id",
        USER,
    ]);
    assert_eq!((code, text(&json, "code")), (2, "BASE_REVISION_MISMATCH"));
    assert_eq!(f.head("published"), before);
    assert_eq!(f.names("draft"), ["Base two", "Unsent"]);
}

#[test]
fn a_changeset_cannot_end_before_the_recorded_rebase_boundary() {
    use beamcraft_cli::workspace::{WorkspaceState, read_meta};
    let f = Fixture::new();
    f.rename("draft", ONE, "Mine");
    let (old_digest, _) = f.create();
    f.rename("draft", TWO, "Also retained");
    // Isolate the metadata/publication boundary with real committed edits. This fixture does
    // not claim a runtime rebase: the old payload names a valid earlier source interval.
    let dir = f.0.join("draft");
    let old_meta = read_meta(&dir).unwrap();
    let mut rebased = old_meta.clone();
    rebased.rebase_sequence = f.head("draft").sequence();
    WorkspaceState::open(&dir)
        .unwrap()
        .compare_and_swap(Some(&old_meta), &rebased)
        .unwrap();
    let published = f.head("published");
    let (code, json) = f.run(&[
        "apply-changeset",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("draft"),
        "--changeset",
        &old_digest.to_string(),
        "--actor-id",
        USER,
    ]);
    assert_eq!(code, 2, "{json:?}");
    assert_eq!(text(&json, "code"), "INTEGRITY_ERROR");
    assert_eq!(f.head("published"), published);
    assert_eq!(read_meta(&dir).unwrap(), rebased);
    let status = f.ok(&[
        "inspect-workspace",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("draft"),
    ]);
    assert_eq!(
        text(status.get("workspace").unwrap(), "rebase_sequence"),
        rebased.rebase_sequence.to_string()
    );
    let (digest, _) = f.create();
    f.ok(&[
        "apply-changeset",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("draft"),
        "--changeset",
        &digest.to_string(),
        "--actor-id",
        USER,
    ]);
    assert_eq!(f.names("published"), ["Also retained", "Mine"]);
    assert_eq!(
        read_meta(&dir).unwrap().rebase_sequence,
        rebased.rebase_sequence
    );
    let mut future = read_meta(&dir).unwrap();
    let old = future.clone();
    future.rebase_sequence = f.head("draft").sequence() + 1;
    WorkspaceState::open(&dir)
        .unwrap()
        .compare_and_swap(Some(&old), &future)
        .unwrap();
    let published = f.head("published");
    let draft = f.head("draft");
    for command in ["inspect-workspace", "publish-workspace", "create-changeset"] {
        let (code, json) = f.run(&[
            command,
            "--store",
            &f.path("published"),
            "--workspace-dir",
            &f.path("draft"),
        ]);
        assert_eq!(code, 2, "{command}: {json:?}");
        assert_eq!(text(&json, "code"), "INTEGRITY_ERROR");
        assert_eq!(f.head("published"), published);
        assert_eq!(f.head("draft"), draft);
    }
}

#[test]
fn wrong_missing_or_corrupt_accepted_pins_refuse_packaging_without_changing_either_head() {
    for variant in 0..4 {
        let f = Fixture::new();
        f.rename("draft", ONE, "Unsent");
        let base = f.head("published");
        match variant {
            0 => f.pin(
                base.revision().as_str(),
                Sha256::digest(b"wrong manifest").unwrap(),
            ),
            1 => f.pin(USER, base.manifest_digest()),
            2 => {
                // Keep the head log, but damage the pinned manifest through a raw fixture.
                let digest = base.manifest_digest().to_string();
                let path = f.0.join("published/blobs").join(&digest[..2]).join(&digest);
                assert!(path.is_file(), "{path:?}");
                fs::write(path, b"corrupt manifest").unwrap();
            }
            _ => {
                let path = f.0.join("draft/workspace.json");
                let meta = beamcraft_cli::workspace::read_meta(&f.0.join("draft")).unwrap();
                let bytes = fs::read_to_string(&path).unwrap().replace(
                    &format!("workspace_id={}", meta.workspace_id),
                    &format!("workspace_id={USER}"),
                );
                fs::write(path, bytes).unwrap();
            }
        }
        let draft = f.head("draft");
        let metadata = fs::read(f.0.join("draft/workspace.json")).unwrap();
        let log = fs::read_dir(f.0.join("published/revisions"))
            .unwrap()
            .count();
        let args = f.create_args();
        let (code, json) = f.run(&args.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(code, 2, "{variant}: {json:?}");
        assert_eq!(text(&json, "code"), "INTEGRITY_ERROR");
        assert_eq!(f.head("draft"), draft);
        assert_eq!(
            fs::read(f.0.join("draft/workspace.json")).unwrap(),
            metadata
        );
        assert_eq!(
            fs::read_dir(f.0.join("published/revisions"))
                .unwrap()
                .count(),
            log
        );
        if variant != 2 {
            assert_eq!(f.head("published"), base);
        }
    }
}

#[test]
fn a_valid_pin_from_another_project_cannot_relabel_the_drafts_project() {
    let f = Fixture::new();
    let other = Fixture::new();
    f.rename("draft", ONE, "Unsent");
    let accepted = other.head("published");
    assert_ne!(
        accepted.manifest().content().project(),
        f.head("draft").manifest().content().project()
    );
    f.pin(accepted.revision().as_str(), accepted.manifest_digest());
    let published = other.head("published");
    let draft = f.head("draft");
    let (code, json) = f.run(&[
        "create-changeset",
        "--store",
        &other.path("published"),
        "--workspace-dir",
        &f.path("draft"),
    ]);
    assert_eq!(code, 2, "{json:?}");
    assert_eq!(text(&json, "code"), "INTEGRITY_ERROR");
    assert_eq!(other.head("published"), published);
    assert_eq!(f.head("draft"), draft);
    assert_eq!(f.names("draft"), ["Base two", "Unsent"]);
}

#[test]
fn a_compacted_accepted_base_is_unavailable_even_when_the_old_local_snapshot_survives() {
    use beamcraft_host::compaction::{self, Keep, NoFault, Request, Safeguard};
    let f = Fixture::new();
    let old = f.head("published");
    f.rename("draft", ONE, "Unsent");
    f.rename("published", TWO, "Accepted later");
    // Deliberately omit this synthetic workspace's pin when pruning its private store.
    // A real compaction that declares the pin is already refused by U14.
    compaction::compact(
        &f.store("published"),
        &Request {
            keep: Keep::Head,
            pins: &[],
            ledger_tail: 0,
            orphan_grace_secs: 0,
        },
        true,
        Safeguard::NoBackup,
        &mut NoFault,
    )
    .unwrap();
    assert_eq!(
        f.store("published").sequence_of(old.revision()).unwrap(),
        None
    );
    assert!(
        f.store("draft").open_at(1).is_ok(),
        "the old local copy still exists"
    );
    let published = f.head("published");
    let draft = f.head("draft");
    let args = f.create_args();
    let (code, json) = f.run(&args.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(code, 2, "{json:?}");
    assert_eq!(text(&json, "code"), "INTEGRITY_ERROR");
    assert_eq!(f.head("published"), published);
    assert_eq!(f.head("draft"), draft);
    assert_eq!(f.names("draft"), ["Base two", "Unsent"]);
}
