// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U14: history statistics, and compaction of the revision store with garbage collection.
//!
//! A store grows with every commit: each revision keeps its record, its manifest blob and every
//! layer blob it names. [`stats`] measures that growth without changing anything. [`plan`] and
//! [`compact`] shrink it: they drop the oldest revisions, keep the newest ones under their
//! original sequence numbers and ids (the head is never renumbered), and delete exactly what no
//! retained revision needs.
//!
//! # What a compaction changes
//!
//! * **Head-log records** below the new *floor* are removed. The floor is the oldest retained
//!   revision; `revisions/floor` states it (see the [`crate::revision`] module for the format) and
//!   makes the log valid as the run `floor..=head`. A removed revision can no longer be opened or
//!   used as an undo target. The report lists every one of them.
//! * **Blobs** that no retained revision references are deleted: layer blobs and manifest blobs of
//!   removed revisions, and blobs no revision ever referenced (left by failed commits) once they
//!   are older than the orphan grace period. Layer blobs that a retained revision still names,
//!   among them every `external` (foreign, read-only) layer, are never touched.
//! * **The outcome ledger** is rewritten without the records of removed revisions, keeping a
//!   configurable tail of tombstones ([`Request::ledger_tail`], default none). An operation id
//!   whose record was dropped is no longer recognised as already accepted; one still in the tail
//!   is answered `Committed` by the ledger and refused as `OP_ID_REUSE` when retried, because its
//!   revision is gone. The records of every retained revision are always kept.
//!
//! # Safety
//!
//! * Compaction needs the exclusive store lease ([`crate::lease`]): it is refused while a
//!   coordinator, a commit or a recovery is active, and they are refused while it runs.
//! * It is refused while an unresolved intent exists in the ledger (run recovery first), and when
//!   a declared live personal workspace is based on a revision that would be removed
//!   ([`Pin`]); it never silently shortens a workspace's base.
//! * Every retained revision is fully verified (record, manifest, base chain inside the retained
//!   run, every layer blob by digest and size) before anything is changed. A damaged retained
//!   revision refuses the compaction.
//! * A real run needs explicit confirmation and a safeguard: a backup made under the exclusive
//!   lease, or an explicit decision to go without ([`Safeguard`]).
//!
//! # Crash safety
//!
//! The steps, in order, are: staged floor marker, **marker install (the commit point)**, removal
//! of records below the floor, ledger replacement, blob deletion. Before the commit point the
//! store is exactly the old one. After it the store is the new one: records below the floor are
//! ignored by readers, a ledger record below the floor is a tombstone, and a blob nobody
//! references is harmless. So an interruption at any step leaves a store whose head opens and
//! whose retained revisions are complete, and running the same compaction again finishes the
//! work and then reports nothing to do. [`CompactionHook`] marks every step for fault injection.
//!
//! # Not done
//!
//! Directory-entry durability (as everywhere in the store), compaction of a personal workspace's
//! draft store (the adapter refuses it), a retention policy chosen by the host, automatic
//! compaction, and any lock that a process ignoring the protocol would respect.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use beamcraft_model::ErrorCode;
use beamcraft_model::digest::ContentDigest;

use crate::lease::{LeaseError, StoreLease};
use crate::ledger::{LedgerError, Record};
use crate::revision::{MAX_REVISIONS, RevisionError, RevisionStore, RootRecord, Span, Uuid7};
use crate::storage::{BlobInfo, StorageError};

/// Largest `keep` count a request may name (the log never holds more revisions).
pub const MAX_KEEP: u64 = MAX_REVISIONS;
/// Largest ledger tail a request may name.
pub const MAX_LEDGER_TAIL: u64 = MAX_REVISIONS;
/// Most blob files one run lists.
const MAX_BLOBS: usize = 262_144;
/// Most files of the head-log directory one run lists.
const MAX_LOG_FILES: usize = 110_000;

/// A revision of this store that a live personal workspace is based on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    /// What names the workspace in reports (its directory).
    pub label: String,
    /// The published revision the workspace was forked from.
    pub revision: Uuid7,
}

/// How much history a compaction keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// Only the current head revision.
    Head,
    /// The newest `n` revisions, `1..=`[`MAX_KEEP`]. More than the log holds keeps all of it.
    Last(u64),
}

/// What to compact.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// How much history to keep.
    pub keep: Keep,
    /// Revisions that live workspaces are based on. A real run needs the caller to have listed
    /// every workspace it can; an unlisted workspace cannot be protected.
    pub pins: &'a [Pin],
    /// Extra ledger records of removed revisions to keep as tombstones, counted in revisions
    /// below the new floor. Zero keeps only the records of retained revisions.
    pub ledger_tail: u64,
    /// Blobs that no revision references are deleted only when at least this many seconds old, so
    /// a blob stored a moment ago for a commit that has not happened yet is not taken away.
    pub orphan_grace_secs: u64,
}

/// Why a compaction cannot proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    /// The ledger holds this many unresolved intents: run recovery first.
    OpenIntents(usize),
    /// Another holder has the store: a coordinator, a commit, a recovery or a compaction.
    StoreInUse,
    /// A declared live workspace is based on a revision the compaction would remove.
    WorkspaceBase {
        /// The workspace, as the caller labelled it.
        label: String,
        /// Log sequence of its base revision.
        sequence: u64,
    },
}

/// One revision a compaction removes from the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// Log sequence.
    pub sequence: u64,
    /// Revision id.
    pub revision: Uuid7,
    /// The operation that created it, when its manifest could be read.
    pub op: Option<Uuid7>,
    /// Manifest digest named by its record.
    pub manifest: ContentDigest,
    /// False when its manifest was already unreadable before the compaction.
    pub readable: bool,
}

/// What happens to the outcome ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerPlan {
    /// Size of the ledger now.
    pub bytes_before: u64,
    /// Size after rotation.
    pub bytes_after: u64,
    /// True when the file is rewritten.
    pub rotated: bool,
    /// Accepted operation ids whose record is dropped: a retry is no longer recognised.
    pub forgotten_ops: Vec<Uuid7>,
    /// Accepted operations of removed revisions whose record stays as a tombstone.
    pub tombstones: u64,
}

/// A workspace pin and where it lands in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinStatus {
    /// The workspace label.
    pub label: String,
    /// Log sequence of its base revision, `None` when the store no longer holds it.
    pub sequence: Option<u64>,
}

/// Everything a compaction would do, computed read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Head sequence (never changes).
    pub head_sequence: u64,
    /// Head revision id (never changes).
    pub head_revision: Uuid7,
    /// Lowest readable sequence now.
    pub old_floor: u64,
    /// Lowest readable sequence after the compaction.
    pub new_floor: u64,
    /// Revisions that stay.
    pub retained: u64,
    /// Revisions that are removed from the history (the undo points that are lost).
    pub removed: Vec<Removed>,
    /// Head-log record files that will be deleted (removed revisions and earlier leftovers).
    pub records_to_remove: u64,
    /// Blob files in the store.
    pub blob_count: u64,
    /// Bytes of those files.
    pub blob_bytes: u64,
    /// Blob files that will be deleted.
    pub blobs_to_delete: u64,
    /// Bytes those files hold.
    pub bytes_to_free: u64,
    /// Blobs no revision references that are younger than the grace period and stay for now.
    pub protected_orphans: u64,
    /// The ledger rotation.
    pub ledger: LedgerPlan,
    /// Where every declared workspace lands.
    pub pins: Vec<PinStatus>,
    /// Reasons a real run would be refused; empty when it can proceed.
    pub blockers: Vec<Blocker>,
    delete: Vec<(ContentDigest, u64)>,
    floor_record: Option<RootRecord>,
    rotated: Option<Vec<u8>>,
}

/// The result of a real compaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compacted {
    /// What was planned, and therefore done.
    pub plan: Plan,
    /// Head-log record files removed by this run.
    pub records_removed: u64,
    /// Blob files removed by this run.
    pub blobs_removed: u64,
    /// Bytes those blob files held.
    pub bytes_freed: u64,
}

/// A declared point of a compaction, in order. A [`CompactionHook`] sees each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionPoint {
    /// The exclusive lease is held and the plan verified. Nothing is changed.
    Planned,
    /// The safeguard ran (the backup is complete). Nothing of the store is changed.
    SafeguardDone,
    /// The floor marker is staged under a temporary name. The store is still the old one.
    FloorStaged,
    /// The floor marker is installed: **the commit point**. The store is now the new one.
    FloorInstalled,
    /// One head-log record below the floor was removed (reported once per record).
    RecordRemoved,
    /// The replacement ledger is staged. The old ledger is still in place.
    LedgerStaged,
    /// The replacement ledger is installed.
    LedgerReplaced,
    /// One blob was removed (reported once per blob).
    BlobRemoved,
    /// Every step is done; the head was reopened and verified.
    Finished,
}

/// A fault a [`CompactionHook`] injects. It carries no detail on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Injected;

/// Observer and fault-injection seam. Returning `Err(Injected)` stops the compaction at once with
/// [`CompactionError::Interrupted`], leaving the store exactly as a crash at that point would.
pub trait CompactionHook {
    /// Called when the compaction reaches `point`.
    fn reached(&mut self, point: CompactionPoint) -> Result<(), Injected>;
}

/// The hook that never fails.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFault;

impl CompactionHook for NoFault {
    fn reached(&mut self, _point: CompactionPoint) -> Result<(), Injected> {
        Ok(())
    }
}

impl<F: FnMut(CompactionPoint) -> Result<(), Injected>> CompactionHook for F {
    fn reached(&mut self, point: CompactionPoint) -> Result<(), Injected> {
        self(point)
    }
}

/// What protects the store against a compaction that turns out to be a mistake.
pub enum Safeguard<'a> {
    /// A backup made by the caller while the exclusive lease is held; a failure refuses the
    /// compaction before anything is changed.
    Backup(&'a mut dyn FnMut() -> Result<(), CompactionError>),
    /// The caller explicitly decided to go without a backup.
    NoBackup,
}

/// Typed failure of statistics, planning or compaction. No variant carries a path or raw OS text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionError {
    /// The request is malformed (a `keep` count of zero or above [`MAX_KEEP`], a ledger tail
    /// above [`MAX_LEDGER_TAIL`]).
    InvalidRequest(&'static str),
    /// A real run was requested without explicit confirmation.
    NotConfirmed,
    /// The compaction cannot proceed; nothing was changed.
    Blocked(Vec<Blocker>),
    /// The safeguard failed; nothing was changed.
    SafeguardFailed(ErrorCode),
    /// A hook stopped the run: the store is as a crash at this point leaves it.
    Interrupted(CompactionPoint),
    /// The revision log is unreadable, or a retained revision is damaged or incomplete.
    Revision(RevisionError),
    /// The outcome ledger is unreadable.
    Ledger(LedgerError),
    /// Storage failed, or the store is too large for one run (blob or file caps).
    Storage(StorageError),
}

impl CompactionError {
    /// Maps the failure onto the F04 section 8 vocabulary.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidRequest(_) | Self::NotConfirmed => ErrorCode::InvalidArgument,
            Self::Blocked(blockers) => {
                if blockers.contains(&Blocker::StoreInUse) {
                    ErrorCode::BackendUnavailable
                } else {
                    ErrorCode::DependencyConflict
                }
            }
            Self::SafeguardFailed(code) => *code,
            Self::Interrupted(_) => ErrorCode::Cancelled,
            Self::Revision(error) => error.code(),
            Self::Ledger(error) => error.code(),
            Self::Storage(error) => error.code(),
        }
    }

    /// A fixed ASCII explanation.
    pub fn message(&self) -> &'static str {
        match self {
            Self::InvalidRequest(message) => message,
            Self::NotConfirmed => "a real compaction needs explicit confirmation",
            Self::Blocked(blockers) => match blockers.first() {
                Some(Blocker::StoreInUse) => {
                    "the store is in use by a coordinator, a commit or another maintenance run"
                }
                Some(Blocker::OpenIntents(_)) => {
                    "an unresolved commit intent exists: run recovery before compacting"
                }
                Some(Blocker::WorkspaceBase { .. }) => {
                    "a live workspace is based on a revision that would be removed"
                }
                None => "the compaction is blocked",
            },
            Self::SafeguardFailed(_) => "the backup could not be completed; nothing was changed",
            Self::Interrupted(_) => "the compaction was interrupted",
            Self::Revision(_) => "the revision history is not intact",
            Self::Ledger(_) => "the outcome ledger is not usable",
            Self::Storage(_) => "store access failed or the store is too large",
        }
    }
}

impl fmt::Display for CompactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CompactionError {}

impl From<RevisionError> for CompactionError {
    fn from(error: RevisionError) -> Self {
        Self::Revision(error)
    }
}

impl From<LedgerError> for CompactionError {
    fn from(error: LedgerError) -> Self {
        Self::Ledger(error)
    }
}

impl From<StorageError> for CompactionError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<LeaseError> for CompactionError {
    fn from(error: LeaseError) -> Self {
        match error {
            LeaseError::Busy => Self::Blocked(vec![Blocker::StoreInUse]),
            LeaseError::Storage(error) => Self::Storage(error),
        }
    }
}

/// One revision as the walk found it.
struct Item {
    sequence: u64,
    record: RootRecord,
    op: Option<Uuid7>,
    blobs: Vec<(ContentDigest, u64)>,
    error: Option<RevisionError>,
}

impl Item {
    fn references(&self) -> impl Iterator<Item = ContentDigest> + '_ {
        std::iter::once(self.record.manifest()).chain(self.blobs.iter().map(|(digest, _)| *digest))
    }
}

/// The whole readable log, oldest first.
struct Walk {
    directory: PathBuf,
    span: Span,
    items: Vec<Item>,
}

fn walk(store: &RevisionStore) -> Result<Walk, CompactionError> {
    let directory = store.head_dir()?.ok_or(RevisionError::NoRevision)?;
    let span = store.span(&directory)?.ok_or(RevisionError::NoRevision)?;
    let mut items = Vec::new();
    for sequence in span.floor..=span.head {
        let record = store.load_record(&directory, sequence)?;
        let item = match store.load_at(&directory, sequence) {
            Ok(info) => Item {
                sequence,
                op: Some(info.manifest().content().op().clone()),
                blobs: info
                    .manifest()
                    .content()
                    .entries()
                    .iter()
                    .map(|entry| (entry.digest(), entry.size()))
                    .chain(
                        info.manifest()
                            .content()
                            .operation_evidence()
                            .map(|pin| (pin.digest(), pin.size())),
                    )
                    .collect(),
                record,
                error: None,
            },
            Err(error) => Item {
                sequence,
                op: None,
                blobs: Vec::new(),
                record,
                error: Some(error),
            },
        };
        items.push(item);
    }
    match items.last().and_then(|item| item.error) {
        Some(error) => Err(error.into()),
        None => Ok(Walk {
            directory,
            span,
            items,
        }),
    }
}

/// Verifies every layer blob of the given revisions once: present, digest, declared size.
fn verify_blobs(store: &RevisionStore, items: &[&Item]) -> Result<(), RevisionError> {
    let mut verified = BTreeMap::new();
    for item in items {
        for (entry, (digest, size)) in item.blobs.iter().enumerate() {
            if let Some(previous_size) = verified.get(digest) {
                if previous_size != size {
                    return Err(RevisionError::SizeMismatch { entry });
                }
                continue;
            }
            let bytes = store.blobs().read(digest).map_err(|error| match error {
                StorageError::NotFound => RevisionError::MissingBlob { entry },
                StorageError::DigestMismatch
                | StorageError::UnsafeEntry
                | StorageError::IntegrityConflict => RevisionError::CorruptBlob { entry },
                other => RevisionError::Storage(other),
            })?;
            if u64::try_from(bytes.len()) != Ok(*size) {
                return Err(RevisionError::SizeMismatch { entry });
            }
            verified.insert(*digest, *size);
        }
    }
    Ok(())
}

fn list_blobs(store: &RevisionStore) -> Result<Vec<BlobInfo>, CompactionError> {
    Ok(store.blobs().list_blobs(MAX_BLOBS)?)
}

fn resolve_pins(items: &[Item], pins: &[Pin]) -> Vec<PinStatus> {
    pins.iter()
        .map(|pin| PinStatus {
            label: pin.label.clone(),
            sequence: items
                .iter()
                .find(|item| item.record.revision() == &pin.revision)
                .map(|item| item.sequence),
        })
        .collect()
}

/// The result of rotating a ledger.
struct Rotation {
    bytes: Vec<u8>,
    forgotten: Vec<Uuid7>,
    tombstones: u64,
}

/// Rewrites the ledger without the records of operations accepted below `cutoff` (and of
/// attempts that targeted a sequence below it). Complete lines that are not valid records are
/// kept verbatim, so what the reader counts as damage is unchanged; the torn tail is kept as one
/// of them. A single genesis line records that the ledger no longer covers the early history.
fn rotate(raw: &[u8], cutoff: u64, floor: u64) -> Rotation {
    let mut lines: Vec<(&[u8], Option<Record>)> = Vec::new();
    let mut pieces = raw.split(|byte| *byte == b'\n').peekable();
    while let Some(piece) = pieces.next() {
        let torn = pieces.peek().is_none();
        if piece.is_empty() {
            continue;
        }
        let parsed = if torn {
            None
        } else {
            std::str::from_utf8(piece)
                .ok()
                .and_then(Record::parse)
                .map(|(record, _)| record)
        };
        lines.push((piece, parsed));
    }
    let mut committed: BTreeMap<Uuid7, u64> = BTreeMap::new();
    let mut attempted: BTreeMap<Uuid7, u64> = BTreeMap::new();
    let mut genesis: Option<u64> = None;
    for (_, record) in &lines {
        match record {
            Some(Record::Genesis { head }) => {
                genesis.get_or_insert(*head);
            }
            Some(Record::Committed(done)) => {
                committed.entry(done.op.clone()).or_insert(done.sequence);
            }
            Some(Record::Intent(intent)) => {
                let seen = attempted
                    .entry(intent.op.clone())
                    .or_insert(intent.sequence);
                *seen = (*seen).max(intent.sequence);
            }
            Some(Record::Aborted { .. }) | None => {}
        }
    }
    let sequence_of = |op: &Uuid7| committed.get(op).or_else(|| attempted.get(op)).copied();
    let mut bytes = Vec::new();
    bytes.extend(
        Record::Genesis {
            head: genesis.unwrap_or(0).max(cutoff.saturating_sub(1)),
        }
        .encode()
        .0,
    );
    for (raw_line, record) in &lines {
        let keep = match record {
            None => true,
            Some(Record::Genesis { .. }) => false,
            Some(Record::Intent(intent)) => sequence_of(&intent.op).is_none_or(|s| s >= cutoff),
            Some(Record::Committed(done)) => sequence_of(&done.op).is_none_or(|s| s >= cutoff),
            Some(Record::Aborted { op, .. }) => sequence_of(op).is_none_or(|s| s >= cutoff),
        };
        if keep {
            bytes.extend_from_slice(raw_line);
            bytes.push(b'\n');
        }
    }
    let forgotten: BTreeSet<Uuid7> = committed
        .iter()
        .filter(|(_, sequence)| **sequence < cutoff)
        .map(|(op, _)| op.clone())
        .collect();
    let tombstones = committed
        .values()
        .filter(|sequence| (cutoff..floor).contains(*sequence))
        .count();
    Rotation {
        bytes,
        forgotten: forgotten.into_iter().collect(),
        tombstones: u64::try_from(tombstones).unwrap_or(u64::MAX),
    }
}

/// Computes the plan; `check_lease` also reports a store in use (false when the caller holds the
/// exclusive lease itself).
fn plan_with(
    store: &RevisionStore,
    request: &Request<'_>,
    check_lease: bool,
) -> Result<Plan, CompactionError> {
    let keep = match request.keep {
        Keep::Head => 1,
        Keep::Last(0) => {
            return Err(CompactionError::InvalidRequest(
                "the number of revisions to keep is at least 1",
            ));
        }
        Keep::Last(n) if n > MAX_KEEP => {
            return Err(CompactionError::InvalidRequest(
                "the number of revisions to keep is above the log limit",
            ));
        }
        Keep::Last(n) => n,
    };
    if request.ledger_tail > MAX_LEDGER_TAIL {
        return Err(CompactionError::InvalidRequest(
            "the ledger tail is above the log limit",
        ));
    }
    let walked = walk(store)?;
    let Span {
        floor: old_floor,
        head,
    } = walked.span;
    let new_floor = head.saturating_sub(keep - 1).max(old_floor);
    let head_item = walked
        .items
        .last()
        .ok_or(CompactionError::Revision(RevisionError::NoRevision))?;

    let (kept, dropped): (Vec<&Item>, Vec<&Item>) = walked
        .items
        .iter()
        .partition(|item| item.sequence >= new_floor);
    if let Some(error) = kept.iter().find_map(|item| item.error) {
        return Err(error.into());
    }
    verify_blobs(store, &kept)?;

    let retained: BTreeSet<ContentDigest> =
        kept.iter().flat_map(|item| item.references()).collect();
    let referenced: BTreeSet<ContentDigest> = walked
        .items
        .iter()
        .flat_map(|item| item.references())
        .collect();
    let listed = list_blobs(store)?;
    let blob_bytes = listed.iter().map(|blob| blob.size).sum::<u64>();
    let mut delete = Vec::new();
    let (mut bytes_to_free, mut protected) = (0u64, 0u64);
    for blob in &listed {
        if retained.contains(&blob.digest) {
            continue;
        }
        if !referenced.contains(&blob.digest) && blob.age_secs < request.orphan_grace_secs {
            protected = protected.saturating_add(1);
            continue;
        }
        bytes_to_free = bytes_to_free.saturating_add(blob.size);
        delete.push((blob.digest, blob.size));
    }

    let pins = resolve_pins(&walked.items, request.pins);
    let mut blockers = Vec::new();
    let intents = store.ledger().scan()?.open_intents().count();
    if intents > 0 {
        blockers.push(Blocker::OpenIntents(intents));
    }
    for pin in &pins {
        if let Some(sequence) = pin.sequence
            && sequence < new_floor
        {
            blockers.push(Blocker::WorkspaceBase {
                label: pin.label.clone(),
                sequence,
            });
        }
    }
    if check_lease && !StoreLease::is_free(store.blobs()).map_err(CompactionError::from)? {
        blockers.push(Blocker::StoreInUse);
    }

    let cutoff = new_floor.saturating_sub(request.ledger_tail).max(1);
    let raw = store.ledger().read_raw()?;
    let bytes_before = raw
        .as_ref()
        .map_or(0, |raw| u64::try_from(raw.len()).unwrap_or(u64::MAX));
    let mut ledger = LedgerPlan {
        bytes_before,
        bytes_after: bytes_before,
        rotated: false,
        forgotten_ops: Vec::new(),
        tombstones: 0,
    };
    let mut rotated = None;
    if let Some(raw) = raw.as_ref().filter(|_| cutoff > 1) {
        let rotation = rotate(raw, cutoff, new_floor);
        ledger.forgotten_ops = rotation.forgotten;
        ledger.tombstones = rotation.tombstones;
        if rotation.bytes != *raw {
            ledger.bytes_after = u64::try_from(rotation.bytes.len()).unwrap_or(u64::MAX);
            ledger.rotated = true;
            rotated = Some(rotation.bytes);
        }
    }

    let records_to_remove = store.records_below(&walked.directory, new_floor)?.len();
    let removed = dropped
        .iter()
        .map(|item| Removed {
            sequence: item.sequence,
            revision: item.record.revision().clone(),
            op: item.op.clone(),
            manifest: item.record.manifest(),
            readable: item.error.is_none(),
        })
        .collect();
    let floor_record = kept
        .iter()
        .find(|item| item.sequence == new_floor)
        .filter(|_| new_floor > old_floor)
        .map(|item| item.record.clone());
    Ok(Plan {
        head_sequence: head,
        head_revision: head_item.record.revision().clone(),
        old_floor,
        new_floor,
        retained: u64::try_from(kept.len()).unwrap_or(u64::MAX),
        removed,
        records_to_remove: u64::try_from(records_to_remove).unwrap_or(u64::MAX),
        blob_count: u64::try_from(listed.len()).unwrap_or(u64::MAX),
        blob_bytes,
        blobs_to_delete: u64::try_from(delete.len()).unwrap_or(u64::MAX),
        bytes_to_free,
        protected_orphans: protected,
        ledger,
        pins,
        blockers,
        delete,
        floor_record,
        rotated,
    })
}

/// Computes what a compaction would do, without changing anything (not even the lock file). The
/// plan lists the revisions that would be lost and the reasons a real run would be refused.
/// Retained revisions are fully verified; a damaged one is an error here too.
pub fn plan(store: &RevisionStore, request: &Request<'_>) -> Result<Plan, CompactionError> {
    plan_with(store, request, true)
}

/// Compacts the store as described in the [module](self) documentation.
///
/// `confirmed` must be true: the caller shows the plan, including the revisions that become
/// unreachable, and the operator agrees. `safeguard` is a backup or an explicit decision to go
/// without. Everything is checked under the exclusive lease before the first change; the plan
/// the run executes is computed under that lease, so it is the plan the lease protects.
pub fn compact(
    store: &RevisionStore,
    request: &Request<'_>,
    confirmed: bool,
    safeguard: Safeguard<'_>,
    hook: &mut dyn CompactionHook,
) -> Result<Compacted, CompactionError> {
    if !confirmed {
        return Err(CompactionError::NotConfirmed);
    }
    let _lease = StoreLease::exclusive(store.blobs())?;
    let plan = plan_with(store, request, false)?;
    if !plan.blockers.is_empty() {
        return Err(CompactionError::Blocked(plan.blockers));
    }
    point(hook, CompactionPoint::Planned)?;
    if let Safeguard::Backup(backup) = safeguard {
        backup()?;
    }
    point(hook, CompactionPoint::SafeguardDone)?;

    let directory = store.head_dir()?.ok_or(RevisionError::NoRevision)?;
    if let Some(record) = &plan.floor_record {
        let staged = store.stage_floor(&directory, record)?;
        point(hook, CompactionPoint::FloorStaged)?;
        store.install_floor(&directory, &staged)?;
        point(hook, CompactionPoint::FloorInstalled)?;
    }
    let mut records_removed = 0u64;
    for sequence in store.records_below(&directory, plan.new_floor)? {
        store.remove_record(&directory, sequence)?;
        records_removed = records_removed.saturating_add(1);
        point(hook, CompactionPoint::RecordRemoved)?;
    }
    if let Some(bytes) = &plan.rotated {
        let ledger = store.ledger();
        let staged = ledger.stage_replacement(bytes)?;
        point(hook, CompactionPoint::LedgerStaged)?;
        ledger.install_replacement(&staged)?;
        point(hook, CompactionPoint::LedgerReplaced)?;
    }
    let (mut blobs_removed, mut bytes_freed) = (0u64, 0u64);
    for (digest, size) in &plan.delete {
        if store.blobs().remove_blob(digest)? {
            blobs_removed = blobs_removed.saturating_add(1);
            bytes_freed = bytes_freed.saturating_add(*size);
        }
        point(hook, CompactionPoint::BlobRemoved)?;
    }
    // The compacted head must open with every blob verified; a failure here is reported, it
    // cannot be undone (the backup is the way back).
    store.open()?;
    point(hook, CompactionPoint::Finished)?;
    Ok(Compacted {
        plan,
        records_removed,
        blobs_removed,
        bytes_freed,
    })
}

fn point(hook: &mut dyn CompactionHook, at: CompactionPoint) -> Result<(), CompactionError> {
    hook.reached(at)
        .map_err(|_| CompactionError::Interrupted(at))
}

/// Measurements of one store. Everything is read-only; blob digests are not re-verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    /// Lowest readable sequence.
    pub first_sequence: u64,
    /// Head sequence.
    pub head_sequence: u64,
    /// Head revision id.
    pub head_revision: Uuid7,
    /// Revisions in the log (`head - first + 1`).
    pub revisions: u64,
    /// Revisions whose manifest cannot be read.
    pub unreadable_revisions: u64,
    /// Blob files in `blobs/`.
    pub blob_count: u64,
    /// Bytes of those files.
    pub blob_bytes: u64,
    /// Blob files the head revision references (its manifest included).
    pub head_blob_count: u64,
    /// Bytes of those files.
    pub head_bytes: u64,
    /// Blob files any revision of the log references.
    pub history_blob_count: u64,
    /// Bytes of those files.
    pub history_bytes: u64,
    /// Blob files the declared workspaces' base revisions reference; `None` without pins.
    pub workspace_blob_count: Option<u64>,
    /// Bytes of those files; `None` without pins.
    pub workspace_bytes: Option<u64>,
    /// Blob files no revision of the log references (reclaimable without losing history).
    pub unreachable_blob_count: u64,
    /// Bytes of those files.
    pub unreachable_bytes: u64,
    /// `unreachable_bytes` as hundredths of a percent of `blob_bytes` (10000 is all of it).
    pub unreachable_share_hundredths: u64,
    /// Blobs a revision references that are not on disk.
    pub missing_blobs: u64,
    /// Bytes of the head-log directory: records, floor marker and leftovers.
    pub revision_log_bytes: u64,
    /// Head-log record files below the floor still on disk (an interrupted compaction).
    pub leftover_records: u64,
    /// Whether the log was compacted (a floor marker exists).
    pub compacted: bool,
    /// Size of the outcome ledger.
    pub ledger_bytes: u64,
    /// Valid ledger records.
    pub ledger_records: u64,
    /// Unresolved intents in the ledger.
    pub open_intents: u64,
    /// Where every declared workspace lands.
    pub pins: Vec<PinStatus>,
}

/// Measures the store: revisions, blob bytes in total, reachable from the head, from the history
/// and from the declared workspaces, the share and count of unreachable blobs, and the sizes of
/// the revision log and the ledger. Changes nothing.
pub fn stats(store: &RevisionStore, pins: &[Pin]) -> Result<Stats, CompactionError> {
    let walked = walk(store)?;
    let listed = list_blobs(store)?;
    let sizes: BTreeMap<ContentDigest, u64> =
        listed.iter().map(|blob| (blob.digest, blob.size)).collect();
    let measure = |set: &BTreeSet<ContentDigest>| -> (u64, u64) {
        let present = set.iter().filter_map(|digest| sizes.get(digest));
        (
            u64::try_from(present.clone().count()).unwrap_or(u64::MAX),
            present.copied().sum(),
        )
    };
    let head_item = walked
        .items
        .last()
        .ok_or(CompactionError::Revision(RevisionError::NoRevision))?;
    let head_set: BTreeSet<ContentDigest> = head_item.references().collect();
    let history: BTreeSet<ContentDigest> = walked
        .items
        .iter()
        .flat_map(|item| item.references())
        .collect();
    let resolved = resolve_pins(&walked.items, pins);
    let workspace = (!pins.is_empty()).then(|| {
        let bases: BTreeSet<u64> = resolved.iter().filter_map(|pin| pin.sequence).collect();
        let set: BTreeSet<ContentDigest> = walked
            .items
            .iter()
            .filter(|item| bases.contains(&item.sequence))
            .flat_map(|item| item.references())
            .collect();
        measure(&set)
    });
    let (head_blob_count, head_bytes) = measure(&head_set);
    let (history_blob_count, history_bytes) = measure(&history);
    let blob_bytes = listed.iter().map(|blob| blob.size).sum::<u64>();
    let unreachable: Vec<&BlobInfo> = listed
        .iter()
        .filter(|blob| !history.contains(&blob.digest))
        .collect();
    let unreachable_bytes = unreachable.iter().map(|blob| blob.size).sum::<u64>();
    let share = if blob_bytes == 0 {
        0
    } else {
        u64::try_from(u128::from(unreachable_bytes) * 10_000 / u128::from(blob_bytes))
            .unwrap_or(u64::MAX)
    };
    let missing = history
        .iter()
        .filter(|digest| !sizes.contains_key(digest))
        .count();
    let files = store
        .blobs()
        .list_namespace_files(&walked.directory, MAX_LOG_FILES)?;
    let scan = store.ledger().scan()?;
    let diagnostics = scan.diagnostics();
    let revisions = walked
        .span
        .head
        .saturating_sub(walked.span.floor)
        .saturating_add(1);
    Ok(Stats {
        first_sequence: walked.span.floor,
        head_sequence: walked.span.head,
        head_revision: head_item.record.revision().clone(),
        revisions,
        unreadable_revisions: u64::try_from(
            walked
                .items
                .iter()
                .filter(|item| item.error.is_some())
                .count(),
        )
        .unwrap_or(u64::MAX),
        blob_count: u64::try_from(listed.len()).unwrap_or(u64::MAX),
        blob_bytes,
        head_blob_count,
        head_bytes,
        history_blob_count,
        history_bytes,
        workspace_blob_count: workspace.map(|(count, _)| count),
        workspace_bytes: workspace.map(|(_, bytes)| bytes),
        unreachable_blob_count: u64::try_from(unreachable.len()).unwrap_or(u64::MAX),
        unreachable_bytes,
        unreachable_share_hundredths: share,
        missing_blobs: u64::try_from(missing).unwrap_or(u64::MAX),
        revision_log_bytes: files.iter().map(|file| file.size).sum(),
        leftover_records: u64::try_from(
            store
                .records_below(&walked.directory, walked.span.floor)?
                .len(),
        )
        .unwrap_or(u64::MAX),
        compacted: walked.span.floor > 1,
        ledger_bytes: diagnostics.bytes,
        ledger_records: u64::try_from(diagnostics.records).unwrap_or(u64::MAX),
        open_intents: u64::try_from(scan.open_intents().count()).unwrap_or(u64::MAX),
        pins: resolved,
    })
}
