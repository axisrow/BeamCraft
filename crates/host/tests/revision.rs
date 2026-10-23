// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U06 public-API tests against the real filesystem; no storage mocks. All layer bytes and
//! identities are original synthetic values.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Barrier,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use beamcraft_host::revision::{
    CommitOutcome, CommitPoint, ContentRejection, ContentVerifier, ExpectedBase, IndexEvidence,
    Injected, LayerEntry, LayerRole, MAX_CONTENT_VERIFY_BYTES, Manifest, REPLAY_LOOKBACK,
    RevisionDraft, RevisionError, RevisionInfo, RevisionStore, Uuid7, VerifyRequest,
};
use beamcraft_host::storage::{BlobStore, MAX_BLOB_BYTES, StorageError};
use beamcraft_model::{
    ErrorCode,
    digest::{ContentDigest, Sha256},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn base_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("u06-tests")
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let base = base_dir();
        fs::create_dir_all(&base).unwrap();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = base.join(format!("test-{time}-{id}"));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn store(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(&self.0).unwrap())
    }
    fn log(&self, sequence: u64) -> PathBuf {
        self.0.join("revisions").join(format!("{sequence:020}"))
    }
    fn blob(&self, digest: ContentDigest) -> PathBuf {
        let name = digest.to_string();
        self.0.join("blobs").join(&name[..2]).join(name)
    }
    fn temp_leftovers(&self) -> usize {
        fs::read_dir(self.0.join("revisions"))
            .map(|d| {
                d.filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_str()
                        .unwrap()
                        .starts_with(".tmp-")
                })
                .count()
            })
            .unwrap_or(0)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let base = base_dir();
        assert!(self.0.is_absolute() && self.0.starts_with(&base) && self.0 != base);
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn id(n: u32) -> Uuid7 {
    Uuid7::parse(&format!("0199e7a0-0000-7000-8000-{n:012x}")).unwrap()
}
fn digest(text: &str) -> ContentDigest {
    Sha256::digest(text.as_bytes()).unwrap()
}

/// Publishes four synthetic layers for generation `n` and returns their entries.
fn layers(store: &RevisionStore, n: u32) -> Vec<LayerEntry> {
    [
        ("root.usda", LayerRole::Authored),
        ("authored.usda", LayerRole::Authored),
        ("derived.usda", LayerRole::Derived),
        ("weak.usda", LayerRole::External),
    ]
    .into_iter()
    .map(|(name, role)| {
        let bytes = format!("#usda 1.0\n# synthetic {name} generation {n}\n");
        store.blobs().publish(bytes.as_bytes()).unwrap();
        LayerEntry::from_bytes(name, role, bytes.as_bytes()).unwrap()
    })
    .collect()
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

#[test]
fn a_pinned_head_refuses_commits_before_an_intent_and_leaves_another_draft_free() {
    use beamcraft_host::lease::{PublicationLease, StoreLease};
    use beamcraft_host::ledger::Outcome;
    let published_fixture = Fixture::new();
    let draft_fixture = Fixture::new();
    let published = published_fixture.store();
    let workspace = draft_fixture.store();
    let accepted = commit_n(&published, 1);
    commit_n(&workspace, 1);
    let candidate = draft(layers(&published, 2), 2);
    // A serving coordinator's maintenance lease must not prevent a queued rebase from pinning.
    let _serving = StoreLease::shared(published.blobs()).unwrap();
    let pin = PublicationLease::exclusive(published.blobs()).unwrap();
    let ledger = fs::read(published_fixture.0.join("ledger/outcomes.log")).unwrap();
    assert_eq!(published.open().unwrap().info(), &accepted);
    assert_eq!(
        published.commit(&candidate, &expect(1)).unwrap_err(),
        RevisionError::StoreBusy
    );
    assert_eq!(published.head().unwrap().unwrap(), accepted);
    assert_eq!(
        fs::read(published_fixture.0.join("ledger/outcomes.log")).unwrap(),
        ledger
    );
    assert_eq!(
        published.query_outcome(&candidate.op).unwrap(),
        Outcome::NotFound
    );
    assert!(!published_fixture.log(2).exists());
    assert_eq!(commit_n(&workspace, 2).sequence(), 2);
    drop(pin);
    assert_eq!(
        published.commit(&candidate, &expect(1)).unwrap().outcome,
        CommitOutcome::Created
    );
    assert_eq!(published.open().unwrap().info().sequence(), 2);
}

#[test]
fn ordinary_committers_share_the_gate_and_an_inflight_writer_refuses_an_exclusive_pin() {
    use beamcraft_host::lease::{LeaseError, PublicationLease, StoreLease};
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let first = PublicationLease::shared(store.blobs()).unwrap();
    let second = PublicationLease::shared(store.blobs()).unwrap();
    assert!(matches!(
        PublicationLease::exclusive(store.blobs()),
        Err(LeaseError::Busy)
    ));
    assert_eq!(
        commit_n(&store, 2).sequence(),
        2,
        "committers remain concurrent"
    );
    drop(first);
    assert!(matches!(
        PublicationLease::exclusive(store.blobs()),
        Err(LeaseError::Busy)
    ));
    drop(second);
    let pin = PublicationLease::exclusive(store.blobs()).unwrap();
    assert!(
        matches!(StoreLease::exclusive(store.blobs()), Err(LeaseError::Busy)),
        "the pinned history cannot be compacted"
    );
    assert!(matches!(
        PublicationLease::shared(store.blobs()),
        Err(LeaseError::Busy)
    ));
    drop(pin);
    assert!(StoreLease::exclusive(store.blobs()).is_ok());
}

#[test]
fn a_nonregular_publication_gate_refuses_every_committer_without_touching_the_root() {
    use beamcraft_host::lease::PublicationLease;
    let fx = Fixture::new();
    let store = fx.store();
    let accepted = commit_n(&store, 1);
    let candidate = draft(layers(&store, 2), 2);
    let path = fx.0.join("publication.lock");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let ledger = fs::read(fx.0.join("ledger/outcomes.log")).unwrap();
    assert!(PublicationLease::exclusive(store.blobs()).is_err());
    assert_eq!(
        store.commit(&candidate, &expect(1)).unwrap_err(),
        RevisionError::Storage(StorageError::UnsafeEntry)
    );
    assert_eq!(store.head().unwrap().unwrap(), accepted);
    assert_eq!(fs::read(fx.0.join("ledger/outcomes.log")).unwrap(), ledger);
    assert!(path.is_dir());
}

#[cfg(unix)]
#[test]
fn a_planted_publication_gate_link_never_locks_or_changes_its_target() {
    use beamcraft_host::lease::PublicationLease;
    use std::os::unix::fs::symlink;
    let fx = Fixture::new();
    let store = fx.store();
    let accepted = commit_n(&store, 1);
    let candidate = draft(layers(&store, 2), 2);
    let target = fx.0.join("sentinel");
    fs::write(&target, b"original bytes").unwrap();
    let path = fx.0.join("publication.lock");
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    assert!(PublicationLease::exclusive(store.blobs()).is_err());
    assert_eq!(
        store.commit(&candidate, &expect(1)).unwrap_err(),
        RevisionError::Storage(StorageError::UnsafeEntry)
    );
    assert_eq!(store.head().unwrap().unwrap(), accepted);
    assert_eq!(fs::read(target).unwrap(), b"original bytes");
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
}

/// Commits revision `n` on top of `n - 1`.
fn commit_n(store: &RevisionStore, n: u32) -> RevisionInfo {
    let d = draft(layers(store, n), n);
    store.commit(&d, &expect(n - 1)).unwrap().info
}

fn err<T: std::fmt::Debug>(r: Result<T, RevisionError>) -> RevisionError {
    r.unwrap_err()
}

#[test]
fn commit_and_open_round_trip_with_a_verified_chain() {
    let fx = Fixture::new();
    let store = fx.store();
    assert_eq!(store.head().unwrap(), None);
    assert_eq!(err(store.open()), RevisionError::NoRevision);
    assert_eq!(RevisionError::NoRevision.code(), ErrorCode::NotFound);

    let entries = layers(&store, 1);
    let first = store
        .commit(&draft(entries, 1), &ExpectedBase::Initial)
        .unwrap();
    assert_eq!(first.outcome, CommitOutcome::Created);
    assert_eq!(first.info.sequence(), 1);
    assert!(first.info.manifest().base().is_none());
    let second = commit_n(&store, 2);
    assert_eq!(second.sequence(), 2);
    let base = second.manifest().base().unwrap();
    assert_eq!(base.revision, id(101));
    assert_eq!(base.manifest, first.info.manifest_digest());

    // A fresh handle on the same directory, as another process would have, sees the same root.
    let fresh = fx.store().open().unwrap();
    assert_eq!(fresh.info(), &second);

    // An earlier revision is readable by its sequence; zero and a sequence past the head are not.
    assert_eq!(fx.store().info_at(1).unwrap(), first.info);
    assert_eq!(fx.store().info_at(2).unwrap(), second);
    assert_eq!(fx.store().open_at(1).unwrap().info(), &first.info);
    assert_eq!(err(fx.store().info_at(0)), RevisionError::NoRevision);
    assert_eq!(err(fx.store().info_at(3)), RevisionError::NoRevision);
    let manifest_bytes = fx.store().blobs().read(&second.manifest_digest()).unwrap();
    assert_eq!(manifest_bytes, second.manifest().encode());
    assert_eq!(
        Sha256::digest(&manifest_bytes).unwrap(),
        second.manifest_digest()
    );
    assert_eq!(
        Manifest::parse(&manifest_bytes).unwrap(),
        *second.manifest()
    );
    assert_eq!(
        fresh.layer_bytes("derived.usda").unwrap(),
        b"#usda 1.0\n# synthetic derived.usda generation 2\n"
    );
    assert_eq!(
        err(fresh.layer_bytes("missing.usda")),
        RevisionError::Storage(StorageError::NotFound)
    );
    assert_eq!(fresh.manifest().content().workspace_generation(), 42);
}

#[test]
fn twenty_reordered_saves_produce_byte_identical_manifests() {
    // One shared store: the layer blobs are identical, only the order of the draft differs.
    let fx = Fixture::new();
    let store = fx.store();
    let entries = layers(&store, 1);
    let mut reference: Option<(Vec<u8>, ContentDigest)> = None;
    for round in 0..20usize {
        let mut shuffled = entries.clone();
        shuffled.rotate_left(round % 4);
        if round % 3 == 0 {
            shuffled.reverse();
        }
        // Each round commits into its own fresh store directory with the same blob bytes.
        let each = Fixture::new();
        let each_store = each.store();
        for e in &shuffled {
            let bytes = fs::read(fx.blob(e.digest())).unwrap();
            each_store.blobs().publish(&bytes).unwrap();
        }
        let info = each_store
            .commit(&draft(shuffled, 1), &ExpectedBase::Initial)
            .unwrap()
            .info;
        let encoded = info.manifest().encode();
        match &reference {
            None => reference = Some((encoded, info.manifest_digest())),
            Some((bytes, d)) => {
                assert_eq!(&encoded, bytes, "round {round}");
                assert_eq!(&info.manifest_digest(), d);
            }
        }
        let on_disk = each_store.blobs().read(&info.manifest_digest()).unwrap();
        assert_eq!(on_disk, reference.as_ref().unwrap().0);
    }
}

#[test]
fn expected_base_is_verified_and_a_conflict_leaves_the_root_unchanged() {
    let fx = Fixture::new();
    let store = fx.store();
    let d1 = draft(layers(&store, 1), 1);
    // A committer that believes something was published when nothing was.
    assert_eq!(
        err(store.commit(&d1, &expect(1))),
        RevisionError::BaseMismatch
    );
    assert_eq!(store.head().unwrap(), None);
    let one = commit_n(&store, 1);

    let d2 = draft(layers(&store, 2), 2);
    // Stale: still thinks the project is empty; or names some other revision.
    assert_eq!(
        err(store.commit(&d2, &ExpectedBase::Initial)),
        RevisionError::BaseMismatch
    );
    assert_eq!(
        err(store.commit(&d2, &ExpectedBase::Revision(id(999)))),
        RevisionError::BaseMismatch
    );
    assert_eq!(
        RevisionError::BaseMismatch.code(),
        ErrorCode::BaseRevisionMismatch
    );
    assert_eq!(store.head().unwrap().unwrap(), one);
    // Another project in the same store.
    let mut foreign = draft(layers(&store, 2), 2);
    foreign.project = id(77);
    assert_eq!(
        err(store.commit(&foreign, &expect(1))),
        RevisionError::ProjectMismatch
    );
    assert_eq!(store.head().unwrap().unwrap(), one);
    // The right base then succeeds.
    assert_eq!(store.commit(&d2, &expect(1)).unwrap().info.sequence(), 2);
}

#[test]
fn missing_corrupt_resized_and_foreign_blobs_abort_before_the_switch() {
    let fx = Fixture::new();
    let store = fx.store();
    let one = commit_n(&store, 1);

    // Missing: the draft names a digest that was never published.
    let mut entries = layers(&store, 2);
    let ghost =
        LayerEntry::from_bytes("derived.usda", LayerRole::Derived, b"never stored").unwrap();
    let position = entries
        .iter()
        .position(|e| e.name() == "derived.usda")
        .unwrap();
    entries[position] = ghost;
    let missing = draft(entries, 2);
    // Entries are sorted by name: authored, derived, root, weak -> derived is index 1.
    assert_eq!(
        err(store.commit(&missing, &expect(1))),
        RevisionError::MissingBlob { entry: 1 }
    );

    // Resized claim: the blob exists and verifies but the manifest would lie about its size.
    let mut entries = layers(&store, 2);
    let real = entries[0].clone();
    entries[0] = LayerEntry::new(real.name(), real.role(), real.digest(), real.size() + 1).unwrap();
    assert_eq!(
        err(store.commit(&draft(entries, 2), &expect(1))),
        RevisionError::SizeMismatch { entry: 2 }
    );

    // Foreign/corrupt: a file under the blob's name whose bytes do not hash to it.
    let entries = layers(&store, 2);
    let victim = entries.iter().find(|e| e.name() == "weak.usda").unwrap();
    fs::write(
        fx.blob(victim.digest()),
        b"foreign bytes under a trusted name",
    )
    .unwrap();
    assert_eq!(
        err(store.commit(&draft(entries, 2), &expect(1))),
        RevisionError::CorruptBlob { entry: 3 }
    );
    assert_eq!(
        RevisionError::CorruptBlob { entry: 3 }.code(),
        ErrorCode::IntegrityError
    );

    // Every failure left the previous complete revision as the root and readable.
    assert_eq!(store.head().unwrap().unwrap(), one);
    assert_eq!(store.open().unwrap().info(), &one);
    assert!(!fx.log(2).exists());
    assert_eq!(fx.temp_leftovers(), 0);
}

#[test]
fn op_id_replay_is_idempotent_and_reuse_with_other_content_is_rejected() {
    let fx = Fixture::new();
    let store = fx.store();
    let d1 = draft(layers(&store, 1), 1);
    let first = store.commit(&d1, &ExpectedBase::Initial).unwrap();
    assert_eq!(first.outcome, CommitOutcome::Created);

    // A retry of the identical request, even with a different entry order and the original
    // (now stale) expected base, returns the recorded revision and writes nothing.
    let mut again = d1.clone();
    again.entries.reverse();
    let replay = store.commit(&again, &ExpectedBase::Initial).unwrap();
    assert_eq!(replay.outcome, CommitOutcome::Replayed);
    assert_eq!(replay.info, first.info);
    assert!(!fx.log(2).exists());

    // Same op id, different content (another generation, another layer, another root).
    let mut changed = d1.clone();
    changed.workspace_generation += 1;
    assert_eq!(
        err(store.commit(&changed, &ExpectedBase::Initial)),
        RevisionError::OpIdReuse
    );
    assert_eq!(RevisionError::OpIdReuse.code(), ErrorCode::OpIdReuse);
    let mut changed = d1.clone();
    changed.entries = layers(&store, 9);
    assert_eq!(
        err(store.commit(&changed, &expect(1))),
        RevisionError::OpIdReuse
    );

    // Same revision id under a new op id: an integrity error, not a second revision.
    let mut reused = draft(layers(&store, 2), 2);
    reused.revision = id(101);
    assert_eq!(
        err(store.commit(&reused, &expect(1))),
        RevisionError::RevisionIdReuse
    );
    assert_eq!(
        RevisionError::RevisionIdReuse.code(),
        ErrorCode::IntegrityError
    );
    assert_eq!(store.head().unwrap().unwrap(), first.info);

    // After the root moved on, the older op still resolves to its recorded revision.
    let two = commit_n(&store, 2);
    let old = store.commit(&d1, &ExpectedBase::Initial).unwrap();
    assert_eq!(old.outcome, CommitOutcome::Replayed);
    assert_eq!(old.info, first.info);
    assert_eq!(store.head().unwrap().unwrap(), two);
}

#[test]
fn old_ops_beyond_the_window_resolve_by_revision_id_or_fall_back_to_the_base_check() {
    let fx = Fixture::new();
    let store = fx.store();
    let total = u32::try_from(REPLAY_LOOKBACK).unwrap() + 2;
    let mut drafts = Vec::new();
    for n in 1..=total {
        let d = draft(layers(&store, n), n);
        store.commit(&d, &expect(n - 1)).unwrap();
        drafts.push(d);
    }
    // The newest REPLAY_LOOKBACK revisions (sequence 3..=66) are found by op id ...
    let inside = store.commit(&drafts[2], &expect(2)).unwrap();
    assert_eq!(inside.outcome, CommitOutcome::Replayed);
    assert_eq!(inside.info.sequence(), 3);
    // ... an older retry of the same request is still found, through its revision id.
    let old = store.commit(&drafts[1], &expect(1)).unwrap();
    assert_eq!(old.outcome, CommitOutcome::Replayed);
    assert_eq!(old.info.sequence(), 2);
    // An old op id with another revision id is, with the durable ledger (U07), recognised as
    // committed and rejected as reuse ...
    let mut unknown = draft(layers(&store, 2), 2);
    unknown.revision = id(900);
    assert_eq!(
        err(store.commit(&unknown, &expect(1))),
        RevisionError::OpIdReuse
    );
    // ... while a store without a ledger cannot recognise it: its stale base keeps it from
    // committing.
    fs::remove_dir_all(fx.0.join("ledger")).unwrap();
    assert_eq!(
        err(store.commit(&unknown, &expect(1))),
        RevisionError::BaseMismatch
    );
    assert_eq!(store.head().unwrap().unwrap().sequence(), u64::from(total));
}

#[test]
fn a_revision_id_cannot_be_reused_beyond_the_replay_window() {
    let fx = Fixture::new();
    let store = fx.store();
    let total = u32::try_from(REPLAY_LOOKBACK).unwrap() + 2;
    for n in 1..=total {
        commit_n(&store, n);
    }
    let head = store.head().unwrap().unwrap();
    assert_eq!(head.sequence(), u64::from(total));

    // A new operation on the current head tries to reuse revision 1's UUID (more than
    // REPLAY_LOOKBACK records back). It must be refused, not published at sequence 67.
    let mut reuse = draft(layers(&store, total + 1), total + 1);
    reuse.revision = id(101);
    let error = err(store.commit(&reuse, &expect(total)));
    assert_eq!(error, RevisionError::RevisionIdReuse);
    assert_eq!(error.code(), ErrorCode::IntegrityError);
    assert!(!fx.log(u64::from(total) + 1).exists());

    // Hence a stale writer that still expects revision 1 can never match the head by UUID.
    let stale = draft(layers(&store, total + 2), total + 2);
    assert_eq!(
        err(store.commit(&stale, &expect(1))),
        RevisionError::BaseMismatch
    );
    assert_eq!(store.head().unwrap().unwrap(), head);

    // An unreadable record means uniqueness cannot be established: fail closed.
    fs::write(fx.log(5), b"garbage").unwrap();
    let fresh = draft(layers(&store, total + 3), total + 3);
    assert_eq!(
        err(store.commit(&fresh, &expect(total))),
        RevisionError::TornRoot
    );
    assert!(!fx.log(u64::from(total) + 1).exists());
}

#[test]
fn a_retry_with_a_different_expected_base_is_not_a_replay() {
    let fx = Fixture::new();
    let store = fx.store();
    // Initial commit, retried with an arbitrary base: the canonical request differs.
    let d1 = draft(layers(&store, 1), 1);
    let one = store.commit(&d1, &ExpectedBase::Initial).unwrap();
    for wrong in [expect(1), ExpectedBase::Revision(id(999))] {
        assert_eq!(
            err(store.commit(&d1, &wrong)),
            RevisionError::OpIdReuse,
            "{wrong:?}"
        );
    }
    assert_eq!(store.head().unwrap().unwrap(), one.info);

    // A commit on a base, retried with the initial or another base: also a different request.
    let d2 = draft(layers(&store, 2), 2);
    let two = store.commit(&d2, &expect(1)).unwrap();
    for wrong in [ExpectedBase::Initial, ExpectedBase::Revision(id(999))] {
        assert_eq!(err(store.commit(&d2, &wrong)), RevisionError::OpIdReuse);
    }
    // The original base replays, also once the base is stale (the root has moved on).
    commit_n(&store, 3);
    let replay = store.commit(&d2, &expect(1)).unwrap();
    assert_eq!(replay.outcome, CommitOutcome::Replayed);
    assert_eq!(replay.info, two.info);
    assert_eq!(
        store.commit(&d1, &ExpectedBase::Initial).unwrap().outcome,
        CommitOutcome::Replayed
    );
}

#[test]
fn a_lost_race_replay_also_compares_the_expected_base() {
    // Two writers with the same op but different bases: exactly one commit, never two, and the
    // other gets OP_ID_REUSE (different canonical request) or a base conflict, never Replayed.
    for round in 0..20 {
        let fx = Fixture::new();
        let store = fx.store();
        commit_n(&store, 1);
        commit_n(&store, 2);
        let d = draft(layers(&store, 3), 3);
        let barrier = Barrier::new(2);
        let bases = [expect(2), ExpectedBase::Revision(id(999))];
        let results: Vec<_> = std::thread::scope(|scope| {
            bases
                .iter()
                .map(|base| {
                    let store = store.clone();
                    let (barrier, d) = (&barrier, &d);
                    scope.spawn(move || {
                        barrier.wait();
                        store.commit(d, base)
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect()
        });
        let created = results
            .iter()
            .filter(|r| matches!(r, Ok(c) if c.outcome == CommitOutcome::Created))
            .count();
        let replayed = results
            .iter()
            .filter(|r| matches!(r, Ok(c) if c.outcome == CommitOutcome::Replayed))
            .count();
        assert_eq!((created, replayed), (1, 0), "round {round}: {results:?}");
        assert!(matches!(
            results.iter().find_map(|r| r.as_ref().err()),
            Some(RevisionError::OpIdReuse | RevisionError::BaseMismatch)
        ));
        assert_eq!(store.head().unwrap().unwrap().sequence(), 3);
    }
}

#[test]
fn abort_at_every_point_before_the_switch_keeps_the_previous_revision_readable() {
    let all = [
        CommitPoint::Checked,
        CommitPoint::IntentRecorded,
        CommitPoint::BlobsVerified,
        CommitPoint::ManifestStored,
        CommitPoint::RecordStaged,
    ];
    for (i, target) in all.into_iter().enumerate() {
        for with_previous in [false, true] {
            let fx = Fixture::new();
            let store = fx.store();
            let n = if with_previous {
                commit_n(&store, 1);
                2
            } else {
                1
            };
            let before = store.head().unwrap();
            let d = draft(layers(&store, n), n);
            let mut seen = Vec::new();
            let result = store.commit_with(&d, &expect(n - 1), None, &mut |p: CommitPoint| {
                seen.push(p);
                if p == target { Err(Injected) } else { Ok(()) }
            });
            assert_eq!(err(result), RevisionError::Aborted(target), "{target:?}");
            assert_eq!(RevisionError::Aborted(target).code(), ErrorCode::Cancelled);
            assert_eq!(seen, all[..=i], "points are reached in order");
            // The root is exactly what it was, complete and readable by a fresh reader.
            assert_eq!(store.head().unwrap(), before);
            assert_eq!(fx.store().open().ok().map(|r| r.info().clone()), before);
            assert!(!fx.log(u64::from(n)).exists());
            assert_eq!(fx.temp_leftovers(), 0, "{target:?}");
            // The identical retry then commits the revision exactly once.
            let retry = store.commit(&d, &expect(n - 1)).unwrap();
            assert_eq!(retry.outcome, CommitOutcome::Created);
            assert_eq!(retry.info.sequence(), u64::from(n));
            assert_eq!(store.open().unwrap().info(), &retry.info);
        }
    }
}

#[test]
fn failure_after_the_switch_is_outcome_unknown_and_the_new_revision_is_complete() {
    let fx = Fixture::new();
    let store = fx.store();
    let one = commit_n(&store, 1);
    let d = draft(layers(&store, 2), 2);
    let mut seen = Vec::new();
    let result = store.commit_with(&d, &expect(1), None, &mut |p: CommitPoint| {
        seen.push(p);
        if p == CommitPoint::Switched {
            Err(Injected)
        } else {
            Ok(())
        }
    });
    assert_eq!(err(result), RevisionError::OutcomeUnknown);
    assert_eq!(
        RevisionError::OutcomeUnknown.code(),
        ErrorCode::OutcomeUnknown
    );
    assert_eq!(
        seen,
        [
            CommitPoint::Checked,
            CommitPoint::IntentRecorded,
            CommitPoint::BlobsVerified,
            CommitPoint::ManifestStored,
            CommitPoint::RecordStaged,
            CommitPoint::Switched
        ]
    );
    // Never reported as success or rolled back: the root is the new, complete revision ...
    let now = store.open().unwrap();
    assert_eq!(now.info().sequence(), 2);
    assert_ne!(now.info(), &one);
    assert_eq!(fx.temp_leftovers(), 0);
    // ... and the same request resolves the uncertainty without a second commit.
    let retry = store.commit(&d, &expect(1)).unwrap();
    assert_eq!(retry.outcome, CommitOutcome::Replayed);
    assert_eq!(retry.info, *now.info());
    assert!(!fx.log(3).exists());
}

type Seen = (String, LayerRole, ContentDigest, Vec<u8>);

struct Fake {
    result: Result<IndexEvidence, ContentRejection>,
    seen: RefCell<Vec<Seen>>,
    request: RefCell<Option<(String, ContentDigest)>>,
}
impl Fake {
    fn new(result: Result<IndexEvidence, ContentRejection>) -> Self {
        Self {
            result,
            seen: RefCell::default(),
            request: RefCell::default(),
        }
    }
}
impl ContentVerifier for Fake {
    fn verify(&self, request: &VerifyRequest<'_>) -> Result<IndexEvidence, ContentRejection> {
        *self.seen.borrow_mut() = request
            .layers
            .iter()
            .map(|l| (l.name.to_owned(), l.role, l.digest, l.bytes.to_vec()))
            .collect();
        *self.request.borrow_mut() = Some((request.root_layer.to_owned(), request.schema));
        self.result.clone()
    }
}
fn evidence(d: &RevisionDraft, pinned: ContentDigest) -> IndexEvidence {
    IndexEvidence {
        project: d.project.to_string(),
        workspace: d.workspace.to_string(),
        generation: d.workspace_generation,
        pinned_revision: pinned,
        root_layer: d.root_layer.clone(),
        schema: d.schema,
        complete: true,
        structure_digest: digest("synthetic index structure"),
        prim_count: 7,
    }
}
fn with_verifier(
    store: &RevisionStore,
    d: &RevisionDraft,
    base: &ExpectedBase,
    v: &Fake,
) -> Result<beamcraft_host::revision::Committed, RevisionError> {
    store.commit_with(d, base, Some(v), &mut beamcraft_host::revision::NoFault)
}

#[test]
fn the_content_verifier_gates_the_switch_and_its_evidence_is_recorded() {
    let fx = Fixture::new();
    let store = fx.store();
    let d1 = draft(layers(&store, 1), 1);
    let pinned_for_initial = digest("any pinned digest of an initial draft");

    // Verifier rejections pass their own stable code through, with the root unchanged.
    for code in [
        ErrorCode::DuplicateId,
        ErrorCode::InvalidId,
        ErrorCode::DanglingRelationship,
        ErrorCode::ForeignReadOnly,
    ] {
        let fake = Fake::new(Err(ContentRejection {
            code,
            message: "synthetic",
        }));
        let e = err(with_verifier(&store, &d1, &ExpectedBase::Initial, &fake));
        assert_eq!(e.code(), code);
        assert!(matches!(e, RevisionError::Content(_)));
        assert_eq!(store.head().unwrap(), None);
    }

    // Evidence that is partial or from another context is refused.
    let mut partial = evidence(&d1, pinned_for_initial);
    partial.complete = false;
    let fake = Fake::new(Ok(partial));
    let e = err(with_verifier(&store, &d1, &ExpectedBase::Initial, &fake));
    assert_eq!(e, RevisionError::IndexIncomplete);
    assert_eq!(e.code(), ErrorCode::DependencyUnavailable);
    for tamper in 0..3 {
        let mut other = evidence(&d1, pinned_for_initial);
        match tamper {
            0 => other.project = id(55).to_string(),
            1 => other.workspace = id(56).to_string(),
            _ => other.generation += 1,
        }
        let e = err(with_verifier(
            &store,
            &d1,
            &ExpectedBase::Initial,
            &Fake::new(Ok(other)),
        ));
        assert_eq!(e, RevisionError::IndexMismatch);
        assert_eq!(e.code(), ErrorCode::StaleWorkspace);
    }
    // A mismatch outranks incompleteness, as in the U03 guard order.
    let mut both = evidence(&d1, pinned_for_initial);
    both.complete = false;
    both.generation += 1;
    assert_eq!(
        err(with_verifier(
            &store,
            &d1,
            &ExpectedBase::Initial,
            &Fake::new(Ok(both))
        )),
        RevisionError::IndexMismatch
    );
    assert_eq!(store.head().unwrap(), None);

    // Accepted: the verifier saw every layer, sorted, with verified bytes; the manifest
    // records the index.
    let fake = Fake::new(Ok(evidence(&d1, pinned_for_initial)));
    let one = with_verifier(&store, &d1, &ExpectedBase::Initial, &fake)
        .unwrap()
        .info;
    let names: Vec<_> = fake.seen.borrow().iter().map(|l| l.0.clone()).collect();
    assert_eq!(
        names,
        ["authored.usda", "derived.usda", "root.usda", "weak.usda"]
    );
    for (_, _, d, bytes) in fake.seen.borrow().iter() {
        assert_eq!(Sha256::digest(bytes).unwrap(), *d);
    }
    let recorded = one.manifest().index().unwrap();
    assert_eq!(recorded.structure, digest("synthetic index structure"));
    assert_eq!(recorded.prims, 7);
    assert_eq!(store.open().unwrap().manifest().index(), Some(recorded));

    // The next revision must pin the base manifest digest; a stale pin is a foreign context.
    let d2 = draft(layers(&store, 2), 2);
    let stale = Fake::new(Ok(evidence(&d2, pinned_for_initial)));
    assert_eq!(
        err(with_verifier(&store, &d2, &expect(1), &stale)),
        RevisionError::IndexMismatch
    );
    let good = Fake::new(Ok(evidence(&d2, one.manifest_digest())));
    assert_eq!(
        with_verifier(&store, &d2, &expect(1), &good)
            .unwrap()
            .info
            .sequence(),
        2
    );

    // A commit without a verifier records no index and does not pretend to.
    let d3 = draft(layers(&store, 3), 3);
    let three = store.commit(&d3, &expect(2)).unwrap().info;
    assert_eq!(three.manifest().index(), None);
}

#[test]
fn the_verifier_is_bound_to_the_selected_root_and_schema() {
    let fx = Fixture::new();
    let store = fx.store();
    let entries = layers(&store, 1);
    let pinned = digest("pinned");
    let d_root = draft(entries.clone(), 1);

    // The same layers with another composition root: the verifier is told, and evidence for the
    // original root cannot be recorded for the new one.
    let mut d_other = d_root.clone();
    d_other.root_layer = "derived.usda".into();
    let fake = Fake::new(Ok(evidence(&d_root, pinned)));
    let error = err(with_verifier(
        &store,
        &d_other,
        &ExpectedBase::Initial,
        &fake,
    ));
    assert_eq!(error, RevisionError::IndexMismatch);
    assert_eq!(error.code(), ErrorCode::StaleWorkspace);
    let asked = fake.request.borrow().clone().unwrap();
    assert_eq!(asked, ("derived.usda".to_owned(), d_other.schema));
    assert_eq!(store.head().unwrap(), None);

    // A different schema identity is refused the same way.
    let mut wrong_schema = evidence(&d_root, pinned);
    wrong_schema.schema = digest("another schema");
    let fake = Fake::new(Ok(wrong_schema));
    assert_eq!(
        err(with_verifier(
            &store,
            &d_root,
            &ExpectedBase::Initial,
            &fake
        )),
        RevisionError::IndexMismatch
    );

    // Evidence that names the draft's root and schema is accepted for either root.
    let fake = Fake::new(Ok(evidence(&d_other, pinned)));
    let one = with_verifier(&store, &d_other, &ExpectedBase::Initial, &fake).unwrap();
    assert_eq!(one.info.manifest().content().root_layer(), "derived.usda");
    assert!(one.info.manifest().index().is_some());
}

#[test]
fn the_verifier_memory_cap_is_checked_before_any_read() {
    let fx = Fixture::new();
    let store = fx.store();
    let mut d = draft(layers(&store, 1), 1);
    // Three declared maximal blobs exceed the verifier cap; nothing needs to exist on disk.
    assert!(3 * MAX_BLOB_BYTES as u64 > MAX_CONTENT_VERIFY_BYTES);
    d.entries = (0..3)
        .map(|i| {
            let role = if i == 0 {
                LayerRole::Derived
            } else {
                LayerRole::Authored
            };
            LayerEntry::new(
                &format!("big{i}.usda"),
                role,
                digest("x"),
                MAX_BLOB_BYTES as u64,
            )
            .unwrap()
        })
        .collect();
    d.root_layer = "big1.usda".into();
    let fake = Fake::new(Ok(evidence(&d, digest("p"))));
    assert_eq!(
        err(with_verifier(&store, &d, &ExpectedBase::Initial, &fake)),
        RevisionError::LimitExceeded
    );
    assert!(fake.seen.borrow().is_empty());
}

#[test]
fn invalid_drafts_are_rejected_before_anything_is_written() {
    let fx = Fixture::new();
    let store = fx.store();
    let entries = layers(&store, 1);
    let mut no_root = draft(entries.clone(), 1);
    no_root.root_layer = "elsewhere.usda".into();
    assert_eq!(
        err(store.commit(&no_root, &ExpectedBase::Initial)),
        RevisionError::InvalidArgument
    );
    let mut dup = draft(entries.clone(), 1);
    dup.entries.push(entries[0].clone());
    assert_eq!(
        err(store.commit(&dup, &ExpectedBase::Initial)),
        RevisionError::DuplicateEntry
    );
    assert_eq!(RevisionError::DuplicateEntry.code(), ErrorCode::DuplicateId);
    assert!(!fx.0.join("revisions").exists());
    assert_eq!(store.head().unwrap(), None);
}

/// Writes a record file the way a (buggy or hostile) producer would.
fn forge_record(fx: &Fixture, sequence: u64, revision: &Uuid7, manifest: ContentDigest) {
    fs::create_dir_all(fx.0.join("revisions")).unwrap();
    let path = fx.log(sequence);
    let _ = fs::remove_file(&path);
    fs::write(
        path,
        format!(
            "beamcraft-revision-root 1\nsequence {sequence}\nrevision {revision}\nmanifest {manifest}\n"
        ),
    )
    .unwrap();
}

fn two(fx: &Fixture) -> (RevisionStore, RevisionInfo, RevisionInfo) {
    let store = fx.store();
    let one = commit_n(&store, 1);
    let two = commit_n(&store, 2);
    (store, one, two)
}

#[test]
fn the_reader_refuses_missing_garbage_collected_corrupt_and_resized_blobs() {
    let fx = Fixture::new();
    let (store, _, head) = two(&fx);
    let entries = head.manifest().content().entries().to_vec();

    // Corrupt in place: same name, same length, different bytes.
    let derived = entries
        .iter()
        .position(|e| e.name() == "derived.usda")
        .unwrap();
    let original = fs::read(fx.blob(entries[derived].digest())).unwrap();
    fs::write(
        fx.blob(entries[derived].digest()),
        vec![b'X'; original.len()],
    )
    .unwrap();
    assert_eq!(
        err(store.open()),
        RevisionError::CorruptBlob { entry: derived }
    );
    // Truncated.
    fs::write(fx.blob(entries[derived].digest()), &original[..4]).unwrap();
    assert_eq!(
        err(store.open()),
        RevisionError::CorruptBlob { entry: derived }
    );
    fs::write(fx.blob(entries[derived].digest()), &original).unwrap();
    assert!(store.open().is_ok());

    // Garbage-collected layer blob: the revision is partial, so nothing is returned.
    let weak = entries
        .iter()
        .position(|e| e.name() == "weak.usda")
        .unwrap();
    fs::remove_file(fx.blob(entries[weak].digest())).unwrap();
    assert_eq!(
        err(store.open()),
        RevisionError::MissingBlob { entry: weak }
    );
    assert_eq!(
        RevisionError::MissingBlob { entry: weak }.code(),
        ErrorCode::IntegrityError
    );
    // The cheap head view does not verify layer blobs; only open does.
    assert_eq!(store.head().unwrap().unwrap(), head);

    // Manifest blob garbage-collected, then corrupted.
    let manifest_blob = fx.blob(head.manifest_digest());
    let manifest_bytes = fs::read(&manifest_blob).unwrap();
    fs::remove_file(&manifest_blob).unwrap();
    assert_eq!(err(store.open()), RevisionError::ManifestMissing);
    assert_eq!(err(store.head()), RevisionError::ManifestMissing);
    fs::write(&manifest_blob, &manifest_bytes[..manifest_bytes.len() / 2]).unwrap();
    assert_eq!(err(store.open()), RevisionError::MalformedManifest);
    fs::write(&manifest_blob, &manifest_bytes).unwrap();
}

#[test]
fn a_manifest_that_lies_about_a_size_or_is_not_canonical_is_refused() {
    let fx = Fixture::new();
    let store = fx.store();
    let entries = layers(&store, 1);
    // A well-formed manifest whose declared size differs from the verified blob.
    let mut lying = entries.clone();
    lying[0] = LayerEntry::new(
        entries[0].name(),
        entries[0].role(),
        entries[0].digest(),
        entries[0].size() + 3,
    )
    .unwrap();
    let d = draft(lying, 1);
    let content = beamcraft_host::revision::Content::from_draft(&d).unwrap();
    let manifest = Manifest::assemble(content, 1, None, None).unwrap();
    let (digest, _) = store.blobs().publish(&manifest.encode()).unwrap();
    forge_record(&fx, 1, &d.revision, digest);
    // root.usda sorts third: authored, derived, root, weak.
    assert_eq!(err(store.open()), RevisionError::SizeMismatch { entry: 2 });
    assert_eq!(
        RevisionError::SizeMismatch { entry: 2 }.code(),
        ErrorCode::IntegrityError
    );

    // A valid-looking record pointing at a published blob that is not a manifest.
    let (junk, _) = store.blobs().publish(b"not a manifest\n").unwrap();
    forge_record(&fx, 1, &d.revision, junk);
    assert_eq!(err(store.open()), RevisionError::MalformedManifest);

    // A non-canonical manifest (an extra trailing line) published under its own digest.
    let mut loose = manifest.encode();
    loose.extend_from_slice(b"\n");
    let (loose_digest, _) = store.blobs().publish(&loose).unwrap();
    forge_record(&fx, 1, &d.revision, loose_digest);
    assert_eq!(err(store.open()), RevisionError::MalformedManifest);
}

#[test]
fn torn_or_inconsistent_head_logs_are_refused_with_typed_errors() {
    // Garbage and truncated record, oversized record.
    for bytes in [
        b"garbage".to_vec(),
        b"beamcraft-revision-root 1\nsequence 2\n".to_vec(),
        vec![b'x'; 600],
        Vec::new(),
    ] {
        let fx = Fixture::new();
        let (store, _, _) = two(&fx);
        fs::write(fx.log(2), &bytes).unwrap();
        assert_eq!(err(store.open()), RevisionError::TornRoot);
        assert_eq!(err(store.head()), RevisionError::TornRoot);
        assert_eq!(RevisionError::TornRoot.code(), ErrorCode::IntegrityError);
    }

    // A gap: record 1 is gone while record 2 exists.
    let fx = Fixture::new();
    let (store, _, _) = two(&fx);
    fs::remove_file(fx.log(1)).unwrap();
    assert_eq!(err(store.open()), RevisionError::TornRoot);
    // A commit on a torn log fails and does not "repair" or advance it.
    let d = draft(layers(&store, 3), 3);
    assert_eq!(err(store.commit(&d, &expect(2))), RevisionError::TornRoot);
    assert!(!fx.log(3).exists());

    // A skipped sequence number, an unknown file, a directory in place of a record.
    let fx = Fixture::new();
    let (store, _, head) = two(&fx);
    fs::copy(fx.log(2), fx.log(4)).unwrap();
    assert_eq!(err(store.open()), RevisionError::TornRoot);
    fs::remove_file(fx.log(4)).unwrap();
    assert_eq!(store.open().unwrap().info(), &head);
    fs::write(fx.0.join("revisions").join("notes.txt"), b"x").unwrap();
    assert_eq!(err(store.open()), RevisionError::TornRoot);
    fs::remove_file(fx.0.join("revisions").join("notes.txt")).unwrap();
    fs::remove_file(fx.log(2)).unwrap();
    fs::create_dir(fx.log(2)).unwrap();
    assert_eq!(err(store.open()), RevisionError::TornRoot);

    // Record, manifest and chain must agree.
    let fx = Fixture::new();
    let (store, one, head) = two(&fx);
    forge_record(&fx, 2, head.revision(), one.manifest_digest());
    assert_eq!(err(store.open()), RevisionError::RootMismatch);
    forge_record(&fx, 2, &id(555), head.manifest_digest());
    assert_eq!(err(store.open()), RevisionError::RootMismatch);
    forge_record(&fx, 2, head.revision(), head.manifest_digest());
    assert!(store.open().is_ok());
    // The base link: record 1 is replaced by another revision's identity.
    forge_record(&fx, 1, &id(556), one.manifest_digest());
    assert_eq!(err(store.open()), RevisionError::RootMismatch);
    assert_eq!(
        RevisionError::RootMismatch.code(),
        ErrorCode::IntegrityError
    );

    // Interrupted-commit leftovers (a temporary record) are ignored by readers.
    let fx = Fixture::new();
    let (store, _, head) = two(&fx);
    fs::write(
        fx.0.join("revisions")
            .join(".tmp-00000000-0000000000000000"),
        b"half",
    )
    .unwrap();
    assert_eq!(store.open().unwrap().info(), &head);
    let again = store
        .commit(&draft(layers(&store, 3), 3), &expect(2))
        .unwrap();
    assert_eq!(again.info.sequence(), 3);
}

#[test]
fn two_writers_on_one_base_never_both_win() {
    for round in 0..100 {
        let fx = Fixture::new();
        let store = fx.store();
        commit_n(&store, 1);
        let drafts = [draft(layers(&store, 2), 2), draft(layers(&store, 3), 3)];
        let barrier = Barrier::new(2);
        let results: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = drafts
                .iter()
                .map(|d| {
                    let store = store.clone();
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        store.commit(d, &expect(1))
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let created = results
            .iter()
            .filter(|r| matches!(r, Ok(c) if c.outcome == CommitOutcome::Created))
            .count();
        let conflicts = results
            .iter()
            .filter(|r| matches!(r, Err(RevisionError::BaseMismatch)))
            .count();
        assert_eq!((created, conflicts), (1, 1), "round {round}: {results:?}");
        let head = store.open().unwrap();
        assert_eq!(head.info().sequence(), 2);
        assert!(fx.log(2).exists() && !fx.log(3).exists());
        assert_eq!(fx.temp_leftovers(), 0);
    }
}

#[test]
fn two_writers_with_the_same_op_commit_once_and_the_other_replays() {
    for round in 0..20 {
        let fx = Fixture::new();
        let store = fx.store();
        commit_n(&store, 1);
        let d = draft(layers(&store, 2), 2);
        let barrier = Barrier::new(2);
        let results: Vec<_> = std::thread::scope(|scope| {
            (0..2)
                .map(|_| {
                    let store = store.clone();
                    let (barrier, d) = (&barrier, &d);
                    scope.spawn(move || {
                        barrier.wait();
                        store.commit(d, &expect(1)).unwrap().outcome
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect()
        });
        let created = results
            .iter()
            .filter(|o| **o == CommitOutcome::Created)
            .count();
        assert_eq!(created, 1, "round {round}: {results:?}");
        assert_eq!(store.open().unwrap().info().sequence(), 2);
        assert!(!fx.log(3).exists());
    }
}
