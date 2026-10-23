// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U06: commit and read of one complete project revision (F04 sections 2, 6 and 8).
//!
//! A revision is an immutable manifest blob (see [`Manifest`]) that names every layer blob of the
//! revision by full SHA-256 and size. The published root is an append-only head log in the
//! store's `revisions/` directory: record `N` is a complete file that appears with a single
//! non-replacing hard link of an already synced temporary file. The root is the record with the
//! highest sequence number, so it switches from revision `N-1` to `N` in one filesystem step and
//! a reader sees either the previous complete revision or the new one, never a partial set of
//! files. Because that link fails when the name exists, it is also the compare-and-swap that F04
//! section 8 requires ("plain read-then-rename is not CAS"): two committers on the same base
//! cannot both win, and there is no lock file that a crash could leave behind.
//!
//! # What is promised
//!
//! * [`RevisionStore::commit_with`] verifies, before the root switch, the expected base, the
//!   project, replay/identity reuse (the op id and base must match a recorded retry; the revision
//!   id is unique across the whole log), every referenced blob (presence, full digest, declared
//!   size) and, when a [`ContentVerifier`] is supplied, the complete U03 evidence for the draft's
//!   root and schema. Any failure
//!   before the switch returns an error with the previous root untouched. Manifest and layer
//!   blobs published before a failed attempt remain as unreferenced immutable blobs.
//! * [`RevisionStore::open`] verifies the record, the manifest, the base link and every
//!   referenced blob again, and refuses missing (for example garbage-collected), corrupt,
//!   resized or torn state with a typed [`RevisionError`].
//! * After a U14 compaction the log starts at a floor (see [`crate::compaction`]): the records
//!   `floor..=head` are the log, the floor's manifest names a base that was removed, and
//!   `revisions/floor` anchors it. Without that marker the floor is 1 and nothing differs.
//! * Manifests are canonical: the same draft on the same base is byte-identical whatever order
//!   its layers were supplied in.
//!
//! # What is not promised
//!
//! * No directory-entry crash durability: the temporary record is flushed and synced, but the
//!   directory is not synced (portable safe `std` cannot do that on every platform), so after
//!   power loss a just-reported commit may be absent. Absence is still a complete previous root.
//! * A crash can leave a `.tmp-*` file in `revisions/` and unreferenced blobs; readers ignore
//!   them. U07 recovery ([`crate::ledger`]) removes abandoned temporaries and reports unreferenced
//!   blobs. A hook failure after the link is [`RevisionError::OutcomeUnknown`], because the new
//!   root may be visible.
//! * There is no personal workspace or multi-client coordination in this module. Every
//!   commit writes an intent and an outcome to the U07 outcome ledger ([`crate::ledger`]); the
//!   [`CommitHook`] points are where it injects faults. Garbage collection and history
//!   compaction are U14 ([`crate::compaction`]); they move the log's floor, see below.
//! * The same namespace protection as [`crate::storage`] is required: a hostile writer to the
//!   store directory can still replace files between a check and its use.
//! * This module never parses USD. Layer content and U03 index checks are done by an injected
//!   [`ContentVerifier`] (the host does not link the layer engine; the caller supplies the
//!   verifier); a commit without a verifier records no index and says so in the manifest.

mod commit;
mod floor;
mod manifest;
mod read;
mod root;

use std::fmt;
use std::io;

use beamcraft_model::ErrorCode;

use crate::lease::LeaseError;
use crate::ledger::LedgerError;
use crate::storage::StorageError;

pub use commit::{
    CommitHook, CommitOutcome, CommitPoint, Committed, ContentRejection, ContentVerifier,
    ExpectedBase, IndexEvidence, Injected, NoFault, VerifiedLayer, VerifyRequest,
};
pub(crate) use floor::Span;
pub use manifest::{
    BaseRef, Content, EvidencePin, IndexRecord, LayerEntry, LayerRole, Manifest, RevisionDraft,
};
pub use read::{Revision, RevisionStore};
pub use root::{RevisionInfo, RootRecord};

/// Most layer entries one manifest may name.
pub const MAX_ENTRIES: usize = 64;
/// Longest layer name in bytes.
pub const MAX_NAME_BYTES: usize = 128;
/// Largest encoded manifest.
pub const MAX_MANIFEST_BYTES: usize = 65_536;
/// Maximum bytes of opaque operation evidence pinned by one revision.
pub const MAX_OPERATION_EVIDENCE_BYTES: u64 = 1_048_576;
/// Largest revision sequence number (and head-log length) this store accepts. Every commit reads
/// all records to prove revision-id uniqueness, so the cap bounds that linear cost.
pub const MAX_REVISIONS: u64 = 100_000;
/// Largest sum of the declared sizes of one revision's layers.
pub const MAX_REVISION_BYTES: u64 = 268_435_456;
/// Largest sum of layer bytes held in memory for a [`ContentVerifier`].
pub const MAX_CONTENT_VERIFY_BYTES: u64 = 67_108_864;
/// Most head-log records inspected for an op-id replay. Revision-id uniqueness is not bounded by
/// this window: it is checked against every record.
pub const REPLAY_LOOKBACK: usize = 64;

/// A full UUIDv7 in canonical lowercase `8-4-4-4-12` form (F04 section 2).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid7(String);

impl Uuid7 {
    /// Validates the complete canonical text; shortened, upper-case or other versions are
    /// [`RevisionError::InvalidId`].
    pub fn parse(text: &str) -> Result<Self, RevisionError> {
        if canonical_uuid7(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(RevisionError::InvalidId)
        }
    }

    /// The canonical text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Canonical lowercase UUIDv7 text: `8-4-4-4-12` hex digits, version 7, RFC 9562 variant.
fn canonical_uuid7(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else if i == 14 {
                b == b'7'
            } else if i == 19 {
                matches!(b, b'8' | b'9' | b'a' | b'b')
            } else {
                b.is_ascii_digit() || matches!(b, b'a'..=b'f')
            }
        })
}

impl fmt::Display for Uuid7 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Typed failure of a revision commit or read. Variants carry no paths or raw OS text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionError {
    /// A draft field is malformed (name, role set, root layer, size, empty entry list).
    InvalidArgument,
    /// An identity is not a full canonical UUIDv7.
    InvalidId,
    /// Two layer entries carry the same name.
    DuplicateEntry,
    /// A count, size, sequence or length cap was exceeded.
    LimitExceeded,
    /// The draft names a different project than the store's root.
    ProjectMismatch,
    /// The expected base is not the current root: stale writer or lost race.
    BaseMismatch,
    /// The op id was already committed with different content or another expected base.
    OpIdReuse,
    /// The revision id was already used by a different operation or content.
    RevisionIdReuse,
    /// No revision has been published.
    NoRevision,
    /// The revision was removed by history compaction (U14): its sequence is below the floor.
    Compacted,
    /// The head log is torn: an unknown name, a gap, an unreadable or malformed record.
    TornRoot,
    /// The manifest blob is absent (for example garbage-collected).
    ManifestMissing,
    /// The manifest blob is corrupt, non-canonical or violates a manifest rule.
    MalformedManifest,
    /// The record, manifest and base chain disagree.
    RootMismatch,
    /// A referenced layer blob is absent; `entry` is its manifest position.
    MissingBlob {
        /// Position in the sorted manifest entry list.
        entry: usize,
    },
    /// A referenced layer blob fails digest or entry verification.
    CorruptBlob {
        /// Position in the sorted manifest entry list.
        entry: usize,
    },
    /// A referenced layer blob verifies but has a different size than the manifest declares.
    SizeMismatch {
        /// Position in the sorted manifest entry list.
        entry: usize,
    },
    /// The revision's pinned operation evidence is absent.
    MissingOperationEvidence,
    /// Pinned operation evidence has a different digest/size or is not a safe regular blob.
    CorruptOperationEvidence,
    /// The content verifier rejected the layers (duplicate or short ID, dangling relationship,
    /// foreign override, invalid layer, ...) with its own stable code.
    Content(ContentRejection),
    /// The supplied U03 evidence covers only part of the snapshot.
    IndexIncomplete,
    /// The supplied U03 evidence is for another project, workspace, generation, base, composition
    /// root or schema.
    IndexMismatch,
    /// A [`CommitHook`] aborted before the root switch; the root is provably unchanged.
    Aborted(CommitPoint),
    /// A [`CommitHook`] failed after the root switch: the new root may be visible. Never
    /// reported as success or as rolled back; a retry with the same op id resolves it.
    OutcomeUnknown,
    /// Blob storage failed.
    Storage(StorageError),
    /// Another holder has the store: a history compaction is running (or a commit while this
    /// call needed the store exclusively). Nothing was written; retry later.
    StoreBusy,
    /// The outcome ledger failed or contradicts the root. See [`crate::ledger`].
    Ledger(LedgerError),
    /// Filesystem I/O failed; only its safe category is retained.
    Io(io::ErrorKind),
}

impl RevisionError {
    /// Maps the failure onto the F04 section 8 vocabulary.
    pub const fn code(self) -> ErrorCode {
        match self {
            Self::InvalidArgument | Self::ProjectMismatch => ErrorCode::InvalidArgument,
            Self::InvalidId => ErrorCode::InvalidId,
            Self::DuplicateEntry => ErrorCode::DuplicateId,
            Self::LimitExceeded => ErrorCode::LimitExceeded,
            Self::BaseMismatch => ErrorCode::BaseRevisionMismatch,
            Self::OpIdReuse => ErrorCode::OpIdReuse,
            Self::NoRevision | Self::Compacted => ErrorCode::NotFound,
            Self::RevisionIdReuse
            | Self::TornRoot
            | Self::ManifestMissing
            | Self::MalformedManifest
            | Self::RootMismatch
            | Self::MissingBlob { .. }
            | Self::CorruptBlob { .. }
            | Self::SizeMismatch { .. } => ErrorCode::IntegrityError,
            Self::MissingOperationEvidence | Self::CorruptOperationEvidence => {
                ErrorCode::IntegrityError
            }
            Self::Content(rejection) => rejection.code,
            Self::IndexIncomplete => ErrorCode::DependencyUnavailable,
            Self::IndexMismatch => ErrorCode::StaleWorkspace,
            Self::Aborted(_) => ErrorCode::Cancelled,
            Self::OutcomeUnknown => ErrorCode::OutcomeUnknown,
            Self::Storage(error) => error.code(),
            Self::StoreBusy => ErrorCode::BackendUnavailable,
            Self::Ledger(error) => error.code(),
            Self::Io(io::ErrorKind::PermissionDenied) => ErrorCode::PermissionDenied,
            Self::Io(_) => ErrorCode::BackendUnavailable,
        }
    }
}

impl fmt::Display for RevisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidArgument => "invalid revision draft",
            Self::InvalidId => "identity is not a full UUIDv7",
            Self::DuplicateEntry => "duplicate layer name",
            Self::LimitExceeded => "revision cap exceeded",
            Self::ProjectMismatch => "draft project differs from the store's project",
            Self::BaseMismatch => "expected base is not the current root",
            Self::OpIdReuse => "operation id already committed with different content",
            Self::RevisionIdReuse => "revision id already used by different content",
            Self::NoRevision => "no revision is published",
            Self::Compacted => "the revision was removed by history compaction",
            Self::TornRoot => "revision head log is torn",
            Self::ManifestMissing => "revision manifest blob is missing",
            Self::MalformedManifest => "revision manifest is corrupt or non-canonical",
            Self::RootMismatch => "revision record, manifest and base chain disagree",
            Self::MissingBlob { .. } => "a referenced layer blob is missing",
            Self::CorruptBlob { .. } => "a referenced layer blob is corrupt",
            Self::SizeMismatch { .. } => "a referenced layer blob has an unexpected size",
            Self::MissingOperationEvidence => "pinned operation evidence is missing",
            Self::CorruptOperationEvidence => "pinned operation evidence is corrupt",
            Self::Content(rejection) => rejection.message,
            Self::IndexIncomplete => "index evidence is partial",
            Self::IndexMismatch => "index evidence belongs to another context",
            Self::Aborted(_) => "commit aborted before the root switch",
            Self::OutcomeUnknown => "commit outcome unknown after the root switch",
            Self::Storage(_) => "blob storage failed",
            Self::StoreBusy => "the store is in use by a history compaction",
            Self::Ledger(error) => match error {
                LedgerError::RootDisagrees => "ledger records a commit that the root lacks",
                _ => "outcome ledger failed",
            },
            Self::Io(_) => "revision I/O failed",
        })
    }
}

impl std::error::Error for RevisionError {}

impl From<StorageError> for RevisionError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<LedgerError> for RevisionError {
    fn from(error: LedgerError) -> Self {
        Self::Ledger(error)
    }
}

impl From<LeaseError> for RevisionError {
    fn from(error: LeaseError) -> Self {
        match error {
            LeaseError::Busy => Self::StoreBusy,
            LeaseError::Storage(error) => Self::Storage(error),
        }
    }
}
