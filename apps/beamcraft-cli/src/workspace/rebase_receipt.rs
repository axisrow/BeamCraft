// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C03.8: bounded, canonical description of a rebase request and its retained dependencies.
//!
//! Decoding checks structure and the footprint's request/operation binding. It grants no
//! permissions and proves no ancestry or materialization: the adapter must verify the actual
//! source interval, accepted history and complete candidate before using a receipt for recovery.

use super::WorkspaceMeta;
use crate::command_evidence::CapturedFootprint;
use beamcraft_engine::footprint::Footprint;
use beamcraft_host::revision::{MAX_OPERATION_EVIDENCE_BYTES, MAX_REVISIONS, RevisionInfo, Uuid7};
use beamcraft_model::changeset::{MAX_OPERATIONS, Operation};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::OpId;
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use std::collections::BTreeSet;
use std::fmt::Write as _;

const MAGIC: &str = "beamcraft-workspace-rebase 1\n";
const REQUEST_MAGIC: &str = "beamcraft-workspace-rebase-request 1";

/// One complete immutable store coordinate, independent of another store's local sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionPin {
    /// Full revision identity.
    pub revision: Uuid7,
    /// Full manifest digest.
    pub manifest: ContentDigest,
    /// One-based sequence in the store this pin belongs to.
    pub sequence: u64,
}

impl RevisionPin {
    /// Pins the exact verified record supplied by the host; this does not verify another store.
    pub fn from_info(info: &RevisionInfo) -> Self {
        Self {
            revision: info.revision().clone(),
            manifest: info.manifest_digest(),
            sequence: info.sequence(),
        }
    }
}

/// Original metadata and three immutable input pins of one rebase, plus every unsent operation.
/// The candidate revision identity is the rebase operation identity. Unsent commands are not
/// marked published; an administrative rebase is not itself a second modelling command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseReceipt {
    /// Caller-supplied, reusable rebase identity.
    pub op_id: OpId,
    /// Project shared by the independently verified inputs.
    pub project: Uuid7,
    /// Schema shared by those inputs.
    pub schema: ContentDigest,
    /// Complete metadata snapshot that the committed result is allowed to advance.
    pub old_meta: WorkspaceMeta,
    /// Sequence of old_meta's accepted base in the published store (not the draft prefix).
    pub old_base_sequence: u64,
    /// Original draft head against which the candidate CAS is attempted.
    pub draft: RevisionPin,
    /// New accepted basis, pinned in the published store.
    pub accepted: RevisionPin,
    /// Complete ordered unsent interval, including earlier administrative receipts if any.
    pub operations: Vec<Operation>,
    /// Actual union of the retained modelling dependencies, recomputed by the adapter.
    pub footprint: Footprint,
}

fn invalid() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidArgument,
        "rebase_receipt",
        "rebase receipt is not canonical or internally consistent",
    )
}
fn limit() -> CommandError {
    CommandError::new(
        ErrorCode::LimitExceeded,
        "rebase_receipt",
        "rebase receipt budget exceeded",
    )
}
fn decimal(text: &str) -> Result<u64, CommandError> {
    let value = text.parse::<u64>().map_err(|_| invalid())?;
    if value.to_string() != text {
        return Err(invalid());
    }
    Ok(value)
}
fn field<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    key: &str,
) -> Result<&'a str, CommandError> {
    lines
        .next()
        .and_then(|line| line.strip_prefix(key))
        .and_then(|tail| tail.strip_prefix(' '))
        .ok_or_else(invalid)
}
fn uuid(text: &str) -> Result<Uuid7, CommandError> {
    Uuid7::parse(text).map_err(|_| invalid())
}
fn pin(text: &str) -> Result<RevisionPin, CommandError> {
    let mut words = text.split(' ');
    let (Some(revision), Some(manifest), Some(sequence), None) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return Err(invalid());
    };
    Ok(RevisionPin {
        revision: uuid(revision)?,
        manifest: manifest.parse().map_err(|_| invalid())?,
        sequence: decimal(sequence)?,
    })
}
fn block<'a>(text: &'a str, key: &str) -> Result<(&'a str, &'a str), CommandError> {
    let (line, remaining) = text.split_once('\n').ok_or_else(invalid)?;
    let size = decimal(
        line.strip_prefix(key)
            .and_then(|tail| tail.strip_prefix(' '))
            .ok_or_else(invalid)?,
    )?;
    if size > MAX_OPERATION_EVIDENCE_BYTES {
        return Err(limit());
    }
    remaining
        .split_at_checked(usize::try_from(size).map_err(|_| limit())?)
        .ok_or_else(invalid)
}

impl RebaseReceipt {
    fn validate(&self) -> Result<(), CommandError> {
        let count = self.operations.len();
        if count == 0 {
            return Err(invalid());
        }
        if count > MAX_OPERATIONS {
            return Err(limit());
        }
        let entries = self
            .footprint
            .reads()
            .len()
            .saturating_add(self.footprint.writes().len());
        if entries == 0 {
            return Err(invalid());
        }
        if entries > MAX_FOOTPRINT_ENTRIES
            || self.draft.sequence >= MAX_REVISIONS
            || self.accepted.sequence > MAX_REVISIONS
        {
            return Err(limit());
        }
        if self.old_meta.synced_sequence == 0
            || self.old_meta.rebase_sequence > self.draft.sequence
            || self
                .draft
                .sequence
                .checked_sub(self.old_meta.synced_sequence)
                != Some(count as u64)
            || self.old_base_sequence == 0
            || self.old_base_sequence >= self.accepted.sequence
            || self.accepted.revision == self.old_meta.base_revision
            || self.accepted.manifest == self.old_meta.base_manifest
            || [
                self.draft.revision.as_str(),
                self.accepted.revision.as_str(),
                self.old_meta.base_revision.as_str(),
            ]
            .contains(&self.op_id.to_string().as_str())
            || self
                .operations
                .last()
                .is_none_or(|op| op.result != self.draft.manifest)
        {
            return Err(invalid());
        }
        let mut ids = BTreeSet::new();
        let mut results = BTreeSet::new();
        for operation in &self.operations {
            if operation.op_id == self.op_id
                || !ids.insert(operation.op_id)
                || !results.insert(operation.result)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn request(&self) -> Result<Vec<u8>, CommandError> {
        self.validate()?;
        let mut out = format!(
            "{REQUEST_MAGIC}\nop {}\nproject {}\nschema {}\nworkspace {}\nuser {}\nbase {} {} {}\nprefix {}\nrebase_sequence {}\ndraft {} {} {}\naccepted {} {} {}\noperations {}\n",
            self.op_id,
            self.project,
            self.schema,
            self.old_meta.workspace_id,
            self.old_meta.user_id,
            self.old_meta.base_revision,
            self.old_meta.base_manifest,
            self.old_base_sequence,
            self.old_meta.synced_sequence,
            self.old_meta.rebase_sequence,
            self.draft.revision,
            self.draft.manifest,
            self.draft.sequence,
            self.accepted.revision,
            self.accepted.manifest,
            self.accepted.sequence,
            self.operations.len()
        );
        for op in &self.operations {
            // Writing to an owned String cannot fail.
            let _ = writeln!(out, "{} {}", op.op_id, op.result);
        }
        if out.len() as u64 > MAX_OPERATION_EVIDENCE_BYTES {
            return Err(limit());
        }
        Ok(out.into_bytes())
    }

    /// Fingerprint of the complete canonical request, including old metadata and all source pins.
    /// A decoder checks that the embedded footprint names this same request and operation.
    pub fn fingerprint(&self) -> Result<ContentDigest, CommandError> {
        Sha256::digest(&self.request()?).map_err(|_| limit())
    }

    /// Canonical opaque evidence bytes, capped by the host's one-MiB evidence budget.
    pub fn encode(&self) -> Result<Vec<u8>, CommandError> {
        let request = self.request()?;
        let captured = CapturedFootprint {
            op_id: self.op_id,
            fingerprint: Sha256::digest(&request).map_err(|_| limit())?,
            footprint: self.footprint.clone(),
        }
        .encode()?;
        let mut out = format!("{MAGIC}request {}\n", request.len()).into_bytes();
        out.extend_from_slice(&request);
        out.extend_from_slice(format!("footprint {}\n", captured.len()).as_bytes());
        out.extend_from_slice(&captured);
        if out.len() as u64 > MAX_OPERATION_EVIDENCE_BYTES {
            return Err(limit());
        }
        Ok(out)
    }

    /// Strict bounded parser. Exact lengths, canonical decimal values, closed fields and a
    /// byte-identical re-encoding are required. A parsed receipt alone is not provenance proof.
    pub fn decode(bytes: &[u8]) -> Result<Self, CommandError> {
        if bytes.len() as u64 > MAX_OPERATION_EVIDENCE_BYTES {
            return Err(limit());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
        let text = text.strip_prefix(MAGIC).ok_or_else(|| {
            CommandError::new(
                ErrorCode::UnsupportedVersion,
                "rebase_receipt",
                "unsupported rebase receipt version",
            )
        })?;
        let (request, remaining) = block(text, "request")?;
        let (footprint, remaining) = block(remaining, "footprint")?;
        if !remaining.is_empty() {
            return Err(invalid());
        }
        let captured = CapturedFootprint::decode(footprint.as_bytes())?;
        let mut lines = request.strip_suffix('\n').ok_or_else(invalid)?.split('\n');
        if lines.next() != Some(REQUEST_MAGIC) {
            return Err(invalid());
        }
        let op_id = OpId::parse(field(&mut lines, "op")?).map_err(|_| invalid())?;
        let project = uuid(field(&mut lines, "project")?)?;
        let schema = field(&mut lines, "schema")?
            .parse()
            .map_err(|_| invalid())?;
        let workspace_id = uuid(field(&mut lines, "workspace")?)?;
        let user_id = uuid(field(&mut lines, "user")?)?;
        let base = pin(field(&mut lines, "base")?)?;
        let synced_sequence = decimal(field(&mut lines, "prefix")?)?;
        let rebase_sequence = decimal(field(&mut lines, "rebase_sequence")?)?;
        let draft = pin(field(&mut lines, "draft")?)?;
        let accepted = pin(field(&mut lines, "accepted")?)?;
        let count = decimal(field(&mut lines, "operations")?)?;
        if count == 0 {
            return Err(invalid());
        }
        if count > MAX_OPERATIONS as u64 {
            return Err(limit());
        }
        let mut operations = Vec::with_capacity(usize::try_from(count).map_err(|_| limit())?);
        for _ in 0..count {
            let (op, result) = lines
                .next()
                .and_then(|line| line.split_once(' '))
                .ok_or_else(invalid)?;
            operations.push(Operation {
                op_id: OpId::parse(op).map_err(|_| invalid())?,
                result: result.parse().map_err(|_| invalid())?,
            });
        }
        if lines.next().is_some() {
            return Err(invalid());
        }
        let receipt = Self {
            op_id,
            project,
            schema,
            old_meta: WorkspaceMeta {
                workspace_id,
                user_id,
                base_revision: base.revision,
                base_manifest: base.manifest,
                synced_sequence,
                rebase_sequence,
            },
            old_base_sequence: base.sequence,
            draft,
            accepted,
            operations,
            footprint: captured.footprint,
        };
        if captured.op_id != receipt.op_id
            || captured.fingerprint != receipt.fingerprint()?
            || receipt.encode()? != bytes
        {
            return Err(invalid());
        }
        Ok(receipt)
    }
}
