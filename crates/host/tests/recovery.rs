// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U07 fault-injection tests against the real filesystem and the real commit path; no storage
//! mocks. A crash is simulated by a [`CommitHook`] that unwinds at a chosen [`CommitPoint`]: no
//! error handling of the commit runs afterwards, so the ledger and the root keep exactly what
//! was synced up to that point. All layer bytes and identities are original synthetic values.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use beamcraft_host::ledger::{
    AbortReason, CommittedRecord, Intent, LEDGER_HEADROOM_BYTES, LedgerError, MAX_LEDGER_BYTES,
    MAX_LEDGER_RECORDS, MAX_RECORD_BYTES, MAX_UNRESOLVED_INTENTS, Outcome, OutcomeSource, Record,
    RecoveryPolicy, RecoveryReport, Scan,
};
use beamcraft_host::revision::{
    BaseRef, CommitOutcome, CommitPoint, Content, ExpectedBase, Injected, LayerEntry, LayerRole,
    Manifest, REPLAY_LOOKBACK, RevisionDraft, RevisionError, RevisionInfo, RevisionStore, Uuid7,
};
use beamcraft_host::storage::{BlobStore, StorageError};
use beamcraft_model::{
    ErrorCode,
    digest::{ContentDigest, Sha256},
};

fn base_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("u07-tests")
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self::at_time(time)
    }
    fn at_time(time: u128) -> Self {
        let base = base_dir();
        fs::create_dir_all(&base).unwrap();
        // Nextest runs tests in separate processes; a timestamp alone can collide.
        // Reserve atomically and skip existing directories, including leftovers from a crash.
        for attempt in 0..1024 {
            let root = base.join(format!("test-{time}-{}-{attempt}", std::process::id()));
            match fs::create_dir(&root) {
                Ok(()) => return Self(root),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create recovery fixture: {error}"),
            }
        }
        panic!("recovery fixture directory attempts exhausted")
    }
    fn store(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(&self.0).unwrap())
    }
    fn log(&self, sequence: u64) -> PathBuf {
        self.0.join("revisions").join(format!("{sequence:020}"))
    }
    fn ledger_dir(&self) -> PathBuf {
        self.0.join("ledger")
    }
    fn ledger_file(&self) -> PathBuf {
        self.ledger_dir().join("outcomes.log")
    }
    fn ledger_bytes(&self) -> Vec<u8> {
        fs::read(self.ledger_file()).unwrap_or_default()
    }
    fn set_ledger(&self, bytes: &[u8]) {
        fs::create_dir_all(self.ledger_dir()).unwrap();
        fs::write(self.ledger_file(), bytes).unwrap();
    }
    fn scan(&self) -> Scan {
        Scan::parse(&self.ledger_bytes()).unwrap()
    }
    fn blob(&self, digest: ContentDigest) -> PathBuf {
        let name = digest.to_string();
        self.0.join("blobs").join(&name[..2]).join(name)
    }
    /// Plants the abandoned temporary files a real crash leaves behind.
    fn plant_temporaries(&self) {
        fs::create_dir_all(self.0.join("revisions")).unwrap();
        fs::create_dir_all(self.0.join("blobs")).unwrap();
        fs::write(self.0.join("revisions").join(".tmp-deadbeef-0000"), b"half").unwrap();
        fs::write(
            self.0.join("blobs").join(".publish-0000000000000000"),
            b"half",
        )
        .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let base = base_dir();
        assert!(self.0.is_absolute() && self.0.starts_with(&base) && self.0 != base);
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn fixtures_with_the_same_timestamp_do_not_reuse_existing_directories() {
    let first = Fixture::at_time(0);
    fs::write(first.0.join("sentinel"), b"keep").unwrap();
    let second = Fixture::at_time(0);
    assert_ne!(first.0, second.0);
    assert_eq!(fs::read(first.0.join("sentinel")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&second.0).unwrap().count(), 0);
}

fn id(n: u32) -> Uuid7 {
    Uuid7::parse(&format!("0199e7a0-0000-7000-8000-{n:012x}")).unwrap()
}
fn digest(text: &str) -> ContentDigest {
    Sha256::digest(text.as_bytes()).unwrap()
}

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

/// Revision `n`: revision id `100 + n`, op id `200 + n`.
fn draft(store: &RevisionStore, n: u32) -> RevisionDraft {
    RevisionDraft {
        project: id(1),
        workspace: id(2),
        revision: id(100 + n),
        op: id(200 + n),
        workspace_generation: 40 + u64::from(n),
        schema: digest("synthetic schema identity"),
        root_layer: "root.usda".into(),
        entries: layers(store, n),
    }
}

fn expect(n: u32) -> ExpectedBase {
    if n == 0 {
        ExpectedBase::Initial
    } else {
        ExpectedBase::Revision(id(100 + n))
    }
}

fn commit_n(store: &RevisionStore, n: u32) -> RevisionInfo {
    store.commit(&draft(store, n), &expect(n - 1)).unwrap().info
}

struct Crash;

/// Runs a commit that "dies" when it reaches `at`: the hook unwinds, so nothing after that
/// point (error handling, ledger abort records, `Switched` bookkeeping) runs.
fn crash_at(store: &RevisionStore, d: &RevisionDraft, expected: &ExpectedBase, at: CommitPoint) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        store.commit_with(
            d,
            expected,
            None,
            &mut |p: CommitPoint| -> Result<(), Injected> {
                if p == at {
                    resume_unwind(Box::new(Crash));
                }
                Ok(())
            },
        )
    }));
    assert!(
        result.is_err(),
        "the commit must not return after a crash at {at:?}"
    );
}

fn quiescent(store: &RevisionStore) -> RecoveryReport {
    store.recover(RecoveryPolicy::quiescent()).unwrap()
}

/// Where a crash at each point leaves the ledger and the root.
struct Shape {
    point: CommitPoint,
    intent: bool,
    switched: bool,
    committed: bool,
}

const SHAPES: [Shape; 7] = [
    Shape {
        point: CommitPoint::Checked,
        intent: false,
        switched: false,
        committed: false,
    },
    Shape {
        point: CommitPoint::IntentRecorded,
        intent: true,
        switched: false,
        committed: false,
    },
    Shape {
        point: CommitPoint::BlobsVerified,
        intent: true,
        switched: false,
        committed: false,
    },
    Shape {
        point: CommitPoint::ManifestStored,
        intent: true,
        switched: false,
        committed: false,
    },
    Shape {
        point: CommitPoint::RecordStaged,
        intent: true,
        switched: false,
        committed: false,
    },
    Shape {
        point: CommitPoint::Switched,
        intent: true,
        switched: true,
        committed: false,
    },
    Shape {
        point: CommitPoint::Recorded,
        intent: true,
        switched: true,
        committed: true,
    },
];

/// The root is a complete revision (verified manifest and every blob) or nothing at all.
fn assert_complete_root(store: &RevisionStore, expected_head: u64) {
    if expected_head == 0 {
        assert_eq!(store.head().unwrap(), None);
        assert_eq!(store.open().unwrap_err(), RevisionError::NoRevision);
        return;
    }
    let revision = store.open().unwrap();
    assert_eq!(revision.info().sequence(), expected_head);
    for layer in revision.manifest().content().entries() {
        assert!(!revision.layer_bytes(layer.name()).unwrap().is_empty());
    }
}

/// Every revision of the root is answered `Committed` with its own revision, sequence and
/// manifest, and nothing beyond the root is.
fn assert_ledger_agrees_with_root(store: &RevisionStore, head: u64) {
    for n in 1..=head {
        match store
            .query_outcome(&id(200 + u32::try_from(n).unwrap()))
            .unwrap()
        {
            Outcome::Committed {
                revision, sequence, ..
            } => {
                assert_eq!(revision, id(100 + u32::try_from(n).unwrap()));
                assert_eq!(sequence, n);
            }
            other => panic!("revision {n} of the root answered {other:?}"),
        }
    }
    let beyond = id(200 + u32::try_from(head).unwrap() + 1);
    assert!(!matches!(
        store.query_outcome(&beyond).unwrap(),
        Outcome::Committed { .. }
    ));
}

fn is_committed(outcome: &Outcome, source: OutcomeSource) -> bool {
    matches!(outcome, Outcome::Committed { source: s, .. } if *s == source)
}

// ---- the commit writes intent -> root -> committed ------------------------------------------

#[test]
fn a_commit_records_intent_then_committed_and_the_ledger_agrees_with_the_root() {
    let fx = Fixture::new();
    let store = fx.store();
    assert_eq!(store.query_outcome(&id(201)).unwrap(), Outcome::NotFound);
    assert!(!fx.ledger_file().exists());

    let one = commit_n(&store, 1);
    commit_n(&store, 2);
    let scan = fx.scan();
    assert_eq!(scan.genesis_head(), Some(0));
    assert_eq!(
        scan.diagnostics().records,
        5,
        "genesis + 2 x (intent, committed)"
    );
    assert_eq!(scan.diagnostics().damaged_lines, 0);
    assert!(!scan.diagnostics().torn_tail);
    assert_eq!(scan.open_intents().count(), 0);
    match store.query_outcome(&id(201)).unwrap() {
        Outcome::Committed {
            revision,
            sequence,
            manifest,
            source,
        } => {
            assert_eq!(revision, id(101));
            assert_eq!(sequence, 1);
            assert_eq!(manifest, one.manifest_digest());
            assert_eq!(source, OutcomeSource::Ledger);
        }
        other => panic!("{other:?}"),
    }
    assert_ledger_agrees_with_root(&store, 2);
    assert_eq!(store.query_outcome(&id(299)).unwrap(), Outcome::NotFound);
    // Replay and read-only queries append nothing.
    let before = fx.ledger_bytes();
    let d1 = draft(&store, 1);
    assert_eq!(
        store.commit(&d1, &expect(0)).unwrap().outcome,
        CommitOutcome::Replayed
    );
    store.query_outcome(&id(201)).unwrap();
    assert_eq!(fx.ledger_bytes(), before);
}

#[test]
fn the_intent_is_on_disk_before_the_root_changes() {
    let fx = Fixture::new();
    let store = fx.store();
    let d = draft(&store, 1);
    let mut checked = Vec::new();
    store
        .commit_with(
            &d,
            &expect(0),
            None,
            &mut |p: CommitPoint| -> Result<(), Injected> {
                let intents = fx.scan().open_intents().count();
                let switched = fx.log(1).exists();
                checked.push((p, intents, switched));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        checked,
        [
            (CommitPoint::Checked, 0, false),
            (CommitPoint::IntentRecorded, 1, false),
            (CommitPoint::BlobsVerified, 1, false),
            (CommitPoint::ManifestStored, 1, false),
            (CommitPoint::RecordStaged, 1, false),
            (CommitPoint::Switched, 1, true),
            (CommitPoint::Recorded, 0, true),
        ]
    );
}

// ---- fault injection at every commit point ---------------------------------------------------

#[test]
fn a_crash_at_every_point_recovers_to_the_old_or_the_new_complete_snapshot() {
    for shape in SHAPES {
        for with_previous in [false, true] {
            let fx = Fixture::new();
            let store = fx.store();
            let n = if with_previous {
                commit_n(&store, 1);
                2
            } else {
                1
            };
            let d = draft(&store, n);
            crash_at(&store, &d, &expect(n - 1), shape.point);
            fx.plant_temporaries();
            let label = format!("{:?} previous={with_previous}", shape.point);
            let (old_head, new_head) = (u64::from(n) - 1, u64::from(n));
            let head = if shape.switched { new_head } else { old_head };

            // A fresh reader sees one complete snapshot, before any recovery runs.
            assert_complete_root(&fx.store(), head);
            assert_eq!(fx.log(new_head).exists(), shape.switched, "{label}");
            assert_eq!(
                fx.scan().open_intents().count() == 1,
                shape.intent && !shape.committed
            );

            // QueryOutcome never claims more than can be proven.
            let op = id(200 + n);
            let before = store.query_outcome(&op).unwrap();
            match (shape.intent, shape.switched, shape.committed) {
                (false, ..) => assert_eq!(before, Outcome::NotFound, "{label}"),
                (true, false, _) => assert_eq!(before, Outcome::Unknown, "{label}"),
                (true, true, false) => {
                    assert!(is_committed(&before, OutcomeSource::Root), "{label}")
                }
                (true, true, true) => {
                    assert!(is_committed(&before, OutcomeSource::Ledger), "{label}")
                }
            }

            let report = quiescent(&store);
            assert_eq!(report.removed_temp_files, 2, "{label}");
            assert_eq!(report.root_error, None, "{label}");
            assert!(report.unprovable.is_empty() && report.disagreements.is_empty());
            let expect_committed = shape.switched && !shape.committed;
            let expect_aborted = shape.intent && !shape.switched;
            assert_eq!(
                report.committed,
                if expect_committed {
                    vec![op.clone()]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                report.aborted,
                if expect_aborted {
                    vec![op.clone()]
                } else {
                    vec![]
                }
            );

            // Old or new, still complete; the ledger now agrees with the root.
            assert_complete_root(&fx.store(), head);
            assert_ledger_agrees_with_root(&store, head);
            let after = store.query_outcome(&op).unwrap();
            match (shape.intent, shape.switched) {
                (false, _) => assert_eq!(after, Outcome::NotFound, "{label}"),
                (true, false) => assert_eq!(
                    after,
                    Outcome::Aborted {
                        reason: AbortReason::Recovered
                    },
                    "{label}"
                ),
                (true, true) => assert!(is_committed(&after, OutcomeSource::Ledger), "{label}"),
            }
            assert_eq!(fx.scan().open_intents().count(), 0, "{label}");

            // Idempotent: a second and third recovery change nothing and report nothing.
            let (ledger, root) = (fx.ledger_bytes(), store.head().unwrap());
            for _ in 0..2 {
                let again = quiescent(&store);
                assert!(again.is_noop(), "{label}: {again:?}");
                assert!(again.committed.is_empty() && again.aborted.is_empty());
                assert_eq!(fx.ledger_bytes(), ledger, "{label}");
                assert_eq!(store.head().unwrap(), root, "{label}");
            }

            // The retry of the same operation: replayed to the recorded revision when the root
            // switched, committed exactly once when it did not.
            let retry = fx.store().commit(&d, &expect(n - 1)).unwrap();
            assert_eq!(
                retry.outcome,
                if shape.switched {
                    CommitOutcome::Replayed
                } else {
                    CommitOutcome::Created
                },
                "{label}"
            );
            assert_eq!(retry.info.sequence(), new_head, "{label}");
            assert_eq!(retry.info.revision(), &id(100 + n));
            assert_complete_root(&fx.store(), new_head);
            assert_ledger_agrees_with_root(&store, new_head);
            assert!(is_committed(
                &store.query_outcome(&op).unwrap(),
                OutcomeSource::Ledger
            ));
            assert!(quiescent(&store).is_noop(), "{label}");
            assert!(!fx.log(new_head + 1).exists());
        }
    }
}

#[test]
fn different_content_under_the_same_op_id_is_op_id_reuse_before_and_after_recovery() {
    // After the root switched (committed or not yet recorded) the op id is bound to its content.
    for point in [CommitPoint::Switched, CommitPoint::Recorded] {
        let fx = Fixture::new();
        let store = fx.store();
        let d = draft(&store, 1);
        crash_at(&store, &d, &expect(0), point);
        let mut changed = d.clone();
        changed.workspace_generation += 1;
        let reuse = RevisionError::OpIdReuse;
        assert_eq!(
            store.commit(&changed, &expect(0)).unwrap_err(),
            reuse,
            "{point:?}"
        );
        quiescent(&store);
        assert_eq!(
            store.commit(&changed, &expect(0)).unwrap_err(),
            reuse,
            "{point:?}"
        );
        assert_eq!(reuse.code(), ErrorCode::OpIdReuse);
        assert_eq!(store.head().unwrap().unwrap().sequence(), 1);
    }

    // An intent that recovery has not resolved may already be in the root: the id stays bound.
    let fx = Fixture::new();
    let store = fx.store();
    let d = draft(&store, 1);
    crash_at(&store, &d, &expect(0), CommitPoint::RecordStaged);
    let mut changed = d.clone();
    changed.root_layer = "authored.usda".into();
    assert_eq!(
        store.commit(&changed, &expect(0)).unwrap_err(),
        RevisionError::OpIdReuse
    );
    // Once recovery proved the attempt never reached the root, the id is free again.
    quiescent(&store);
    assert_eq!(
        store.commit(&changed, &expect(0)).unwrap().outcome,
        CommitOutcome::Created
    );
}

#[test]
fn an_op_id_is_bound_to_its_expected_base_too() {
    // Unresolved attempt: the same content under a different expected base is another request.
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let d = draft(&store, 2);
    crash_at(&store, &d, &expect(1), CommitPoint::RecordStaged);
    assert_eq!(
        store.commit(&d, &ExpectedBase::Initial).unwrap_err(),
        RevisionError::OpIdReuse
    );
    quiescent(&store);
    // Once the attempt is closed as not accepted the id is free again.
    assert_eq!(
        store.commit(&d, &expect(1)).unwrap().outcome,
        CommitOutcome::Created
    );

    // Committed attempt, also after recovery: the original (now stale) base replays, another
    // base is OP_ID_REUSE.
    let fx = Fixture::new();
    let store = fx.store();
    let d = draft(&store, 1);
    crash_at(&store, &d, &expect(0), CommitPoint::Switched);
    quiescent(&store);
    commit_n(&store, 2);
    assert_eq!(
        store.commit(&d, &expect(0)).unwrap().outcome,
        CommitOutcome::Replayed
    );
    assert_eq!(
        store.commit(&d, &expect(2)).unwrap_err(),
        RevisionError::OpIdReuse
    );
    assert_eq!(store.head().unwrap().unwrap().sequence(), 2);
}

#[test]
fn graceful_cancellation_is_recorded_and_a_failed_switch_hook_is_resolved_by_a_retry() {
    let fx = Fixture::new();
    let store = fx.store();
    let d = draft(&store, 1);
    let result = store.commit_with(&d, &expect(0), None, &mut |p: CommitPoint| {
        if p == CommitPoint::ManifestStored {
            Err(Injected)
        } else {
            Ok(())
        }
    });
    assert_eq!(
        result.unwrap_err(),
        RevisionError::Aborted(CommitPoint::ManifestStored)
    );
    assert_eq!(
        store.query_outcome(&id(201)).unwrap(),
        Outcome::Aborted {
            reason: AbortReason::Cancelled
        }
    );
    assert_eq!(fx.scan().open_intents().count(), 0);
    assert!(quiescent(&store).is_noop());

    // A later failure in the commit is "rejected", with the root untouched.
    let mut missing = draft(&store, 1);
    missing.entries[0] =
        LayerEntry::from_bytes("root.usda", LayerRole::Authored, b"never stored").unwrap();
    assert!(matches!(
        store.commit(&missing, &expect(0)).unwrap_err(),
        RevisionError::MissingBlob { .. }
    ));
    assert_eq!(
        store.query_outcome(&id(201)).unwrap(),
        Outcome::Aborted {
            reason: AbortReason::Rejected
        }
    );

    // A hook failure after the switch: OUTCOME_UNKNOWN to the caller, but the root proves it.
    let result = store.commit_with(&d, &expect(0), None, &mut |p: CommitPoint| {
        if p == CommitPoint::Switched {
            Err(Injected)
        } else {
            Ok(())
        }
    });
    assert_eq!(result.unwrap_err(), RevisionError::OutcomeUnknown);
    assert!(is_committed(
        &store.query_outcome(&id(201)).unwrap(),
        OutcomeSource::Root
    ));
    // The identical retry resolves it without a second revision and records it in the ledger.
    let retry = store.commit(&d, &expect(0)).unwrap();
    assert_eq!(retry.outcome, CommitOutcome::Replayed);
    assert!(is_committed(
        &store.query_outcome(&id(201)).unwrap(),
        OutcomeSource::Ledger
    ));
    assert_eq!(fx.scan().open_intents().count(), 0);
    assert!(!fx.log(2).exists());
}

// ---- torn ledger tails at every byte ---------------------------------------------------------

fn last_record_start(bytes: &[u8]) -> usize {
    bytes[..bytes.len() - 1]
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |at| at + 1)
}

#[test]
fn a_torn_committed_record_at_every_byte_still_recovers_to_the_proven_commit() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    commit_n(&store, 2);
    let full = fx.ledger_bytes();
    let start = last_record_start(&full);
    let op2 = id(202);
    let root = store.head().unwrap();
    for cut in 0..full.len() - start {
        fx.set_ledger(&full[..start + cut]);
        let torn = fx.scan();
        assert_eq!(torn.diagnostics().torn_tail, cut > 0, "cut {cut}");
        assert_eq!(torn.open_intents().count(), 1, "cut {cut}");

        // Never trusted: the root proves acceptance, the torn record is not consulted.
        assert!(is_committed(
            &store.query_outcome(&op2).unwrap(),
            OutcomeSource::Root
        ));
        let report = quiescent(&store);
        assert_eq!(report.committed, vec![op2.clone()], "cut {cut}");
        assert_eq!(report.ledger.torn_tail, cut > 0);
        assert!(is_committed(
            &store.query_outcome(&op2).unwrap(),
            OutcomeSource::Ledger
        ));

        let healed = fx.scan();
        // An unterminated fragment that is the whole record minus its newline becomes a valid
        // record once terminated; any shorter fragment becomes one damaged, ignored line.
        let damaged = usize::from(cut > 0 && cut + 1 < full.len() - start);
        assert_eq!(healed.diagnostics().damaged_lines, damaged, "cut {cut}");
        assert!(!healed.diagnostics().torn_tail && healed.open_intents().count() == 0);
        let bytes = fx.ledger_bytes();
        assert!(quiescent(&store).is_noop(), "cut {cut}");
        assert_eq!(fx.ledger_bytes(), bytes, "cut {cut}");
        assert_eq!(store.head().unwrap(), root);
        assert_complete_root(&store, 2);
    }
    fx.set_ledger(&full);
    assert!(quiescent(&store).is_noop());
}

#[test]
fn a_torn_intent_at_every_byte_proves_nothing_was_accepted_and_a_retry_commits() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let d2 = draft(&store, 2);
    crash_at(&store, &d2, &expect(1), CommitPoint::IntentRecorded);
    let full = fx.ledger_bytes();
    let start = last_record_start(&full);
    let op2 = id(202);
    let len = full.len() - start;
    for cut in 0..=len {
        fx.set_ledger(&full[..start + cut]);
        let answer = store.query_outcome(&op2).unwrap();
        if cut == len {
            // The whole intent is there and the root has not switched: unresolved, not "no".
            assert_eq!(answer, Outcome::Unknown);
        } else {
            // No complete intent can exist without being synced first, so no switch happened.
            assert_eq!(answer, Outcome::NotFound, "cut {cut}");
        }
        let report = quiescent(&store);
        assert_eq!(report.aborted.len(), usize::from(cut == len), "cut {cut}");
        assert!(report.committed.is_empty());
        assert_complete_root(&store, 1);
        assert!(!fx.log(2).exists());
        let settled = fx.ledger_bytes();
        assert!(quiescent(&store).is_noop());
        assert_eq!(fx.ledger_bytes(), settled);
    }
    // The retry after a torn intent appends on a fresh line and commits once.
    for cut in [0, 1, len / 2, len - 1] {
        let fx = Fixture::new();
        let store = fx.store();
        commit_n(&store, 1);
        let d2 = draft(&store, 2);
        crash_at(&store, &d2, &expect(1), CommitPoint::IntentRecorded);
        let bytes = fx.ledger_bytes();
        let start = last_record_start(&bytes);
        fx.set_ledger(&bytes[..start + cut]);
        let retry = store.commit(&d2, &expect(1)).unwrap();
        assert_eq!(retry.outcome, CommitOutcome::Created, "cut {cut}");
        assert_eq!(
            fx.scan().diagnostics().damaged_lines,
            usize::from(cut > 0 && cut + 1 < len)
        );
        assert!(is_committed(
            &store.query_outcome(&op2).unwrap(),
            OutcomeSource::Ledger
        ));
        assert_ledger_agrees_with_root(&store, 2);
        assert!(quiescent(&store).is_noop());
    }
}

#[test]
fn flipped_garbage_and_oversized_ledger_content_is_counted_never_trusted() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    commit_n(&store, 2);
    let good = fx.ledger_bytes();

    // Flip one byte of every record in turn: that record is damaged, nothing else changes, and
    // no answer ever says something the root does not hold.
    let mut starts = vec![0usize];
    starts.extend(
        good.iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .map(|(i, _)| i + 1),
    );
    starts.pop();
    for start in starts {
        let mut flipped = good.clone();
        flipped[start + 10] ^= 0x01;
        fx.set_ledger(&flipped);
        let scan = fx.scan();
        assert_eq!(scan.diagnostics().damaged_lines, 1, "record at {start}");
        for n in 1..=2u32 {
            let answer = store.query_outcome(&id(200 + n)).unwrap();
            assert!(
                matches!(answer, Outcome::Committed { .. } | Outcome::Unknown),
                "record at {start}: {answer:?}"
            );
        }
        quiescent(&store);
        assert_ledger_agrees_with_root(&store, 2);
        assert_complete_root(&store, 2);
    }

    // Garbage and an over-long line of hostile bytes.
    let mut hostile = good.clone();
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..20_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        hostile.push(u8::try_from(state & 0xff).unwrap());
    }
    hostile.extend(vec![b'x'; MAX_RECORD_BYTES * 3]);
    hostile.push(b'\n');
    fx.set_ledger(&hostile);
    assert!(fx.scan().diagnostics().damaged_lines >= 1);
    assert!(is_committed(
        &store.query_outcome(&id(201)).unwrap(),
        OutcomeSource::Ledger
    ));
    quiescent(&store);
    // A new commit still works and lands on its own line after the garbage.
    commit_n(&store, 3);
    assert_ledger_agrees_with_root(&store, 3);
    assert!(is_committed(
        &store.query_outcome(&id(203)).unwrap(),
        OutcomeSource::Ledger
    ));
}

// ---- the root is the authority ---------------------------------------------------------------

fn forged_commit(op: u32, revision: u32, sequence: u64) -> Vec<u8> {
    Record::Committed(CommittedRecord {
        op: id(op),
        revision: id(revision),
        sequence,
        manifest: digest("forged manifest"),
    })
    .encode()
    .0
}

#[test]
fn a_ledger_commit_the_root_does_not_hold_is_refused_not_trusted() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let mut bytes = fx.ledger_bytes();
    bytes.extend(forged_commit(250, 350, 2)); // beyond the root
    bytes.extend(forged_commit(251, 101, 1)); // wrong manifest for a real revision
    fx.set_ledger(&bytes);
    for op in [250, 251] {
        assert_eq!(
            store.query_outcome(&id(op)).unwrap_err(),
            RevisionError::Ledger(LedgerError::RootDisagrees)
        );
    }
    assert_eq!(
        RevisionError::Ledger(LedgerError::RootDisagrees).code(),
        ErrorCode::IntegrityError
    );
    let report = quiescent(&store);
    assert_eq!(report.disagreements, vec![id(250), id(251)]);
    // Reporting changes nothing: still complete, still the same ledger.
    assert_eq!(fx.ledger_bytes(), bytes);
    assert_complete_root(&store, 1);
    // A forged commit does not let a commit of that op id "replay".
    let mut forged = draft(&store, 50);
    forged.op = id(250);
    assert_eq!(
        store.commit(&forged, &expect(1)).unwrap_err(),
        RevisionError::Ledger(LedgerError::RootDisagrees)
    );
    assert!(!fx.log(2).exists());
}

#[test]
fn contradicting_accepted_records_make_the_ledger_inconsistent() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let mut bytes = fx.ledger_bytes();
    bytes.extend(forged_commit(201, 101, 1));
    fx.set_ledger(&bytes);
    let inconsistent = RevisionError::Ledger(LedgerError::Inconsistent);
    assert_eq!(store.query_outcome(&id(201)).unwrap_err(), inconsistent);
    assert_eq!(
        store.recover(RecoveryPolicy::quiescent()).unwrap_err(),
        inconsistent
    );
    let d = draft(&store, 2);
    assert_eq!(store.commit(&d, &expect(1)).unwrap_err(), inconsistent);
    assert!(
        !fx.log(2).exists(),
        "a contradictory ledger blocks the commit before the root"
    );
}

#[test]
fn an_unreadable_root_leaves_intents_open_and_unproven() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let d = draft(&store, 2);
    crash_at(&store, &d, &expect(1), CommitPoint::RecordStaged);
    // A foreign file in the head log makes the root unverifiable.
    fs::write(fx.0.join("revisions").join("notes.txt"), b"x").unwrap();
    assert_eq!(store.query_outcome(&id(202)).unwrap(), Outcome::Unknown);
    let report = quiescent(&store);
    assert_eq!(report.root_error, Some(RevisionError::TornRoot));
    assert_eq!(report.unprovable, vec![(id(202), RevisionError::TornRoot)]);
    assert!(report.committed.is_empty() && report.aborted.is_empty());
    assert!(report.unreferenced.is_none());
    assert_eq!(fx.scan().open_intents().count(), 1);
    assert_eq!(store.query_outcome(&id(202)).unwrap(), Outcome::Unknown);
    // Once the root verifies again the same recovery decides.
    fs::remove_file(fx.0.join("revisions").join("notes.txt")).unwrap();
    assert_eq!(quiescent(&store).aborted, vec![id(202)]);
}

#[test]
fn an_attempt_that_lost_the_race_to_another_revision_is_aborted_not_committed() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    // Our attempt for sequence 2 crashed after staging ...
    let ours = draft(&store, 2);
    crash_at(&store, &ours, &expect(1), CommitPoint::RecordStaged);
    // ... and another committer took sequence 2 with a different operation.
    let mut other = draft(&store, 3);
    other.workspace_generation = 99;
    let winner = store.commit(&other, &expect(1)).unwrap().info;
    assert_eq!(winner.sequence(), 2);
    let report = quiescent(&store);
    assert_eq!(report.aborted, vec![id(202)]);
    assert!(report.committed.is_empty());
    assert_ledger_agrees_with_root(&store, 1);
    assert_complete_root(&store, 2);
    assert_eq!(
        store.query_outcome(&id(202)).unwrap(),
        Outcome::Aborted {
            reason: AbortReason::Recovered
        }
    );
}

// ---- recovery policy, temporary files, unreferenced blobs ------------------------------------

#[test]
fn recovery_leaves_a_young_intent_and_its_files_alone_until_it_may_assume_a_crash() {
    let fx = Fixture::new();
    let store = fx.store();
    let d = draft(&store, 1);
    crash_at(&store, &d, &expect(0), CommitPoint::RecordStaged);
    fx.plant_temporaries();
    let report = store.recover(RecoveryPolicy::live(3600)).unwrap();
    assert_eq!(report.in_flight, vec![id(201)]);
    assert!(report.committed.is_empty() && report.aborted.is_empty());
    assert_eq!(report.removed_temp_files, 0);
    assert_eq!(store.query_outcome(&id(201)).unwrap(), Outcome::Unknown);
    assert_eq!(fx.scan().open_intents().count(), 1);
    // Its layer blobs are protected from the unreferenced report while it is in flight.
    assert_eq!(
        report.unreferenced.as_ref().map(|u| u.total),
        Some(1),
        "only its manifest blob"
    );
    assert_eq!(RecoveryPolicy::live(3600).min_age_secs(), 3600);

    let report = quiescent(&store);
    assert_eq!(report.aborted, vec![id(201)]);
    assert_eq!(report.removed_temp_files, 2);
}

#[test]
fn recovery_removes_only_abandoned_temporaries_and_reports_unreferenced_blobs_without_deleting() {
    let fx = Fixture::new();
    let store = fx.store();
    let one = commit_n(&store, 1);
    let d2 = draft(&store, 2);
    crash_at(&store, &d2, &expect(1), CommitPoint::ManifestStored);
    fx.plant_temporaries();
    // A non-temporary look-alike and a directory with the temp prefix are left alone.
    fs::write(fx.0.join("blobs").join("keep.me"), b"keep").unwrap();
    fs::create_dir(fx.0.join("blobs").join(".publish-dir")).unwrap();

    let aborted_manifest = Manifest::assemble(
        Content::from_draft(&d2).unwrap(),
        2,
        Some(BaseRef {
            revision: id(101),
            manifest: one.manifest_digest(),
        }),
        None,
    )
    .unwrap();
    let manifest_digest = Sha256::digest(&aborted_manifest.encode()).unwrap();
    assert!(fx.blob(manifest_digest).exists());

    let report = quiescent(&store);
    assert_eq!(report.removed_temp_files, 2);
    assert!(fx.0.join("blobs").join("keep.me").exists());
    assert!(fx.0.join("blobs").join(".publish-dir").is_dir());
    assert!(!fx.0.join("revisions").join(".tmp-deadbeef-0000").exists());

    let unreferenced = report.unreferenced.unwrap();
    assert_eq!(
        unreferenced.total, 5,
        "4 layers + the manifest of the aborted attempt"
    );
    assert!(unreferenced.digests.contains(&manifest_digest));
    assert!(unreferenced.digests.windows(2).all(|w| w[0] < w[1]));
    for entry in d2.entries.iter() {
        assert!(unreferenced.digests.contains(&entry.digest()));
    }
    for entry in one.manifest().content().entries() {
        assert!(!unreferenced.digests.contains(&entry.digest()));
    }
    // Nothing was deleted: every reported blob is still there, and the root's blobs verify.
    for digest in &unreferenced.digests {
        assert!(fx.blob(*digest).exists());
    }
    assert_complete_root(&store, 1);
    assert!(!unreferenced.digests.contains(&one.manifest_digest()));
}

// ---- boundaries, caps, hostile namespace -----------------------------------------------------

#[test]
fn a_full_ledger_refuses_the_commit_before_anything_is_written() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let mut bytes = fx.ledger_bytes();
    let line = vec![b'x'; MAX_RECORD_BYTES - 2];
    let cap = usize::try_from(MAX_LEDGER_BYTES).unwrap();
    while bytes.len() + line.len() + 1 + 50 <= cap {
        bytes.extend(&line);
        bytes.push(b'\n');
    }
    // Leave 50 bytes: less than any intent record.
    let rest = cap - bytes.len() - 50;
    bytes.extend(vec![b'x'; rest - 1]);
    bytes.push(b'\n');
    fx.set_ledger(&bytes);
    assert!(store.query_outcome(&id(201)).is_ok());
    let d = draft(&store, 2);
    let error = store.commit(&d, &expect(1)).unwrap_err();
    assert_eq!(error, RevisionError::Ledger(LedgerError::LimitExceeded));
    assert_eq!(error.code(), ErrorCode::LimitExceeded);
    assert!(!fx.log(2).exists());
    assert_eq!(fx.ledger_bytes(), bytes);
    assert_complete_root(&store, 1);

    // Above the cap the file is not read at all.
    let mut huge = bytes.clone();
    huge.resize(
        usize::try_from(MAX_LEDGER_BYTES + LEDGER_HEADROOM_BYTES).unwrap() + 1,
        b'\n',
    );
    fx.set_ledger(&huge);
    assert_eq!(
        store.query_outcome(&id(201)).unwrap_err(),
        RevisionError::Ledger(LedgerError::LimitExceeded)
    );
}

#[test]
fn an_unsafe_ledger_entry_fails_the_commit_before_the_root_and_after_the_switch_is_unknown() {
    let fx = Fixture::new();
    let store = fx.store();
    // A directory where the ledger file belongs: refused before the root is touched.
    fs::create_dir_all(fx.ledger_file()).unwrap();
    let d = draft(&store, 1);
    assert_eq!(
        store.commit(&d, &expect(0)).unwrap_err(),
        RevisionError::Ledger(LedgerError::Storage(StorageError::UnsafeEntry))
    );
    assert!(!fx.0.join("revisions").exists());
    assert_eq!(store.head().unwrap(), None);
    fs::remove_dir(fx.ledger_file()).unwrap();

    // The ledger breaks after the intent: the switch happens, the Committed record cannot be
    // written, and the caller hears OUTCOME_UNKNOWN, never success.
    let result = store.commit_with(
        &d,
        &expect(0),
        None,
        &mut |p: CommitPoint| -> Result<(), Injected> {
            if p == CommitPoint::Switched {
                fs::remove_file(fx.ledger_file()).unwrap();
                fs::create_dir(fx.ledger_file()).unwrap();
            }
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err(), RevisionError::OutcomeUnknown);
    assert_complete_root(&store, 1);
    // The ledger is unusable, so QueryOutcome reports an error rather than a guess ...
    assert!(store.query_outcome(&id(201)).is_err());
    // ... and once the namespace is repaired the root still proves the operation.
    fs::remove_dir(fx.ledger_file()).unwrap();
    assert!(is_committed(
        &store.query_outcome(&id(201)).unwrap(),
        OutcomeSource::Root
    ));
}

#[test]
fn absence_is_proven_only_when_the_ledger_covers_the_whole_history() {
    // A store whose revisions predate the ledger: an op that is not recorded is unknown, not
    // "not applied".
    let fx = Fixture::new();
    let store = fx.store();
    for n in 1..=3 {
        commit_n(&store, n);
    }
    fs::remove_dir_all(fx.ledger_dir()).unwrap();
    assert!(is_committed(
        &store.query_outcome(&id(203)).unwrap(),
        OutcomeSource::Root
    ));
    assert_eq!(store.query_outcome(&id(299)).unwrap(), Outcome::Unknown);
    // Recovery on such a store is a no-op for the ledger (nothing to close).
    assert!(quiescent(&store).is_noop());
    // A later commit starts a ledger whose genesis records the head it found.
    commit_n(&store, 4);
    assert_eq!(fx.scan().genesis_head(), Some(3));
    assert_eq!(store.query_outcome(&id(299)).unwrap(), Outcome::Unknown);
    assert!(is_committed(
        &store.query_outcome(&id(204)).unwrap(),
        OutcomeSource::Ledger
    ));

    // A ledger that began on an empty store proves absence; a damaged line does not.
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    assert_eq!(store.query_outcome(&id(299)).unwrap(), Outcome::NotFound);
    let mut bytes = fx.ledger_bytes();
    bytes.extend(b"garbage line that may have been a record\n");
    fx.set_ledger(&bytes);
    assert_eq!(store.query_outcome(&id(299)).unwrap(), Outcome::Unknown);
}

#[test]
fn concurrent_recoveries_converge_and_never_conflict() {
    for round in 0..10 {
        let fx = Fixture::new();
        let store = fx.store();
        commit_n(&store, 1);
        let d = draft(&store, 2);
        let point = if round % 2 == 0 {
            CommitPoint::Switched
        } else {
            CommitPoint::RecordStaged
        };
        crash_at(&store, &d, &expect(1), point);
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| {
                    let mine = fx.store();
                    barrier.wait();
                    mine.recover(RecoveryPolicy::quiescent()).unwrap();
                    mine.query_outcome(&id(202)).unwrap();
                });
            }
        });
        let switched = point == CommitPoint::Switched;
        assert_complete_root(&store, if switched { 2 } else { 1 });
        assert_eq!(fx.scan().open_intents().count(), 0, "round {round}");
        assert_ledger_agrees_with_root(&store, if switched { 2 } else { 1 });
        let settled = fx.ledger_bytes();
        assert!(quiescent(&store).is_noop());
        assert_eq!(fx.ledger_bytes(), settled);
        match store.query_outcome(&id(202)).unwrap() {
            Outcome::Committed { .. } => assert!(switched),
            Outcome::Aborted { reason } => {
                assert!(!switched);
                assert_eq!(reason, AbortReason::Recovered);
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn open_recovered_returns_the_verified_root_with_the_report() {
    let fx = Fixture::new();
    let store = fx.store();
    assert_eq!(
        store
            .open_recovered(RecoveryPolicy::quiescent())
            .unwrap_err(),
        RevisionError::NoRevision
    );
    commit_n(&store, 1);
    let d = draft(&store, 2);
    crash_at(&store, &d, &expect(1), CommitPoint::Switched);
    let (revision, report) = fx
        .store()
        .open_recovered(RecoveryPolicy::quiescent())
        .unwrap();
    assert_eq!(revision.info().sequence(), 2);
    assert_eq!(report.committed, vec![id(202)]);
}

// ---- review findings: capacity, outcome proofs, attempt identity -----------------------------

/// Commits `count` revisions 1..=count.
fn commit_many(store: &RevisionStore, count: u32) {
    for n in 1..=count {
        commit_n(store, n);
    }
}

fn open_intent(op: u32) -> Record {
    Record::Intent(Intent {
        op: id(op),
        project: id(1),
        revision: id(100_000 + op),
        sequence: 1,
        base: None,
        fingerprint: digest(&format!("content {op}")),
        created: 0,
        attempt: digest(&format!("attempt {op}")),
        blobs: vec![digest("blob")],
    })
}

#[test]
fn admission_refuses_an_attempt_that_would_break_the_readers_limits() {
    // 4,096 unresolved intents: a 4,097th attempt is refused, not written and then unreadable.
    let fx = Fixture::new();
    let store = fx.store();
    let mut bytes = Record::Genesis { head: 0 }.encode().0;
    for op in 0..u32::try_from(MAX_UNRESOLVED_INTENTS).unwrap() {
        bytes.extend(open_intent(10_000 + op).encode().0);
    }
    fx.set_ledger(&bytes);
    let d = draft(&store, 1);
    assert_eq!(
        store.commit(&d, &expect(0)).unwrap_err(),
        RevisionError::Ledger(LedgerError::LimitExceeded)
    );
    assert_eq!(fx.ledger_bytes(), bytes, "nothing was appended");
    assert!(!fx.0.join("revisions").exists());
    // The ledger still reads, and recovery can close everything that is open.
    assert!(store.query_outcome(&id(10_000)).is_ok());
    let report = quiescent(&store);
    assert_eq!(report.aborted.len(), MAX_UNRESOLVED_INTENTS);
    assert_eq!(
        store.commit(&d, &expect(0)).unwrap().outcome,
        CommitOutcome::Created
    );

    // A ledger at the line limit refuses too.
    let fx = Fixture::new();
    let store = fx.store();
    let mut lines = Record::Genesis { head: 0 }.encode().0;
    lines.extend(vec![b'\n'; MAX_LEDGER_RECORDS - 2]);
    fx.set_ledger(&lines);
    assert_eq!(
        store.commit(&draft(&store, 1), &expect(0)).unwrap_err(),
        RevisionError::Ledger(LedgerError::LimitExceeded)
    );
    assert_eq!(fx.ledger_bytes(), lines);
    assert!(store.query_outcome(&id(201)).is_ok());
}

#[test]
fn admission_reserves_room_for_the_closing_record_and_recovery_may_use_the_headroom() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let cap = usize::try_from(MAX_LEDGER_BYTES).unwrap();
    let pad = |fx: &Fixture, leave: usize| {
        let mut bytes = fx.ledger_bytes();
        while bytes.len() + MAX_RECORD_BYTES < cap - leave {
            bytes.extend(vec![b'x'; MAX_RECORD_BYTES - 2]);
            bytes.push(b'\n');
        }
        let rest = cap - leave - bytes.len();
        bytes.extend(vec![b'x'; rest - 1]);
        bytes.push(b'\n');
        fx.set_ledger(&bytes);
        bytes
    };
    // 800 bytes left: the intent (about 630) fits, but not together with its closing record.
    let bytes = pad(&fx, 800);
    assert_eq!(bytes.len(), cap - 800);
    let d = draft(&store, 2);
    assert_eq!(
        store.commit(&d, &expect(1)).unwrap_err(),
        RevisionError::Ledger(LedgerError::LimitExceeded)
    );
    assert_eq!(fx.ledger_bytes(), bytes);
    assert!(!fx.log(2).exists());

    // An attempt admitted earlier can still be closed even when the file is past the base limit:
    // closing records use the headroom.
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let d = draft(&store, 2);
    crash_at(&store, &d, &expect(1), CommitPoint::RecordStaged);
    let bytes = pad(&fx, 50);
    assert!(bytes.len() > cap - 100);
    let report = quiescent(&store);
    assert_eq!(report.aborted, vec![id(202)]);
    assert!(fx.ledger_bytes().len() > bytes.len());
    assert!(
        u64::try_from(fx.ledger_bytes().len()).unwrap() <= MAX_LEDGER_BYTES + LEDGER_HEADROOM_BYTES
    );
    assert_eq!(fx.scan().open_intents().count(), 0);
    // The padding is damaged history, so the ledger alone cannot prove the op never happened.
    assert_eq!(store.query_outcome(&id(202)).unwrap(), Outcome::Unknown);
}

#[test]
fn an_abort_is_checked_against_the_root_and_damaged_history_is_not_a_proof() {
    let fx = Fixture::new();
    let store = fx.store();
    let d = draft(&store, 1);
    // Attempt 1 is cancelled; the retry switches the root and crashes before `committed`.
    let cancelled = store.commit_with(&d, &expect(0), None, &mut |p: CommitPoint| {
        if p == CommitPoint::ManifestStored {
            Err(Injected)
        } else {
            Ok(())
        }
    });
    assert!(cancelled.is_err());
    crash_at(&store, &d, &expect(0), CommitPoint::Switched);
    // The retry's intent line is damaged: only the earlier abort is left in the ledger.
    let mut bytes = fx.ledger_bytes();
    let text = String::from_utf8(bytes.clone()).unwrap();
    let start = text.rfind("L1 intent").unwrap();
    bytes[start + 12] ^= 0x01;
    fx.set_ledger(&bytes);
    assert_eq!(fx.scan().diagnostics().damaged_lines, 1);
    assert!(is_committed(
        &store.query_outcome(&id(201)).unwrap(),
        OutcomeSource::Root
    ));

    // A damaged ledger cannot prove that every attempt was closed: Unknown, not Aborted.
    let fx = Fixture::new();
    let store = fx.store();
    let d = draft(&store, 1);
    let cancelled = store.commit_with(&d, &expect(0), None, &mut |p: CommitPoint| {
        if p == CommitPoint::ManifestStored {
            Err(Injected)
        } else {
            Ok(())
        }
    });
    assert!(cancelled.is_err());
    assert_eq!(
        store.query_outcome(&id(201)).unwrap(),
        Outcome::Aborted {
            reason: AbortReason::Cancelled
        }
    );
    let mut bytes = fx.ledger_bytes();
    bytes.extend(b"damaged line, maybe a lost intent\n");
    fx.set_ledger(&bytes);
    assert_eq!(store.query_outcome(&id(201)).unwrap(), Outcome::Unknown);
}

#[test]
fn not_found_needs_the_ledger_to_account_for_every_revision_of_the_root() {
    let fx = Fixture::new();
    let store = fx.store();
    let total = u32::try_from(REPLAY_LOOKBACK).unwrap() + 6;
    commit_many(&store, total);
    assert_eq!(store.query_outcome(&id(999)).unwrap(), Outcome::NotFound);

    // Cut the ledger at a record boundary after revision 2: genesis 0 and no damaged line, but
    // revisions 3.. are not accounted for, so an unrecorded op is unknown, not "never applied".
    let full = fx.ledger_bytes();
    let mut ends = full
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .map(|(i, _)| i + 1);
    let keep = ends.nth(2 * 2).unwrap(); // genesis + 2 x (intent, committed)
    fx.set_ledger(&full[..keep]);
    let scan = fx.scan();
    assert_eq!(scan.genesis_head(), Some(0));
    assert_eq!(scan.diagnostics().damaged_lines, 0);
    assert_eq!(store.query_outcome(&id(999)).unwrap(), Outcome::Unknown);
    // An op the cut dropped and the window no longer shows is unknown as well.
    assert_eq!(store.query_outcome(&id(203)).unwrap(), Outcome::Unknown);
    // Recovery cannot rebuild what was never recorded; it reports nothing false.
    assert!(quiescent(&store).is_noop());
}

#[test]
fn a_committed_record_must_name_the_operation_and_every_old_record_is_checked() {
    let fx = Fixture::new();
    let store = fx.store();
    let one = commit_n(&store, 1);
    // Another op id claims revision 1's real revision, sequence and manifest digest.
    let mut bytes = fx.ledger_bytes();
    bytes.extend(
        Record::Committed(CommittedRecord {
            op: id(777),
            revision: id(101),
            sequence: 1,
            manifest: one.manifest_digest(),
        })
        .encode()
        .0,
    );
    fx.set_ledger(&bytes);
    assert_eq!(
        store.query_outcome(&id(777)).unwrap_err(),
        RevisionError::Ledger(LedgerError::RootDisagrees)
    );
    assert_eq!(quiescent(&store).disagreements, vec![id(777)]);
    // It cannot make a commit of that op id "replay" either.
    let mut other = draft(&store, 50);
    other.op = id(777);
    assert_eq!(
        store.commit(&other, &expect(1)).unwrap_err(),
        RevisionError::Ledger(LedgerError::RootDisagrees)
    );

    // Records older than the replay window are verified by recovery, not skipped.
    let fx = Fixture::new();
    let store = fx.store();
    commit_many(&store, u32::try_from(REPLAY_LOOKBACK).unwrap() + 6);
    let mut bytes = fx.ledger_bytes();
    bytes.extend(forged_commit(778, 101, 1));
    fx.set_ledger(&bytes);
    let report = quiescent(&store);
    assert_eq!(report.disagreements, vec![id(778)]);
    assert!(report.root_error.is_none());
}

#[test]
fn an_identical_retry_after_a_crash_at_the_switch_replays_even_after_the_root_moved_on() {
    let fx = Fixture::new();
    let store = fx.store();
    commit_n(&store, 1);
    let d = draft(&store, 2);
    crash_at(&store, &d, &expect(1), CommitPoint::Switched);
    // Other operations push revision 2 far out of the replay window while its intent is open.
    for n in 3..u32::try_from(REPLAY_LOOKBACK).unwrap() + 6 {
        commit_n(&store, n);
    }
    assert_eq!(fx.scan().open_intents().count(), 1);
    let before = store.head().unwrap().unwrap();
    let retry = store.commit(&d, &expect(1)).unwrap();
    assert_eq!(retry.outcome, CommitOutcome::Replayed);
    assert_eq!(retry.info.sequence(), 2);
    assert_eq!(
        store.head().unwrap().unwrap(),
        before,
        "no second publication"
    );
    assert!(is_committed(
        &store.query_outcome(&id(202)).unwrap(),
        OutcomeSource::Ledger
    ));
}

#[test]
fn each_attempt_has_its_own_identity_and_an_abort_closes_only_that_attempt() {
    let fx = Fixture::new();
    let store = fx.store();
    let d = draft(&store, 1);
    // Two identical attempts of the same op and content, started back to back (same second),
    // the first left unresolved by a crash, the second cancelled.
    crash_at(&store, &d, &expect(0), CommitPoint::RecordStaged);
    let cancelled = store.commit_with(&d, &expect(0), None, &mut |p: CommitPoint| {
        if p == CommitPoint::ManifestStored {
            Err(Injected)
        } else {
            Ok(())
        }
    });
    assert_eq!(
        cancelled.unwrap_err(),
        RevisionError::Aborted(CommitPoint::ManifestStored)
    );
    let scan = fx.scan();
    assert_eq!(
        scan.open_intents().count(),
        1,
        "the first attempt is still open"
    );
    let attempts: Vec<_> = scan.open_intents().map(|(_, i)| i.attempt).collect();
    assert_ne!(attempts[0], digest("attempt"));
    // The op is not reported as aborted while an attempt is unresolved.
    assert_eq!(store.query_outcome(&id(201)).unwrap(), Outcome::Unknown);
}
