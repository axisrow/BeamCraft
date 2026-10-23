// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Recovery of unresolved intents, QueryOutcome and the read-only reports around them.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use beamcraft_model::digest::ContentDigest;

use crate::lease::StoreLease;
use crate::revision::{
    REPLAY_LOOKBACK, Revision, RevisionError, RevisionInfo, RevisionStore, Uuid7,
};
use crate::storage::StorageError;

use super::record::{AbortReason, CommittedRecord, Intent, Record};
use super::state::{LedgerDiagnostics, OpStatus};
use super::{LedgerError, Outcome, OutcomeSource, now_secs};

/// Most revisions whose manifests one unreferenced-blob report walks.
const MAX_REFERENCE_SCAN: u64 = 4096;
/// Most blob files one unreferenced-blob report lists.
const MAX_LISTED_BLOBS: usize = 262_144;
/// Most unreferenced digests a report carries (the total is always exact).
const MAX_REPORTED_UNREFERENCED: usize = 1024;

/// How recovery decides whether an intent belongs to a crashed attempt.
///
/// Recovery cannot see whether another process is still committing. An intent is only closed,
/// and a temporary file only removed, when it is at least `min_age_secs` old. Zero asserts that
/// no commit is running on this store (host start-up, an operator after a crash). There is
/// deliberately no `Default`: the caller must state the assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryPolicy {
    min_age_secs: u64,
}

impl RecoveryPolicy {
    /// Asserts that no commit is running: every unresolved intent and every temporary file is
    /// treated as the remains of a crash.
    pub const fn quiescent() -> Self {
        Self { min_age_secs: 0 }
    }

    /// Leaves alone anything younger than `min_age_secs` seconds, because a live committer may
    /// still own it.
    pub const fn live(min_age_secs: u64) -> Self {
        Self { min_age_secs }
    }

    /// The configured minimum age in seconds.
    pub const fn min_age_secs(self) -> u64 {
        self.min_age_secs
    }
}

/// Blobs no retained revision's manifest references. A report, never a deletion list: drafts
/// and workspaces that are not yet published also hold unreferenced blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreferenced {
    /// Exact number of unreferenced blobs found.
    pub total: usize,
    /// The first (ascending) at most 1024 of them.
    pub digests: Vec<ContentDigest>,
}

/// What one recovery run found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Operations this run closed as committed (the root holds their revision).
    pub committed: Vec<Uuid7>,
    /// Operations this run closed as not accepted (the root never switched to them).
    pub aborted: Vec<Uuid7>,
    /// Operations whose intent is younger than the policy age: left alone, answered `Unknown`.
    pub in_flight: Vec<Uuid7>,
    /// Operations that cannot be decided because the root could not be read; they stay
    /// unresolved. The error says why.
    pub unprovable: Vec<(Uuid7, RevisionError)>,
    /// Operations the ledger records as committed although the root does not contain them
    /// (lost directory entry, tampering). Nothing is changed for them.
    pub disagreements: Vec<Uuid7>,
    /// Set when the head log itself could not be verified; nothing about the root was decided.
    pub root_error: Option<RevisionError>,
    /// Abandoned temporary files removed (revision record temps and blob publish temps).
    pub removed_temp_files: usize,
    /// Blobs that no revision references, or `None` when that was not computed (the head log
    /// is unreadable or longer than 4096 revisions, or more than 262,144 blobs exist).
    pub unreferenced: Option<Unreferenced>,
    /// What the ledger reader ignored.
    pub ledger: LedgerDiagnostics,
}

impl RecoveryReport {
    /// True when the run changed nothing: no intent closed and no file removed.
    pub fn is_noop(&self) -> bool {
        self.committed.is_empty() && self.aborted.is_empty() && self.removed_temp_files == 0
    }
}

enum Verdict {
    Committed(CommittedRecord),
    NotSwitched,
}

impl RevisionStore {
    /// Closes every unresolved intent from the root, removes abandoned temporary files and
    /// reports unreferenced blobs. See the [module](crate::ledger) documentation for the
    /// promises, the concurrency model and what is not done.
    ///
    /// Per intent: if the head log holds the intent's revision (same id, same op, same content
    /// fingerprint) at the intent's sequence, a `Committed` record is appended; if the root has
    /// not reached that sequence or holds another revision there, an `Aborted` record is
    /// appended; if the root cannot be verified the intent stays open and is reported. A
    /// second call finds nothing open and changes nothing.
    pub fn recover(&self, policy: RecoveryPolicy) -> Result<RecoveryReport, RevisionError> {
        // Recovery appends to the ledger, so it never runs beside a history compaction.
        let _lease = StoreLease::shared(&self.blobs)?;
        let ledger = self.ledger();
        let scan = ledger.scan()?;
        let directory = self.head_dir()?;
        let head = match &directory {
            Some(directory) => self.load_head_in(directory),
            None => Ok(None),
        };
        let now = now_secs();

        let mut decided: Vec<(&Intent, &ContentDigest, Verdict)> = Vec::new();
        let mut in_flight = Vec::new();
        let mut unprovable = Vec::new();
        let mut protected = BTreeSet::new();
        for (id, intent) in scan.open_intents() {
            if now.saturating_sub(intent.created) < policy.min_age_secs {
                in_flight.push(intent.op.clone());
                protected.extend(intent.blobs.iter().copied());
                continue;
            }
            match self.judge(intent, directory.as_deref(), &head) {
                Ok(verdict) => decided.push((intent, id, verdict)),
                Err(error) => {
                    unprovable.push((intent.op.clone(), error));
                    protected.extend(intent.blobs.iter().copied());
                }
            }
        }

        let mut records = Vec::new();
        let (mut committed, mut aborted) = (Vec::new(), Vec::new());
        let mut committed_ops = BTreeSet::new();
        for (intent, _, verdict) in &decided {
            if let Verdict::Committed(record) = verdict
                && committed_ops.insert(intent.op.clone())
            {
                records.push(Record::Committed(record.clone()));
                committed.push(intent.op.clone());
            }
        }
        let mut aborted_ops = BTreeSet::new();
        for (intent, id, verdict) in &decided {
            if matches!(verdict, Verdict::NotSwitched) && !committed_ops.contains(&intent.op) {
                records.push(Record::Aborted {
                    op: intent.op.clone(),
                    intent: **id,
                    reason: AbortReason::Recovered,
                });
                if aborted_ops.insert(intent.op.clone()) {
                    aborted.push(intent.op.clone());
                }
            }
        }
        if !records.is_empty() {
            let genesis = scan.genesis_head().is_none().then(|| {
                head.as_ref()
                    .ok()
                    .and_then(Option::as_ref)
                    .map_or(0, RevisionInfo::sequence)
            });
            ledger.append(&records, genesis)?;
        }

        let mut disagreements: Vec<Uuid7> = match &head {
            Ok(head) => scan
                .committed()
                .filter(|record| self.disagrees(directory.as_deref(), head.as_ref(), record))
                .map(|record| record.op.clone())
                .collect(),
            Err(_) => Vec::new(),
        };
        disagreements.sort();

        let removed_temp_files =
            self.remove_temporaries(Duration::from_secs(policy.min_age_secs))?;
        let unreferenced = match &head {
            Ok(head) => self.unreferenced(directory.as_deref(), head.as_ref(), &protected)?,
            Err(_) => None,
        };
        Ok(RecoveryReport {
            committed,
            aborted,
            in_flight,
            unprovable,
            disagreements,
            root_error: head.err(),
            removed_temp_files,
            unreferenced,
            ledger: scan.diagnostics(),
        })
    }

    /// Runs [`Self::recover`] and then opens the root, so a caller that restarts after a crash
    /// gets the verified complete revision together with what recovery did. Nothing published
    /// is [`RevisionError::NoRevision`].
    pub fn open_recovered(
        &self,
        policy: RecoveryPolicy,
    ) -> Result<(Revision, RecoveryReport), RevisionError> {
        let report = self.recover(policy)?;
        Ok((self.open()?, report))
    }

    /// The host-ledger QueryOutcome for one operation id. Read-only: it appends nothing, so it
    /// is safe while commits run. See the [module](crate::ledger) documentation for the exact
    /// meaning of each answer. A ledger `Committed` that the root contradicts is
    /// [`LedgerError::RootDisagrees`], not an acceptance.
    pub fn query_outcome(&self, op: &Uuid7) -> Result<Outcome, RevisionError> {
        let scan = self.ledger().scan()?;
        let directory = self.head_dir()?;
        let head = match &directory {
            Some(directory) => self.load_head_in(directory),
            None => Ok(None),
        };
        match scan.status(op) {
            OpStatus::Committed(record) => {
                // A record below the compaction floor is a tombstone: the revision is gone, the
                // acceptance was proven by the root before it was compacted.
                if let Some(directory) = directory.as_deref()
                    && record.sequence < self.floor_of(directory)?
                {
                    return Ok(Outcome::Committed {
                        revision: record.revision,
                        sequence: record.sequence,
                        manifest: record.manifest,
                        source: OutcomeSource::Ledger,
                    });
                }
                match self.load_committed(directory.as_deref(), &record) {
                    Ok(_) => Ok(Outcome::Committed {
                        revision: record.revision,
                        sequence: record.sequence,
                        manifest: record.manifest,
                        source: OutcomeSource::Ledger,
                    }),
                    Err(error @ RevisionError::Ledger(LedgerError::RootDisagrees)) => Err(error),
                    Err(_) => Ok(Outcome::Unknown),
                }
            }
            OpStatus::Unresolved => {
                for intent in scan.open_intents_of(op) {
                    if let Ok(Verdict::Committed(record)) =
                        self.judge(intent, directory.as_deref(), &head)
                    {
                        return Ok(Outcome::Committed {
                            revision: record.revision,
                            sequence: record.sequence,
                            manifest: record.manifest,
                            source: OutcomeSource::Root,
                        });
                    }
                }
                Ok(Outcome::Unknown)
            }
            OpStatus::Aborted(reason) => {
                // An abort is only a statement about one attempt. A retry may have switched the
                // root with its own intent damaged or lost, so the root is consulted first, and
                // a ledger with ignored lines cannot prove that every attempt was closed.
                let Ok(head) = &head else {
                    return Ok(Outcome::Unknown);
                };
                if let (Some(head), Some(directory)) = (head, directory.as_deref())
                    && let Some(info) = self.find_op(directory, head, op)
                {
                    return Ok(Outcome::Committed {
                        revision: info.revision().clone(),
                        sequence: info.sequence(),
                        manifest: info.manifest_digest(),
                        source: OutcomeSource::Root,
                    });
                }
                Ok(if scan.diagnostics().damaged_lines == 0 {
                    Outcome::Aborted { reason }
                } else {
                    Outcome::Unknown
                })
            }
            OpStatus::Unseen => {
                let Ok(head) = head else {
                    return Ok(Outcome::Unknown);
                };
                // Lines the reader had to ignore may have been records of this very operation.
                let intact = scan.diagnostics().damaged_lines == 0;
                let Some(head) = head else {
                    // Nothing is published. A ledger that began on an empty store proves the
                    // operation never reached the root; otherwise the revisions it would be
                    // among may be missing, and that is not a proof either way.
                    return Ok(
                        if intact && scan.genesis_head().is_none_or(|first| first == 0) {
                            Outcome::NotFound
                        } else {
                            Outcome::Unknown
                        },
                    );
                };
                let Some(directory) = directory.as_deref() else {
                    return Ok(Outcome::Unknown);
                };
                if let Some(info) = self.find_op(directory, &head, op) {
                    return Ok(Outcome::Committed {
                        revision: info.revision().clone(),
                        sequence: info.sequence(),
                        manifest: info.manifest_digest(),
                        source: OutcomeSource::Root,
                    });
                }
                // Revisions that predate the ledger were never journaled, and a ledger cut at a
                // record boundary looks intact: absence from the ledger is a proof only when the
                // ledger began before any revision existed and accounts for every sequence of
                // the root. After a U14 compaction the root starts at its floor: the ledger must
                // account for every retained sequence, and an operation of the removed history
                // is, by decision, no longer recognised (so not finding it proves nothing was
                // lost and nothing new is pending).
                let head_sequence = head.sequence();
                let before = self.floor_of(directory)?.saturating_sub(1);
                let covered =
                    scan.covered(before, head_sequence) == head_sequence.saturating_sub(before);
                Ok(
                    if intact && scan.genesis_head().is_some_and(|first| first <= before) && covered
                    {
                        Outcome::NotFound
                    } else {
                        Outcome::Unknown
                    },
                )
            }
        }
    }

    /// Decides one intent from the root alone.
    fn judge(
        &self,
        intent: &Intent,
        directory: Option<&Path>,
        head: &Result<Option<RevisionInfo>, RevisionError>,
    ) -> Result<Verdict, RevisionError> {
        let Some(head) = head.as_ref().map_err(|error| *error)? else {
            return Ok(Verdict::NotSwitched);
        };
        if intent.sequence > head.sequence() {
            return Ok(Verdict::NotSwitched);
        }
        let directory = directory.ok_or(RevisionError::TornRoot)?;
        let info = self.load_at(directory, intent.sequence)?;
        let content = info.manifest().content();
        let same = info.revision() == &intent.revision
            && content.op() == &intent.op
            && content.fingerprint()? == intent.fingerprint;
        Ok(if same {
            Verdict::Committed(CommittedRecord {
                op: intent.op.clone(),
                revision: intent.revision.clone(),
                sequence: intent.sequence,
                manifest: info.manifest_digest(),
            })
        } else {
            Verdict::NotSwitched
        })
    }

    /// Loads the head-log revision a `Committed` record names and checks that the root really
    /// holds it. A record the root contradicts is [`LedgerError::RootDisagrees`]; a root that
    /// cannot be read is its own error, which proves nothing either way.
    pub(crate) fn load_committed(
        &self,
        directory: Option<&Path>,
        record: &CommittedRecord,
    ) -> Result<RevisionInfo, RevisionError> {
        let disagree = RevisionError::Ledger(LedgerError::RootDisagrees);
        let directory = directory.ok_or(disagree)?;
        let reached = self
            .load_head_in(directory)?
            .is_some_and(|head| head.sequence() >= record.sequence);
        if !reached {
            return Err(disagree);
        }
        let info = self.load_at(directory, record.sequence)?;
        if info.revision() != &record.revision
            || info.manifest_digest() != record.manifest
            || info.manifest().content().op() != &record.op
        {
            return Err(disagree);
        }
        Ok(info)
    }

    /// True when the root demonstrably lacks what a `Committed` record claims: a sequence the
    /// root has not reached, a record that cannot be read, or another revision, manifest or
    /// operation at that sequence. Every recorded outcome is checked, not only recent ones.
    fn disagrees(
        &self,
        directory: Option<&Path>,
        head: Option<&RevisionInfo>,
        record: &CommittedRecord,
    ) -> bool {
        let (Some(head), Some(directory)) = (head, directory) else {
            return true;
        };
        if record.sequence > head.sequence() {
            return true;
        }
        // Compacted away on purpose (U14): nothing in the log can contradict it.
        if self
            .floor_of(directory)
            .is_ok_and(|floor| record.sequence < floor)
        {
            return false;
        }
        self.load_at(directory, record.sequence)
            .map_or(true, |info| {
                info.revision() != &record.revision
                    || info.manifest_digest() != record.manifest
                    || info.manifest().content().op() != &record.op
            })
    }

    /// Finds the revision an op id created among the newest [`REPLAY_LOOKBACK`] head-log
    /// records. A read failure ends the walk quietly: the caller then claims nothing.
    fn find_op(&self, directory: &Path, head: &RevisionInfo, op: &Uuid7) -> Option<RevisionInfo> {
        let mut current = head.clone();
        for _ in 0..REPLAY_LOOKBACK {
            if current.manifest().content().op() == op {
                return Some(current);
            }
            let previous = current.sequence().checked_sub(1).filter(|s| *s > 0)?;
            current = self.load_at(directory, previous).ok()?;
        }
        None
    }

    fn remove_temporaries(&self, min_age: Duration) -> Result<usize, RevisionError> {
        let mut removed = 0usize;
        for (name, prefix) in [
            ("revisions", ".tmp-"),
            ("blobs", ".publish-"),
            ("ledger", ".tmp-"),
        ] {
            let directory = match self.blobs.namespace_dir(name, false) {
                Ok(directory) => directory,
                Err(StorageError::NotFound) => continue,
                Err(error) => return Err(error.into()),
            };
            removed =
                removed.saturating_add(self.blobs.remove_stale_temps(&directory, prefix, min_age)?);
        }
        Ok(removed)
    }

    /// Blobs that no manifest of the head log references (the manifest blobs themselves count
    /// as referenced), minus the layer blobs of intents that are still open. `None` when the
    /// answer is not computable within the caps.
    fn unreferenced(
        &self,
        directory: Option<&Path>,
        head: Option<&RevisionInfo>,
        protected: &BTreeSet<ContentDigest>,
    ) -> Result<Option<Unreferenced>, RevisionError> {
        let mut referenced: BTreeSet<ContentDigest> = protected.clone();
        if let Some(head) = head {
            let Some(directory) = directory else {
                return Ok(None);
            };
            let floor = self.floor_of(directory)?;
            if head.sequence().saturating_sub(floor) >= MAX_REFERENCE_SCAN {
                return Ok(None);
            }
            for sequence in floor..=head.sequence() {
                let Ok(info) = self.load_at(directory, sequence) else {
                    return Ok(None);
                };
                referenced.insert(info.manifest_digest());
                referenced.extend(
                    info.manifest()
                        .content()
                        .operation_evidence()
                        .map(|pin| pin.digest()),
                );
                referenced.extend(
                    info.manifest()
                        .content()
                        .entries()
                        .iter()
                        .map(|e| e.digest()),
                );
            }
        }
        let listed = match self.blobs.list_digests(MAX_LISTED_BLOBS) {
            Ok(listed) => listed,
            Err(StorageError::LimitExceeded) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let all: Vec<ContentDigest> = listed
            .into_iter()
            .filter(|digest| !referenced.contains(digest))
            .collect();
        Ok(Some(Unreferenced {
            total: all.len(),
            digests: all.into_iter().take(MAX_REPORTED_UNREFERENCED).collect(),
        }))
    }
}
