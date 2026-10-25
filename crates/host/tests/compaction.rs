// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U14 host-level tests of history statistics, compaction and garbage collection against the real
//! filesystem; no storage mocks. All layer bytes and identities are original synthetic values.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use beamcraft_host::compaction::{
    Blocker, CompactionError, CompactionPoint, Injected, Keep, NoFault, Pin, Request, Safeguard,
    compact, plan, stats,
};
use beamcraft_host::lease::StoreLease;
use beamcraft_host::ledger::{Outcome, RecoveryPolicy};
use beamcraft_host::revision::{
    CommitPoint, ExpectedBase, LayerEntry, LayerRole, RevisionDraft, RevisionError, RevisionStore,
    Uuid7,
};
use beamcraft_host::storage::BlobStore;
use beamcraft_model::{
    ErrorCode,
    digest::{ContentDigest, Sha256},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("u14-tests");
        fs::create_dir_all(&base).unwrap();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // The process id keeps nextest's process-per-test runs from colliding when two test
        // processes start within the same nanosecond (the counter is per process).
        let root = base.join(format!("test-{time}-{}-{id}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn store(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(&self.0).unwrap())
    }
    fn blob_path(&self, digest: &ContentDigest) -> PathBuf {
        let name = digest.to_string();
        self.0.join("blobs").join(&name[..2]).join(name)
    }
    /// Every file of the store with its bytes, keyed by the relative path.
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    let key = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.insert(key, fs::read(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.0, &self.0, &mut out);
        out
    }
    fn records(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.0.join("revisions"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_str().unwrap().to_owned())
            .filter(|n| !n.starts_with(".tmp-"))
            .collect();
        names.sort();
        names
    }
    fn blob_files(&self) -> Vec<String> {
        let mut names = Vec::new();
        for shard in fs::read_dir(self.0.join("blobs")).unwrap() {
            let shard = shard.unwrap().path();
            if shard.is_dir() {
                for blob in fs::read_dir(&shard).unwrap() {
                    names.push(blob.unwrap().file_name().to_str().unwrap().to_owned());
                }
            }
        }
        names.sort();
        names
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.to_string_lossy().contains("u14-tests"));
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn id(n: u32) -> Uuid7 {
    Uuid7::parse(&format!("0199e7a0-0000-7000-8000-{n:012x}")).unwrap()
}

fn digest(text: &str) -> ContentDigest {
    Sha256::digest(text.as_bytes()).unwrap()
}

/// The synthetic foreign layer: identical bytes in every revision, like an attached reference.
const FOREIGN: &[u8] = b"#usda 1.0\n# synthetic foreign reference layer\n";

/// Publishes the layers of generation `n`: three owned layers that change every revision and one
/// foreign layer that never does.
fn layers(store: &RevisionStore, n: u32) -> Vec<LayerEntry> {
    store.blobs().publish(FOREIGN).unwrap();
    let mut entries =
        vec![LayerEntry::from_bytes("weak.usda", LayerRole::External, FOREIGN).unwrap()];
    for (name, role) in [
        ("root.usda", LayerRole::Authored),
        ("authored.usda", LayerRole::Authored),
        ("derived.usda", LayerRole::Derived),
    ] {
        let bytes = format!("#usda 1.0\n# synthetic {name} generation {n}\n");
        store.blobs().publish(bytes.as_bytes()).unwrap();
        entries.push(LayerEntry::from_bytes(name, role, bytes.as_bytes()).unwrap());
    }
    entries
}

fn draft(entries: Vec<LayerEntry>, n: u32) -> RevisionDraft {
    RevisionDraft {
        project: id(1),
        workspace: id(2),
        revision: id(100 + n),
        op: id(200 + n),
        workspace_generation: 40 + u64::from(n),
        schema: digest("synthetic schema identity"),
        root_layer: "root.usda".into(),
        entries,
    }
}

fn expect(n: u32) -> ExpectedBase {
    if n == 0 {
        ExpectedBase::Initial
    } else {
        ExpectedBase::Revision(id(100 + n))
    }
}

/// Commits revisions `1..=count`.
fn build(store: &RevisionStore, count: u32) {
    for n in 1..=count {
        let entries = layers(store, n);
        store.commit(&draft(entries, n), &expect(n - 1)).unwrap();
    }
}

fn request<'a>(keep: Keep, pins: &'a [Pin]) -> Request<'a> {
    Request {
        keep,
        pins,
        ledger_tail: 0,
        orphan_grace_secs: 0,
    }
}

fn run(
    store: &RevisionStore,
    keep: Keep,
) -> Result<beamcraft_host::compaction::Compacted, CompactionError> {
    compact(
        store,
        &request(keep, &[]),
        true,
        Safeguard::NoBackup,
        &mut NoFault,
    )
}

fn every_blob_of(store: &RevisionStore, sequence: u64) -> Vec<ContentDigest> {
    let info = store.info_at(sequence).unwrap();
    let mut all = vec![info.manifest_digest()];
    all.extend(
        info.manifest()
            .content()
            .entries()
            .iter()
            .map(|e| e.digest()),
    );
    all
}

#[test]
fn stats_measure_the_store_and_change_nothing() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 4);
    // A blob nothing references (a failed commit leaves such blobs).
    store.blobs().publish(b"orphan bytes").unwrap();
    let before = fx.snapshot();

    let s = stats(&store, &[]).unwrap();
    assert_eq!((s.first_sequence, s.head_sequence, s.revisions), (1, 4, 4));
    assert_eq!(s.head_revision, id(104));
    // 4 revisions x (3 owned layers + 1 manifest) + 1 shared foreign layer + 1 orphan.
    assert_eq!(s.blob_count, 4 * 4 + 1 + 1);
    assert_eq!(s.unreachable_blob_count, 1);
    assert_eq!(s.unreachable_bytes, 12);
    assert_eq!(s.history_blob_count, 4 * 4 + 1);
    assert_eq!(s.head_blob_count, 5);
    assert!(s.head_bytes < s.history_bytes && s.history_bytes < s.blob_bytes);
    assert_eq!(
        s.unreachable_bytes * 10_000 / s.blob_bytes,
        s.unreachable_share_hundredths
    );
    assert_eq!(
        (s.missing_blobs, s.open_intents, s.leftover_records),
        (0, 0, 0)
    );
    assert!(!s.compacted && s.revision_log_bytes > 0 && s.ledger_bytes > 0);
    assert!(s.ledger_records >= 4);
    assert_eq!(s.workspace_bytes, None);

    let pin = Pin {
        label: "ws".into(),
        revision: id(102),
    };
    let with = stats(&store, std::slice::from_ref(&pin)).unwrap();
    assert_eq!(with.pins[0].sequence, Some(2));
    assert_eq!(with.workspace_blob_count, Some(5));
    assert_eq!(fx.snapshot(), before, "statistics are read-only");
}

#[test]
fn a_dry_run_changes_nothing_and_reports_exactly_what_a_real_run_does() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 5);
    store.blobs().publish(b"orphan bytes").unwrap();
    let before = fx.snapshot();

    let planned = plan(&store, &request(Keep::Head, &[])).unwrap();
    assert_eq!(fx.snapshot(), before, "the plan is read-only");
    assert!(planned.blockers.is_empty());
    assert_eq!(
        (planned.old_floor, planned.new_floor, planned.retained),
        (1, 5, 1)
    );
    let lost: Vec<u64> = planned.removed.iter().map(|r| r.sequence).collect();
    assert_eq!(lost, vec![1, 2, 3, 4]);
    assert_eq!(planned.removed[0].revision, id(101));
    assert_eq!(planned.removed[0].op, Some(id(201)));
    assert!(planned.removed.iter().all(|r| r.readable));
    assert_eq!(planned.records_to_remove, 4);
    assert_eq!(planned.ledger.forgotten_ops.len(), 4);
    assert!(planned.ledger.rotated && planned.ledger.bytes_after < planned.ledger.bytes_before);
    // Four old revisions x 4 blobs, plus the orphan; the shared foreign layer stays.
    assert_eq!(planned.blobs_to_delete, 16 + 1);

    let done = run(&store, Keep::Head).unwrap();
    assert_eq!(done.bytes_freed, planned.bytes_to_free);
    assert_eq!(done.blobs_removed, planned.blobs_to_delete);
    assert_eq!(done.records_removed, planned.records_to_remove);
    assert_eq!(done.plan.removed, planned.removed);
}

#[test]
fn a_real_run_needs_confirmation_and_changes_nothing_without_it() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 3);
    let before = fx.snapshot();
    let refused = compact(
        &store,
        &request(Keep::Head, &[]),
        false,
        Safeguard::NoBackup,
        &mut NoFault,
    );
    assert_eq!(refused.unwrap_err(), CompactionError::NotConfirmed);
    assert_eq!(fx.snapshot(), before);
}

#[test]
fn keep_head_keeps_the_head_id_sequence_and_every_retained_blob() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 5);
    let head_before = store.head().unwrap().unwrap();
    let head_blobs = every_blob_of(&store, 5);
    let foreign = digest(std::str::from_utf8(FOREIGN).unwrap());

    let done = run(&store, Keep::Head).unwrap();
    assert_eq!(done.plan.new_floor, 5);

    let fresh = fx.store();
    let head = fresh.head().unwrap().unwrap();
    assert_eq!(
        head, head_before,
        "the head keeps its id, sequence and manifest"
    );
    assert_eq!((head.sequence(), head.revision()), (5, &id(105)));
    assert_eq!(fresh.first_sequence().unwrap(), Some(5));
    fresh.open().unwrap();
    assert_eq!(fresh.open_at(5).unwrap().info(), &head);
    for old in 1..=4 {
        assert_eq!(fresh.info_at(old).unwrap_err(), RevisionError::Compacted);
        assert_eq!(RevisionError::Compacted.code(), ErrorCode::NotFound);
    }
    assert_eq!(fresh.sequence_of(&id(105)).unwrap(), Some(5));
    assert_eq!(fresh.sequence_of(&id(101)).unwrap(), None);
    assert_eq!(fx.records(), vec!["00000000000000000005", "floor"]);

    // Exactly the head's blobs are left: owned layers, manifest and the foreign layer, byte-exact.
    let mut expected: Vec<String> = head_blobs.iter().map(ToString::to_string).collect();
    expected.sort();
    assert_eq!(fx.blob_files(), expected);
    assert_eq!(fs::read(fx.blob_path(&foreign)).unwrap(), FOREIGN);

    // An operation of the removed history is no longer recognised, and a new one is provably new.
    assert_eq!(fresh.query_outcome(&id(201)).unwrap(), Outcome::NotFound);
    assert_eq!(fresh.query_outcome(&id(999)).unwrap(), Outcome::NotFound);

    // The log keeps working: the next commit chains on the compacted head.
    let entries = layers(&fresh, 6);
    let next = fresh.commit(&draft(entries, 6), &expect(5)).unwrap().info;
    assert_eq!(next.sequence(), 6);
    assert_eq!(next.manifest().base().unwrap().revision, id(105));
    fresh.open().unwrap();
    assert_eq!(fresh.info_at(5).unwrap().revision(), &id(105));
}

#[test]
fn keep_last_n_retains_the_newest_revisions_with_their_chain() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 6);
    let done = run(&store, Keep::Last(3)).unwrap();
    assert_eq!((done.plan.old_floor, done.plan.new_floor), (1, 4));
    assert_eq!(done.plan.retained, 3);
    assert_eq!(
        fx.records(),
        vec![
            "00000000000000000004",
            "00000000000000000005",
            "00000000000000000006",
            "floor"
        ]
    );
    let fresh = fx.store();
    for sequence in 4..=6 {
        let revision = fresh.open_at(sequence).unwrap();
        assert_eq!(revision.info().sequence(), sequence);
    }
    // The chain inside the retained run is still verified.
    assert_eq!(
        fresh
            .info_at(5)
            .unwrap()
            .manifest()
            .base()
            .unwrap()
            .revision,
        id(104)
    );
    // A keep count above the log changes no history.
    let again = run(&fresh, Keep::Last(100)).unwrap();
    assert_eq!((again.plan.old_floor, again.plan.new_floor), (4, 4));
    assert!(again.plan.removed.is_empty());
}

#[test]
fn zero_and_absurd_counts_are_refused_before_anything_is_read_or_written() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 3);
    let before = fx.snapshot();
    for keep in [Keep::Last(0), Keep::Last(100_001), Keep::Last(u64::MAX)] {
        let error = run(&store, keep).unwrap_err();
        assert!(
            matches!(error, CompactionError::InvalidRequest(_)),
            "{keep:?}"
        );
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        assert!(plan(&store, &request(keep, &[])).is_err());
    }
    let mut tail = request(Keep::Head, &[]);
    tail.ledger_tail = u64::MAX;
    assert!(matches!(
        plan(&store, &tail).unwrap_err(),
        CompactionError::InvalidRequest(_)
    ));
    assert_eq!(fx.snapshot(), before);
    // An empty store has nothing to compact.
    let empty = Fixture::new();
    assert_eq!(
        run(&empty.store(), Keep::Head).unwrap_err(),
        CompactionError::Revision(RevisionError::NoRevision)
    );
}

#[test]
fn a_workspace_based_on_a_revision_that_would_go_refuses_the_compaction() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 5);
    let before = fx.snapshot();
    let stale = Pin {
        label: "alice".into(),
        revision: id(102),
    };
    let current = Pin {
        label: "bob".into(),
        revision: id(105),
    };
    let unknown = Pin {
        label: "carol".into(),
        revision: id(999),
    };
    let pins = [stale.clone(), current, unknown];

    let planned = plan(&store, &request(Keep::Head, &pins)).unwrap();
    assert_eq!(
        planned.blockers,
        vec![Blocker::WorkspaceBase {
            label: "alice".into(),
            sequence: 2
        }]
    );
    assert_eq!(
        planned.pins[2].sequence, None,
        "an unknown base is reported, not fatal"
    );
    let error = compact(
        &store,
        &request(Keep::Head, &pins),
        true,
        Safeguard::NoBackup,
        &mut NoFault,
    )
    .unwrap_err();
    assert!(matches!(&error, CompactionError::Blocked(b) if b.len() == 1));
    assert_eq!(error.code(), ErrorCode::DependencyConflict);
    assert_eq!(fx.snapshot(), before);

    // Keeping enough history protects the same workspace.
    let ok = compact(
        &store,
        &request(Keep::Last(4), std::slice::from_ref(&stale)),
        true,
        Safeguard::NoBackup,
        &mut NoFault,
    )
    .unwrap();
    assert_eq!(ok.plan.new_floor, 2);
    assert_eq!(fx.store().sequence_of(&id(102)).unwrap(), Some(2));
}

#[test]
fn an_unfinished_intent_refuses_the_compaction_until_recovery_closes_it() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 3);
    let entries = layers(&store, 4);
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.commit_with(&draft(entries, 4), &expect(3), None, &mut |point| {
            if point == CommitPoint::IntentRecorded {
                panic!("simulated crash");
            }
            Ok(())
        });
    }));
    assert!(crashed.is_err());
    let before = fx.snapshot();

    let planned = plan(&store, &request(Keep::Head, &[])).unwrap();
    assert_eq!(planned.blockers, vec![Blocker::OpenIntents(1)]);
    let error = run(&store, Keep::Head).unwrap_err();
    assert!(matches!(&error, CompactionError::Blocked(b) if b == &[Blocker::OpenIntents(1)]));
    assert_eq!(fx.snapshot(), before);

    store.recover(RecoveryPolicy::quiescent()).unwrap();
    let done = run(&store, Keep::Head).unwrap();
    assert_eq!(done.plan.new_floor, 3);
    // The abandoned attempt's blobs were unreferenced and are collected too.
    assert!(
        !fx.blob_files()
            .contains(&digest("#usda 1.0\n# synthetic root.usda generation 4\n").to_string())
    );
}

#[test]
fn an_active_holder_of_the_store_refuses_the_compaction_and_a_compaction_refuses_commits() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 3);
    let before = fx.snapshot();
    {
        // What a running coordinator holds.
        let _serving = StoreLease::shared(store.blobs()).unwrap();
        let planned = plan(&store, &request(Keep::Head, &[])).unwrap();
        assert_eq!(planned.blockers, vec![Blocker::StoreInUse]);
        let error = run(&store, Keep::Head).unwrap_err();
        assert_eq!(error, CompactionError::Blocked(vec![Blocker::StoreInUse]));
        assert_eq!(error.code(), ErrorCode::BackendUnavailable);
        assert_eq!(fx.snapshot(), before);
    }
    // Under the exclusive lease no commit and no recovery can run.
    let held = StoreLease::exclusive(store.blobs()).unwrap();
    let entries = layers(&store, 4);
    assert_eq!(
        store.commit(&draft(entries, 4), &expect(3)).unwrap_err(),
        RevisionError::StoreBusy
    );
    assert_eq!(
        store.recover(RecoveryPolicy::quiescent()).unwrap_err(),
        RevisionError::StoreBusy
    );
    assert_eq!(
        RevisionError::StoreBusy.code(),
        ErrorCode::BackendUnavailable
    );
    drop(held);
    run(&store, Keep::Head).unwrap();
}

#[test]
fn a_damaged_or_missing_retained_blob_refuses_the_compaction_and_a_removed_one_does_not() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 4);
    let before = fx.snapshot();
    let head_layer = digest("#usda 1.0\n# synthetic derived.usda generation 4\n");

    fs::write(fx.blob_path(&head_layer), b"flipped").unwrap();
    let error = run(&store, Keep::Head).unwrap_err();
    assert!(matches!(
        error,
        CompactionError::Revision(RevisionError::CorruptBlob { .. })
    ));
    assert_eq!(error.code(), ErrorCode::IntegrityError);
    fs::remove_file(fx.blob_path(&head_layer)).unwrap();
    assert!(matches!(
        run(&store, Keep::Head).unwrap_err(),
        CompactionError::Revision(RevisionError::MissingBlob { .. })
    ));
    assert!(plan(&store, &request(Keep::Head, &[])).is_err());
    fs::write(
        fx.blob_path(&head_layer),
        b"#usda 1.0\n# synthetic derived.usda generation 4\n",
    )
    .unwrap();
    assert_eq!(fx.snapshot(), before, "nothing changed while refusing");

    // Damage in a revision that is being removed does not stop the compaction.
    let doomed = digest("#usda 1.0\n# synthetic derived.usda generation 1\n");
    fs::remove_file(fx.blob_path(&doomed)).unwrap();
    let done = run(&store, Keep::Head).unwrap();
    assert_eq!(done.plan.new_floor, 4);
    fx.store().open().unwrap();
}

#[test]
fn an_unreadable_removed_revision_is_listed_and_removed() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 4);
    let manifest = store.info_at(2).unwrap().manifest_digest();
    fs::remove_file(fx.blob_path(&manifest)).unwrap();
    let planned = plan(&store, &request(Keep::Head, &[])).unwrap();
    let broken: Vec<u64> = planned
        .removed
        .iter()
        .filter(|r| !r.readable)
        .map(|r| r.sequence)
        .collect();
    assert_eq!(broken, vec![2]);
    assert_eq!(planned.removed[1].op, None);
    run(&store, Keep::Head).unwrap();
    fx.store().open().unwrap();
}

#[test]
fn young_unreferenced_blobs_survive_the_grace_period_and_old_ones_do_not() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 2);
    store.blobs().publish(b"stored a moment ago").unwrap();
    let mut grace = request(Keep::Head, &[]);
    grace.orphan_grace_secs = 3600;
    let planned = plan(&store, &grace).unwrap();
    assert_eq!(planned.protected_orphans, 1);
    // Only the first revision's own blobs are collected.
    assert_eq!(planned.blobs_to_delete, 4);
    let done = compact(&store, &grace, true, Safeguard::NoBackup, &mut NoFault).unwrap();
    assert_eq!(done.blobs_removed, 4);
    assert!(
        fx.blob_files()
            .contains(&digest("stored a moment ago").to_string())
    );
    run(&store, Keep::Head).unwrap();
    assert!(
        !fx.blob_files()
            .contains(&digest("stored a moment ago").to_string())
    );
}

#[test]
fn the_ledger_keeps_retained_records_and_a_configurable_tombstone_tail() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 6);
    let mut keep_tail = request(Keep::Last(2), &[]);
    keep_tail.ledger_tail = 2;
    let planned = plan(&store, &keep_tail).unwrap();
    assert_eq!(planned.new_floor, 5);
    assert_eq!(planned.ledger.tombstones, 2);
    assert_eq!(planned.ledger.forgotten_ops, vec![id(201), id(202)]);
    compact(&store, &keep_tail, true, Safeguard::NoBackup, &mut NoFault).unwrap();

    let fresh = fx.store();
    let committed = |op: u32| fresh.query_outcome(&id(200 + op)).unwrap();
    // Retained revisions: proven by the root.
    assert!(matches!(
        committed(5),
        Outcome::Committed { sequence: 5, .. }
    ));
    assert!(matches!(
        committed(6),
        Outcome::Committed { sequence: 6, .. }
    ));
    // Tombstones: accepted, revision gone, a retry is refused rather than applied twice.
    assert!(matches!(
        committed(3),
        Outcome::Committed { sequence: 3, .. }
    ));
    assert!(matches!(
        committed(4),
        Outcome::Committed { sequence: 4, .. }
    ));
    let entries = layers(&fresh, 4);
    assert_eq!(
        fresh.commit(&draft(entries, 4), &expect(6)).unwrap_err(),
        RevisionError::OpIdReuse
    );
    // Beyond the tail: no longer recognised as accepted.
    assert_eq!(committed(2), Outcome::NotFound);
    assert_eq!(committed(1), Outcome::NotFound);
    // Recovery sees no disagreement with the root.
    let report = fresh.recover(RecoveryPolicy::quiescent()).unwrap();
    assert!(report.disagreements.is_empty());
    assert!(report.is_noop());
}

#[test]
fn compaction_is_idempotent() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 5);
    store.blobs().publish(b"orphan bytes").unwrap();
    run(&store, Keep::Last(2)).unwrap();
    let after_first = fx.snapshot();

    let planned = plan(&store, &request(Keep::Last(2), &[])).unwrap();
    assert!(planned.removed.is_empty());
    assert_eq!(
        (
            planned.records_to_remove,
            planned.blobs_to_delete,
            planned.bytes_to_free
        ),
        (0, 0, 0)
    );
    assert!(!planned.ledger.rotated);
    let again = run(&store, Keep::Last(2)).unwrap();
    assert_eq!(
        (
            again.records_removed,
            again.blobs_removed,
            again.bytes_freed
        ),
        (0, 0, 0)
    );
    assert_eq!(fx.snapshot(), after_first, "a repeated run changes no byte");
    run(&store, Keep::Head).unwrap();
    assert_eq!(run(&store, Keep::Head).unwrap().blobs_removed, 0);
}

#[test]
fn a_failed_backup_refuses_the_compaction_before_anything_changes() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 3);
    let before = fx.snapshot();
    let mut backup = || {
        Err(CompactionError::SafeguardFailed(
            ErrorCode::BackendUnavailable,
        ))
    };
    let error = compact(
        &store,
        &request(Keep::Head, &[]),
        true,
        Safeguard::Backup(&mut backup),
        &mut NoFault,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::BackendUnavailable);
    assert_eq!(fx.snapshot(), before);
}

#[test]
fn a_crash_at_every_step_leaves_the_old_or_the_new_store_and_a_rerun_finishes() {
    // First count the steps of an uninterrupted run.
    let counted = Fixture::new();
    let mut points = Vec::new();
    {
        let store = counted.store();
        build(&store, 5);
        store.blobs().publish(b"orphan bytes").unwrap();
        compact(
            &store,
            &request(Keep::Last(2), &[]),
            true,
            Safeguard::NoBackup,
            &mut |p| {
                points.push(p);
                Ok(())
            },
        )
        .unwrap();
    }
    assert_eq!(points.len(), 23, "{points:?}");
    let commit_point = points
        .iter()
        .position(|p| *p == CompactionPoint::FloorInstalled)
        .unwrap();
    let final_records = counted.records();
    let final_blobs = counted.blob_files();

    for (crash_at, point) in points.iter().enumerate() {
        let fx = Fixture::new();
        let store = fx.store();
        build(&store, 5);
        store.blobs().publish(b"orphan bytes").unwrap();
        let old_head = store.head().unwrap().unwrap();
        let old_manifest_of_two = store.info_at(2).unwrap();

        let mut calls = 0usize;
        let result = compact(
            &store,
            &request(Keep::Last(2), &[]),
            true,
            Safeguard::NoBackup,
            &mut |_| {
                let now = calls;
                calls += 1;
                if now == crash_at {
                    Err(Injected)
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(
            result.unwrap_err(),
            CompactionError::Interrupted(*point),
            "step {crash_at}"
        );

        // A fresh handle, as after a restart: the head is the same complete revision.
        let fresh = fx.store();
        let head = fresh.head().unwrap().unwrap();
        assert_eq!(head, old_head, "step {crash_at}");
        fresh.open().unwrap();
        // Before the commit point the whole old history reads; after it the new one does. Never
        // a mix: the revision at the new floor and the head always read.
        assert!(
            fresh.open_at(4).is_ok() && fresh.open_at(5).is_ok(),
            "step {crash_at}"
        );
        if crash_at < commit_point {
            assert_eq!(fresh.first_sequence().unwrap(), Some(1), "step {crash_at}");
            assert_eq!(fresh.info_at(2).unwrap(), old_manifest_of_two);
            fresh.open_at(1).unwrap();
        } else {
            assert_eq!(fresh.first_sequence().unwrap(), Some(4), "step {crash_at}");
            assert_eq!(fresh.info_at(2).unwrap_err(), RevisionError::Compacted);
        }
        // Recovery agrees with the root and does not mistake the ledger for damage.
        let report = fresh.recover(RecoveryPolicy::quiescent()).unwrap();
        assert!(report.disagreements.is_empty(), "step {crash_at}");
        assert!(report.root_error.is_none(), "step {crash_at}");
        assert!(matches!(
            fresh.query_outcome(&id(205)).unwrap(),
            Outcome::Committed { .. }
        ));
        let s = stats(&fresh, &[]).unwrap();
        assert_eq!(s.head_sequence, 5, "step {crash_at}");
        assert_eq!(s.missing_blobs, 0, "step {crash_at}");

        // Running the same compaction again finishes the work, to the same final state.
        run(&fresh, Keep::Last(2)).unwrap();
        assert_eq!(fx.records(), final_records, "step {crash_at}");
        assert_eq!(fx.blob_files(), final_blobs, "step {crash_at}");
        let finished = plan(&fresh, &request(Keep::Last(2), &[])).unwrap();
        assert_eq!(
            (
                finished.records_to_remove,
                finished.blobs_to_delete,
                finished.ledger.rotated
            ),
            (0, 0, false),
            "step {crash_at}"
        );
        fresh.open().unwrap();
        // And the store takes new commits.
        let entries = layers(&fresh, 6);
        fresh.commit(&draft(entries, 6), &expect(5)).unwrap();
    }
}

#[test]
fn a_torn_or_forged_floor_marker_and_a_gap_in_the_run_are_refused() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 5);
    run(&store, Keep::Last(3)).unwrap();
    let marker = fx.0.join("revisions").join("floor");
    let good = fs::read(&marker).unwrap();

    for bad in [
        &b"beamcraft-history-floor 1\nsequence 3\n"[..],
        b"garbage",
        b"",
    ] {
        fs::write(&marker, bad).unwrap();
        assert_eq!(fx.store().head().unwrap_err(), RevisionError::TornRoot);
    }
    // A marker that names another revision than the record at its sequence.
    let forged = String::from_utf8(good.clone())
        .unwrap()
        .replace(&id(103).to_string(), &id(777).to_string());
    fs::write(&marker, forged).unwrap();
    assert_eq!(fx.store().open().unwrap_err(), RevisionError::RootMismatch);
    // A floor of 1 is never a marker.
    fs::write(
        &marker,
        String::from_utf8(good.clone())
            .unwrap()
            .replace("sequence 3", "sequence 1"),
    )
    .unwrap();
    assert_eq!(fx.store().head().unwrap_err(), RevisionError::TornRoot);
    fs::write(&marker, &good).unwrap();
    fx.store().open().unwrap();

    // A record missing inside the retained run.
    let middle = fx.0.join("revisions").join(format!("{:020}", 4));
    let kept = fs::read(&middle).unwrap();
    fs::remove_file(&middle).unwrap();
    assert_eq!(fx.store().head().unwrap_err(), RevisionError::TornRoot);
    fs::write(&middle, kept).unwrap();
    // A marker without the record it names.
    let floor_record = fx.0.join("revisions").join(format!("{:020}", 3));
    let floor_bytes = fs::read(&floor_record).unwrap();
    fs::remove_file(&floor_record).unwrap();
    assert_eq!(fx.store().head().unwrap_err(), RevisionError::TornRoot);
    fs::write(&floor_record, floor_bytes).unwrap();
    fx.store().open().unwrap();
}

#[test]
fn leftover_records_below_the_floor_are_ignored_and_then_removed() {
    let fx = Fixture::new();
    let store = fx.store();
    build(&store, 4);
    let old_record = fs::read(fx.0.join("revisions").join(format!("{:020}", 1))).unwrap();
    run(&store, Keep::Last(2)).unwrap();
    // Put a stale record back, as an interrupted removal would leave it.
    fs::write(
        fx.0.join("revisions").join(format!("{:020}", 1)),
        old_record,
    )
    .unwrap();
    assert_eq!(fx.store().first_sequence().unwrap(), Some(3));
    assert_eq!(stats(&fx.store(), &[]).unwrap().leftover_records, 1);
    fx.store().open().unwrap();
    let done = run(&store, Keep::Last(2)).unwrap();
    assert_eq!(done.records_removed, 1);
    assert_eq!(
        fx.records(),
        vec!["00000000000000000003", "00000000000000000004", "floor"]
    );
}
