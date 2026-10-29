// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and an invented foreign layer, MPL-2.0.

//! U14 end to end: `inspect-history` and `compact-history` through the real binary, one fresh
//! process per call. The store is built by `create-plate` and `attach-reference` (real revisions
//! with meshes, quantities and a foreign layer); a compaction must leave the head readable with the
//! same plates, meshes and quantities, the foreign layer byte for byte, and refuse everything
//! unsafe without changing a byte.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_host::{
    revision::{CommitPoint, ExpectedBase, Injected, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// An invented foreign model: one scope, nothing from any product.
const SITE: &str = "#usda 1.0\n(\n    defaultPrim = \"Site\"\n    metersPerUnit = 1.0\n    upAxis = \"Y\"\n)\ndef Xform \"Site\"\n{\n    def Scope \"Block\"\n    {\n    }\n}\n";
const USER: &str = "019a7bf0-0000-7000-8000-00000000d201";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("u14-tests"),
        };
        fs::create_dir(fixture.shared.root.join("published")).unwrap();
        fs::create_dir(fixture.shared.root.join("workspaces")).unwrap();
        fs::create_dir(fixture.shared.root.join("sources")).unwrap();
        fixture
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn published(&self) -> String {
        self.path("published")
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
    /// A refusal: the exit code and the stable error code; no byte of the store may change.
    fn refused(&self, args: &[&str]) -> (i32, String) {
        let before = self.snapshot();
        let (code, body) = self.run(args);
        assert_ne!(code, 0, "{args:?}: {body:?}");
        assert_eq!(text(&body, "outcome"), "rejected", "{args:?}: {body:?}");
        assert_eq!(self.snapshot(), before, "{args:?} changed the store");
        (code, text(&body, "code"))
    }
    fn plate(&self, name: &str) {
        self.ok(&[
            "create-plate",
            "--store",
            &self.published(),
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--name",
            name,
        ]);
    }
    fn attach_reference(&self) {
        let source = self.shared.root.join("sources").join("site.usda");
        fs::write(&source, SITE).unwrap();
        self.ok(&[
            "attach-reference",
            "--store",
            &self.published(),
            "--source",
            source.to_str().unwrap(),
            "--name",
            "Site",
        ]);
    }
    /// Four plates and a foreign reference: five revisions.
    fn build(&self) {
        self.plate("P1");
        self.plate("P2");
        self.attach_reference();
        self.plate("P3");
        self.plate("P4");
    }
    fn stats(&self) -> Json {
        self.ok(&["inspect-history", "--store", &self.published()])
    }
    fn dry_run(&self, keep: &[&str], extra: &[&str]) -> Json {
        let published = self.published();
        let mut args = vec!["compact-history", "--store", published.as_str()];
        args.extend_from_slice(keep);
        args.push("--dry-run");
        args.extend_from_slice(extra);
        self.ok(&args)
    }
    /// Every file of a store directory with its bytes.
    fn snapshot_of(&self, name: &str) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    let key = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/");
                    out.insert(key, fs::read(&path).unwrap());
                }
            }
        }
        let root = self.shared.root.join(name);
        let mut out = BTreeMap::new();
        if root.exists() {
            walk(&root, &root, &mut out);
        }
        out
    }
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        self.snapshot_of("published")
    }
    fn blob_bytes_on_disk(&self) -> u64 {
        self.snapshot()
            .iter()
            .filter(|(path, _)| path.starts_with("blobs/"))
            .map(|(_, bytes)| u64::try_from(bytes.len()).unwrap())
            .sum()
    }
    fn blob_path(&self, digest: &str) -> PathBuf {
        self.shared
            .root
            .join("published")
            .join("blobs")
            .join(&digest[..2])
            .join(digest)
    }
    fn store_handle(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.shared.root.join("published")).unwrap())
    }
}

fn num(json: &Json, path: &[&str]) -> u64 {
    at(json, path).unsigned().unwrap()
}
fn flag(json: &Json, path: &[&str]) -> bool {
    at(json, path).boolean().unwrap()
}
fn list(json: &Json, path: &[&str]) -> Vec<Json> {
    at(json, path).array().unwrap().to_vec()
}

/// What a reader sees of the head: revision, plates (meshes, quantities) and references.
fn head_view(fx: &Fixture) -> (Json, Json, Json) {
    let store = fx.published();
    (
        fx.ok(&["open-revision", "--store", &store]),
        fx.ok(&["inspect-elements", "--store", &store]),
        fx.ok(&["inspect-reference", "--store", &store]),
    )
}

#[test]
fn stats_describe_the_store_and_change_nothing() {
    let fx = Fixture::new();
    fx.build();
    let before = fx.snapshot();
    let s = fx.stats();
    assert_eq!(text(&s, "outcome"), "history_stats");
    assert!(flag(&s, &["changes_nothing"]));
    assert_eq!(num(&s, &["revisions"]), 5);
    assert_eq!(
        (num(&s, &["first_sequence"]), num(&s, &["head_sequence"])),
        (1, 5)
    );
    assert!(!flag(&s, &["compacted"]));
    assert_eq!(num(&s, &["blobs", "bytes"]), fx.blob_bytes_on_disk());
    assert!(
        num(&s, &["reachable_from_head", "bytes"]) < num(&s, &["reachable_from_history", "bytes"])
    );
    assert_eq!(num(&s, &["unreachable", "blobs"]), 0);
    assert_eq!(num(&s, &["missing_blobs"]), 0);
    assert!(num(&s, &["revision_log", "bytes"]) > 0 && num(&s, &["ledger", "bytes"]) > 0);
    assert_eq!(num(&s, &["ledger", "open_intents"]), 0);
    assert!(!flag(&s, &["reachable_from_workspaces", "declared"]));
    assert_eq!(fx.snapshot(), before);
}

#[test]
fn a_dry_run_changes_no_byte_and_lists_the_undo_points_that_would_be_lost() {
    let fx = Fixture::new();
    fx.build();
    let before = fx.snapshot();
    let report = fx.dry_run(&["--keep-head"], &[]);
    assert_eq!(fx.snapshot(), before, "the dry run is byte-identical");
    assert_eq!(text(&report, "outcome"), "compaction_dry_run");
    assert!(flag(&report, &["changes_nothing"]) && flag(&report, &["would_proceed"]));
    assert_eq!(num(&report, &["revisions_removed"]), 4);
    assert_eq!(num(&report, &["revisions_retained"]), 1);
    assert_eq!(num(&report, &["first_sequence_after"]), 5);
    assert!(num(&report, &["bytes_freed"]) > 0 && num(&report, &["blobs_removed"]) > 0);
    let lost = list(&report, &["undo_history_lost", "listed"]);
    let sequences: Vec<u64> = lost.iter().map(|r| num(r, &["sequence"])).collect();
    assert_eq!(sequences, vec![1, 2, 3, 4]);
    assert!(lost.iter().all(|r| text(r, "op_id").len() == 36));
    assert_eq!(
        num(
            &report,
            &["undo_history_lost", "oldest_restorable_sequence"]
        ),
        5
    );
    assert_eq!(num(&report, &["ledger", "operation_ids_forgotten"]), 4);
    assert!(list(&report, &["blockers"]).is_empty());
    // `--keep N` keeps the last N.
    let three = fx.dry_run(&["--keep", "3"], &[]);
    assert_eq!(num(&three, &["revisions_removed"]), 2);
    assert_eq!(num(&three, &["first_sequence_after"]), 3);
    assert_eq!(fx.snapshot(), before);
}

#[test]
fn a_compaction_without_confirmation_or_a_backup_decision_is_refused() {
    let fx = Fixture::new();
    fx.build();
    let store = fx.published();
    let backup = fx.path("backup");
    let base = ["compact-history", "--store", store.as_str(), "--keep-head"];
    let with = |extra: &[&str]| -> Vec<String> {
        base.iter().chain(extra).map(ToString::to_string).collect()
    };
    let refuse = |extra: &[&str]| {
        let args = with(extra);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (_, code) = fx.refused(&args);
        assert_eq!(code, "INVALID_ARGUMENT", "{extra:?}");
    };
    refuse(&["--no-backup", "--no-workspaces"]); // no --yes
    refuse(&["--yes", "--no-workspaces"]); // no backup decision
    refuse(&[
        "--yes",
        "--no-backup",
        "--backup",
        &backup,
        "--no-workspaces",
    ]); // both
    refuse(&["--yes", "--no-backup"]); // workspaces not declared
    refuse(&[
        "--yes",
        "--no-backup",
        "--no-workspaces",
        "--workspace-dir",
        &fx.path("workspaces"),
    ]);
    refuse(&["--dry-run", "--yes"]);
    refuse(&["--dry-run", "--no-backup"]);
    refuse(&["--yes", "--no-backup", "--no-workspaces", "--bogus", "1"]);
    refuse(&["--yes", "--no-backup", "--no-workspaces", "--yes"]);
    refuse(&["--yes", "--no-backup", "--no-workspaces", "--keep", "2"]); // two selectors
    refuse(&["--yes", "--backup", "relative-dir", "--no-workspaces"]); // not absolute
    assert!(!Path::new(&backup).exists());
    let (_, code) = fx.refused(&[
        "compact-history",
        "--store",
        &store,
        "--yes",
        "--no-backup",
        "--no-workspaces",
    ]);
    assert_eq!(code, "INVALID_ARGUMENT", "no keep selector");
    for keep in ["0", "100001", "18446744073709551615", "-1", "many", ""] {
        let (_, code) = fx.refused(&[
            "compact-history",
            "--store",
            &store,
            "--keep",
            keep,
            "--yes",
            "--no-backup",
            "--no-workspaces",
        ]);
        assert_eq!(code, "INVALID_ARGUMENT", "--keep {keep}");
        let (_, code) = fx.refused(&[
            "compact-history",
            "--store",
            &store,
            "--keep",
            keep,
            "--dry-run",
        ]);
        assert_eq!(code, "INVALID_ARGUMENT", "dry-run --keep {keep}");
    }
    // The backup destination must be empty or absent, and outside the store.
    let occupied = fx.shared.root.join("occupied");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("keep.txt"), b"mine").unwrap();
    let (_, code) = fx.refused(&[
        "compact-history",
        "--store",
        &store,
        "--keep-head",
        "--yes",
        "--backup",
        occupied.to_str().unwrap(),
        "--no-workspaces",
    ]);
    assert_eq!(code, "INVALID_ARGUMENT");
    assert_eq!(fs::read(occupied.join("keep.txt")).unwrap(), b"mine");
    let inside = fx.shared.root.join("published").join("backup");
    let (_, code) = fx.refused(&[
        "compact-history",
        "--store",
        &store,
        "--keep-head",
        "--yes",
        "--backup",
        inside.to_str().unwrap(),
        "--no-workspaces",
    ]);
    assert_eq!(code, "INVALID_ARGUMENT");
    assert!(!inside.exists());
}

#[test]
fn compaction_keeps_the_head_meshes_quantities_and_the_foreign_layer() {
    let fx = Fixture::new();
    fx.build();
    let (open_before, plates_before, references_before) = head_view(&fx);
    let site_digest = {
        let foreign = list(&references_before, &["foreign_layers"]);
        text(&foreign[0], "digest")
    };
    let foreign_before = fs::read(fx.blob_path(&site_digest)).unwrap();
    assert_eq!(foreign_before, SITE.as_bytes());
    let store_before = fx.snapshot();
    let planned = fx.dry_run(&["--keep-head"], &["--no-workspaces"]);

    let backup = fx.path("backup");
    let done = fx.ok(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep-head",
        "--yes",
        "--backup",
        &backup,
        "--no-workspaces",
    ]);
    assert_eq!(text(&done, "outcome"), "compacted");
    assert!(!flag(&done, &["changes_nothing"]) && !flag(&done, &["nothing_to_do"]));
    // The run did what the dry run said it would.
    assert_eq!(
        num(&done, &["bytes_freed_now"]),
        num(&planned, &["bytes_freed"])
    );
    assert_eq!(
        num(&done, &["blobs_removed_now"]),
        num(&planned, &["blobs_removed"])
    );
    assert_eq!(num(&done, &["revisions_removed"]), 4);
    assert_eq!(
        fx.blob_bytes_on_disk(),
        num(&planned, &["bytes_before"]) - num(&planned, &["bytes_freed"])
    );

    // The backup is the whole store as it was, and it opens.
    assert_eq!(
        fx.snapshot_of("backup").len(),
        store_before.len() - usize::from(store_before.contains_key("maintenance.lock"))
    );
    let backup_open = fx.ok(&["open-revision", "--store", &backup]);
    assert_eq!(
        text(&backup_open, "revision_id"),
        text(&open_before, "revision_id")
    );
    let backup_stats = fx.ok(&["inspect-history", "--store", &backup]);
    assert_eq!(num(&backup_stats, &["revisions"]), 5);

    // The head is the same revision with the same number and the same content.
    let (open_after, plates_after, references_after) = head_view(&fx);
    assert_eq!(open_after, open_before);
    assert_eq!(
        plates_after, plates_before,
        "meshes and quantities are recomputed equal"
    );
    assert_eq!(references_after, references_before);
    assert_eq!(
        fs::read(fx.blob_path(&site_digest)).unwrap(),
        foreign_before
    );
    assert!(
        fx.ok(&[
            "verify-blob",
            "--store",
            &fx.published(),
            "--digest",
            &site_digest
        ])
        .get("digest")
        .is_ok()
    );

    // Statistics see a compacted store with only the head left.
    let s = fx.stats();
    assert!(flag(&s, &["compacted"]));
    assert_eq!(
        (num(&s, &["revisions"]), num(&s, &["first_sequence"])),
        (1, 5)
    );
    assert_eq!(num(&s, &["unreachable", "blobs"]), 0);
    assert_eq!(
        num(&s, &["blobs", "bytes"]),
        num(&s, &["reachable_from_head", "bytes"])
    );

    // Old operation ids are no longer recognised as accepted.
    let old_op = text(
        &list(&planned, &["undo_history_lost", "listed"])[0],
        "op_id",
    );
    let (_, outcome) = fx.run(&[
        "inspect-revision-outcome",
        "--store",
        &fx.published(),
        "--op-id",
        &old_op,
    ]);
    assert_ne!(text(&outcome, "outcome"), "outcome_committed");

    // The compacted store keeps taking revisions, and a repeat of the command changes nothing.
    fx.plate("P5");
    let after_edit = fx.ok(&["open-revision", "--store", &fx.published()]);
    assert_eq!(text(&after_edit, "sequence"), "6");
    let again = fx.ok(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep",
        "2",
        "--yes",
        "--no-backup",
        "--no-workspaces",
    ]);
    assert_eq!(num(&again, &["revisions_removed"]), 0);
    assert!(flag(&again, &["nothing_to_do"]));
    assert!(flag(&again, &["backup_declined"]));
    let settled = fx.snapshot();
    let repeat = fx.ok(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep",
        "2",
        "--yes",
        "--no-backup",
        "--no-workspaces",
    ]);
    assert!(flag(&repeat, &["nothing_to_do"]));
    assert_eq!(fx.snapshot(), settled, "a repeated run changes no byte");
}

#[test]
fn a_live_workspace_pins_its_base_revision() {
    let fx = Fixture::new();
    fx.build();
    let store = fx.published();
    let alice = fx.path("workspaces/alice");
    fx.ok(&[
        "create-workspace",
        "--store",
        &store,
        "--workspace-dir",
        &alice,
        "--user-id",
        USER,
    ]);
    // The published head moves on; the workspace stays on its fork point (sequence 5).
    fx.plate("P5");
    fx.plate("P6");
    let workspace_before = fx.snapshot_of("workspaces");

    let report = fx.dry_run(&["--keep-head"], &["--workspace-dir", &alice]);
    assert!(!flag(&report, &["would_proceed"]));
    let blockers = list(&report, &["blockers"]);
    assert_eq!(text(&blockers[0], "kind"), "workspace_base");
    assert_eq!(num(&blockers[0], &["base_sequence"]), 5);
    let pinned = list(&report, &["workspaces", "workspaces"]);
    assert_eq!(num(&pinned[0], &["base_sequence"]), 5);

    let (exit, code) = fx.refused(&[
        "compact-history",
        "--store",
        &store,
        "--keep-head",
        "--yes",
        "--no-backup",
        "--workspaces-root",
        &fx.path("workspaces"),
    ]);
    assert_eq!((exit, code.as_str()), (2, "DEPENDENCY_CONFLICT"));
    // Statistics show what the workspace holds.
    let s = fx.ok(&[
        "inspect-history",
        "--store",
        &store,
        "--workspace-dir",
        &alice,
    ]);
    assert!(flag(&s, &["reachable_from_workspaces", "declared"]));
    assert!(num(&s, &["reachable_from_workspaces", "reachable_bytes"]) > 0);

    // Keeping enough history lets the compaction through; the workspace is untouched and can
    // still publish (its base is the published revision before P5, which is retained).
    let done = fx.ok(&[
        "compact-history",
        "--store",
        &store,
        "--keep",
        "3",
        "--yes",
        "--no-backup",
        "--workspaces-root",
        &fx.path("workspaces"),
    ]);
    assert_eq!(num(&done, &["first_sequence_after"]), 5);
    assert_eq!(fx.snapshot_of("workspaces"), workspace_before);

    // A workspace's own draft store is never compacted.
    let (_, code) = fx.refused(&[
        "compact-history",
        "--store",
        &alice,
        "--keep-head",
        "--dry-run",
    ]);
    assert_eq!(code, "INVALID_ARGUMENT");
    let (_, code) = fx.refused(&[
        "compact-history",
        "--store",
        &alice,
        "--keep-head",
        "--yes",
        "--no-backup",
        "--no-workspaces",
    ]);
    assert_eq!(code, "INVALID_ARGUMENT");
    // An unusable workspace cannot be protected, so it refuses.
    let broken = fx.shared.root.join("workspaces").join("broken");
    fs::create_dir(&broken).unwrap();
    fs::write(broken.join("workspace.json"), b"not a workspace").unwrap();
    let (_, code) = fx.refused(&[
        "compact-history",
        "--store",
        &store,
        "--keep-head",
        "--yes",
        "--no-backup",
        "--workspaces-root",
        &fx.path("workspaces"),
    ]);
    assert_eq!(code, "BACKEND_UNAVAILABLE");
}

#[test]
fn an_unfinished_intent_blocks_the_compaction_until_recovery_runs() {
    let fx = Fixture::new();
    fx.build();
    // A commit that dies right after its intent reached the ledger.
    let store = fx.store_handle();
    let head = store.head().unwrap().unwrap();
    let content = head.manifest().content();
    let draft = RevisionDraft {
        project: content.project().clone(),
        workspace: content.workspace().clone(),
        revision: Uuid7::parse("0199e7a0-0000-7000-8000-0000000000f1").unwrap(),
        op: Uuid7::parse("0199e7a0-0000-7000-8000-0000000000f2").unwrap(),
        workspace_generation: content.workspace_generation() + 1,
        schema: content.schema(),
        root_layer: content.root_layer().to_owned(),
        entries: content.entries().to_vec(),
    };
    let crashed = catch_unwind(AssertUnwindSafe(|| {
        let _ = store.commit_with(
            &draft,
            &ExpectedBase::Revision(head.revision().clone()),
            None,
            &mut |point: CommitPoint| -> Result<(), Injected> {
                if point == CommitPoint::IntentRecorded {
                    panic!("simulated crash");
                }
                Ok(())
            },
        );
    }));
    assert!(crashed.is_err());

    let report = fx.dry_run(&["--keep-head"], &["--no-workspaces"]);
    assert!(!flag(&report, &["would_proceed"]));
    assert_eq!(
        text(&list(&report, &["blockers"])[0], "kind"),
        "open_intents"
    );
    let (exit, code) = fx.refused(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep-head",
        "--yes",
        "--no-backup",
        "--no-workspaces",
    ]);
    assert_eq!((exit, code.as_str()), (2, "DEPENDENCY_CONFLICT"));
    fx.ok(&[
        "recover-revision",
        "--store",
        &fx.published(),
        "--min-age-secs",
        "0",
    ]);
    let done = fx.ok(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep-head",
        "--yes",
        "--no-backup",
        "--no-workspaces",
    ]);
    assert_eq!(text(&done, "outcome"), "compacted");
}

#[test]
fn a_running_coordinator_holds_the_store() {
    let fx = Fixture::new();
    fx.build();
    let token = fx.shared.root.join("token");
    fs::write(&token, "a-synthetic-token-of-more-than-32-characters").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let client = format!("{USER}:{}", token.to_str().unwrap());
    let mut server = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args([
            "serve-coordinator",
            "--store",
            &fx.published(),
            "--workspaces-root",
            &fx.path("workspaces"),
            "--client",
            &client,
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(server.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let listening = beamcraft_json::decode(line.trim_end().as_bytes(), &Limits::DEFAULT).unwrap();
    let address = text(&listening, "address");

    let report = fx.dry_run(&["--keep-head"], &["--no-workspaces"]);
    assert!(!flag(&report, &["would_proceed"]));
    assert_eq!(
        text(&list(&report, &["blockers"])[0], "kind"),
        "store_in_use"
    );
    let (exit, code) = fx.refused(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep-head",
        "--yes",
        "--no-backup",
        "--no-workspaces",
    ]);
    assert_eq!((exit, code.as_str()), (3, "BACKEND_UNAVAILABLE"));

    // One request ends the server; then the store is free again.
    fx.ok(&[
        "inspect-coordinator-head",
        "--address",
        &address,
        "--token-file",
        token.to_str().unwrap(),
    ]);
    let mut rest = Vec::new();
    stdout.read_to_end(&mut rest).unwrap();
    assert_eq!(server.wait().unwrap().code(), Some(0));
    let done = fx.ok(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep-head",
        "--yes",
        "--no-backup",
        "--no-workspaces",
    ]);
    assert_eq!(text(&done, "outcome"), "compacted");
}

#[test]
fn a_damaged_retained_blob_refuses_and_a_restored_backup_serves_the_old_history() {
    let fx = Fixture::new();
    fx.build();
    let backup = fx.path("backup");
    fx.ok(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep",
        "2",
        "--yes",
        "--backup",
        &backup,
        "--no-workspaces",
    ]);
    // The backup still holds the whole history.
    let old = fx.ok(&["inspect-history", "--store", &backup]);
    assert_eq!(num(&old, &["revisions"]), 5);
    let head = fx.ok(&["open-revision", "--store", &fx.published()]);
    let layers = list(&head, &["layers"]);
    let digest = text(&layers[0], "digest");
    let blob = fx.blob_path(&digest);
    let good = fs::read(&blob).unwrap();
    fs::write(&blob, b"flipped").unwrap();
    let (_, code) = fx.refused(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep-head",
        "--yes",
        "--no-backup",
        "--no-workspaces",
    ]);
    assert_eq!(code, "INTEGRITY_ERROR");
    fs::remove_file(&blob).unwrap();
    let (_, code) = fx.refused(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep-head",
        "--dry-run",
    ]);
    assert_eq!(code, "INTEGRITY_ERROR");
    fs::write(&blob, good).unwrap();
    fx.ok(&[
        "compact-history",
        "--store",
        &fx.published(),
        "--keep-head",
        "--dry-run",
    ]);
}
