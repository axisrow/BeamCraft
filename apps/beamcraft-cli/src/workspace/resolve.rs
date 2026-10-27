// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C04.2: explicit resolution of a recorded semantic conflict by renouncing one's own work.
//!
//! Resolving discards one named own unsent operation and everything unsent after it: the
//! resolution commits a new draft revision whose layers are byte for byte the layers of the
//! state before the discarded operation, plus a canonical discard receipt as its operation
//! evidence. Later planning, packaging and publication never see the renounced work again,
//! while accepted history and every other actor's edit stay untouched. The receipt is a
//! recorded operation with its own identity: a retry in any process replays the recorded
//! outcome exactly once, and the same identity with another payload is refused.

use super::{WorkspaceMeta, WorkspaceState, rebase_receipt::RevisionPin, unusable};
use crate::{
    command_evidence as evidence,
    plate::{PlateError, RevisionSummary, host_uuid, integrity, summarize},
};
use beamcraft_host::{
    ledger::Outcome,
    revision::{
        CommitHook, EvidencePin, ExpectedBase, LayerEntry, MAX_OPERATION_EVIDENCE_BYTES,
        RevisionDraft, RevisionStore, Uuid7,
    },
};
use beamcraft_model::{
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
    id::OpId,
};
use std::path::Path;

const MAGIC: &str = "beamcraft-workspace-discard 1";

fn bad() -> PlateError {
    integrity("discard receipt is not usable")
}

fn reuse() -> PlateError {
    PlateError::new(
        ErrorCode::OpIdReuse,
        "resolve identity is bound to another request",
    )
}

/// Caller inputs. The CLI actor is trusted local context; a coordinator must derive it from
/// authentication. Reusing the identity with another payload is refused, including after a
/// failed attempt.
pub struct ResolveRequest {
    /// Reusable resolution operation identity.
    pub op_id: OpId,
    /// Trusted acting user, required to own the workspace.
    pub actor: Uuid7,
    /// The own unsent operation to renounce, named as recorded in the draft history.
    pub discarded: OpId,
    /// Exact observed draft head; the resolution is refused when it moved.
    pub expected_draft: Uuid7,
}

/// One completed resolution, or its original historical result on replay.
pub struct Resolved {
    /// True when this operation already committed; no new revision is written.
    pub replayed: bool,
    /// The resolution revision, which is the restored draft head.
    pub revision: RevisionSummary,
    /// The renounced operations in draft order, the named one first.
    pub renounced: Vec<OpId>,
}

/// Canonical evidence of one explicit resolution. Decoding checks structure and the context
/// binding; it proves no ancestry or restoration: the adapter verifies the source interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardReceipt {
    /// The reusable resolution identity; names the revision the receipt is pinned to.
    pub op_id: OpId,
    /// The oldest renounced own unsent operation.
    pub discarded: OpId,
    /// Draft sequence of the discarded operation.
    pub at: u64,
    /// The draft head the resolution was recorded against.
    pub head: RevisionPin,
    /// The workspace the resolution belongs to.
    pub workspace: Uuid7,
    /// The project of the renounced work.
    pub project: Uuid7,
    /// The schema of the renounced work.
    pub schema: ContentDigest,
}

impl DiscardReceipt {
    fn encode(&self) -> Result<Vec<u8>, PlateError> {
        if self.op_id == self.discarded || self.at == 0 {
            return Err(bad());
        }
        let text = format!(
            "{MAGIC}\nop {}\ndiscarded {}\nat {}\nhead {} {} {}\nworkspace {}\nproject {}\nschema {}\n",
            self.op_id,
            self.discarded,
            self.at,
            self.head.revision,
            self.head.manifest,
            self.head.sequence,
            self.workspace,
            self.project,
            self.schema,
        );
        if text.len() as u64 > MAX_OPERATION_EVIDENCE_BYTES {
            return Err(bad());
        }
        Ok(text.into_bytes())
    }

    /// Strict bounded parser: closed keys, full identities and a byte-identical canonical
    /// re-encoding. A parsed receipt alone is not provenance proof.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, PlateError> {
        if bytes.len() as u64 > MAX_OPERATION_EVIDENCE_BYTES {
            return Err(bad());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
        let mut lines = text.strip_suffix('\n').ok_or_else(bad)?.split('\n');
        if lines.next() != Some(MAGIC) {
            return Err(bad());
        }
        let mut field = |key: &str| -> Result<&str, PlateError> {
            lines
                .next()
                .and_then(|line| line.strip_prefix(key))
                .and_then(|line| line.strip_prefix(' '))
                .ok_or_else(bad)
        };
        let op_id = OpId::parse(field("op")?).map_err(|_| bad())?;
        let discarded = OpId::parse(field("discarded")?).map_err(|_| bad())?;
        let at = field("at")?.parse().map_err(|_| bad())?;
        let mut words = field("head")?.split(' ');
        let head = match (words.next(), words.next(), words.next(), words.next()) {
            (Some(revision), Some(manifest), Some(sequence), None) => RevisionPin {
                revision: host_uuid(revision)?,
                manifest: manifest.parse().map_err(|_| bad())?,
                sequence: sequence.parse().map_err(|_| bad())?,
            },
            _ => return Err(bad()),
        };
        let workspace = host_uuid(field("workspace")?)?;
        let project = host_uuid(field("project")?)?;
        let schema = field("schema")?.parse().map_err(|_| bad())?;
        if lines.next().is_some() || op_id == discarded || at == 0 {
            return Err(bad());
        }
        let receipt = Self {
            op_id,
            discarded,
            at,
            head,
            workspace,
            project,
            schema,
        };
        if receipt.encode()? != bytes {
            return Err(bad());
        }
        Ok(receipt)
    }
}

fn fingerprint(
    request: &ResolveRequest,
    meta: &WorkspaceMeta,
) -> Result<ContentDigest, PlateError> {
    // Bind caller arguments and immutable context. The receipt separately binds the actual pins.
    let text = format!(
        "beamcraft-resolve-call 1\nworkspace {}\nactor {}\nop {}\ndiscarded {}\nexpected {}\n",
        meta.workspace_id, request.actor, request.op_id, request.discarded, request.expected_draft,
    );
    Sha256::digest(text.as_bytes())
        .map_err(|_| integrity("resolve request digest could not be computed"))
}

fn replay(
    workspace: &RevisionStore,
    meta: &WorkspaceMeta,
    request: &ResolveRequest,
) -> Result<Option<Resolved>, PlateError> {
    let op = host_uuid(&request.op_id.to_string())?;
    let sequence = match workspace.query_outcome(&op)? {
        Outcome::Committed { sequence, .. } => Some(sequence),
        Outcome::Unknown => Some(workspace.sequence_of(&op)?.ok_or_else(|| {
            PlateError::new(
                ErrorCode::OutcomeUnknown,
                "resolve outcome is unknown; recover the revision store",
            )
        })?),
        Outcome::NotFound | Outcome::Aborted { .. } => {
            if workspace.changeset_claim(&op)?.is_some() {
                workspace.sequence_of(&op)?
            } else {
                None
            }
        }
    };
    let Some(sequence) = sequence else {
        return Ok(None);
    };
    let revision = workspace.open_at(sequence)?;
    let receipt = DiscardReceipt::decode(&revision.operation_evidence()?.ok_or_else(reuse)?)
        .map_err(|_| reuse())?;
    if receipt.op_id != request.op_id
        || receipt.discarded != request.discarded
        || receipt.head.revision.as_str() != request.expected_draft.as_str()
        || revision.info().revision().as_str() != op.as_str()
        || revision.manifest().content().op() != &op
        || revision.manifest().index().is_none()
        || revision.manifest().content().workspace() != &receipt.workspace
        || revision.manifest().content().project() != &receipt.project
        || revision.manifest().content().schema() != receipt.schema
        || revision.manifest().base().is_none_or(|base| {
            base.revision != receipt.head.revision || base.manifest != receipt.head.manifest
        })
        || receipt.workspace != meta.workspace_id
        || workspace.changeset_claim(&op)? != Some(fingerprint(request, meta)?)
    {
        return Err(reuse());
    }
    let renounced = crate::command_evidence::interval(
        workspace,
        receipt.at.saturating_sub(1),
        receipt.head.sequence,
    )?
    .into_iter()
    .map(|operation| operation.op_id)
    .collect();
    Ok(Some(Resolved {
        replayed: true,
        revision: summarize(revision.info()),
        renounced,
    }))
}

/// Resolves the draft's recorded semantic conflicts by renouncing one named own unsent
/// operation and every unsent operation after it.
///
/// The resolution commits a new draft revision that restores the exact layers of the state
/// before the named operation and carries the discard receipt as its evidence, so later
/// planning, packaging and publication exclude the renounced work while accepted history and
/// other actors' edits stay untouched. The resolution revision is an undo boundary: the derived
/// undo and redo stacks never reach across it, so no later step can resurrect the renounced
/// layers. Every input is bounded and checked: the named operation must be recorded unsent
/// command work of this workspace's actor, and the observed draft head must still be current
/// (`BASE_REVISION_MISMATCH` otherwise), so a concurrent local edit loses the draft CAS with the
/// same code. The renounced range must contain no evidence-less revision (`INVALID_ARGUMENT`);
/// the pending-conflict record resolves when the head moves. The only local writes are the
/// receipt blob, the undo-log boundary marker, the resolution revision and its intent; no
/// metadata, accepted store or published revision is touched.
pub fn resolve_workspace(
    workspace: &RevisionStore,
    dir: &Path,
    request: &ResolveRequest,
    hook: &mut dyn CommitHook,
) -> Result<Resolved, PlateError> {
    let state = WorkspaceState::open(dir)?;
    let meta = state.meta().cloned().ok_or_else(unusable)?;
    if request.actor != meta.user_id {
        return Err(PlateError::new(
            ErrorCode::PermissionDenied,
            "resolve actor does not own the workspace",
        ));
    }
    if let Some(prior) = replay(workspace, &meta, request)? {
        return Ok(prior);
    }
    let op = host_uuid(&request.op_id.to_string())?;
    workspace.claim_changeset(&op, &fingerprint(request, &meta)?)?;
    let head = workspace.open()?;
    let info = head.info();
    if info.revision().as_str() != request.expected_draft.as_str() {
        return Err(PlateError::new(
            ErrorCode::BaseRevisionMismatch,
            "the draft head moved since the resolution was planned",
        ));
    }
    if info.sequence() <= meta.synced_sequence {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "there is no unsent work to renounce",
        ));
    }
    let unsent =
        crate::command_evidence::interval(workspace, meta.synced_sequence, info.sequence())?;
    let position = unsent
        .iter()
        .rposition(|operation| operation.op_id == request.discarded)
        .ok_or_else(|| {
            PlateError::new(
                ErrorCode::InvalidArgument,
                "the named operation is not unsent work of this draft",
            )
        })?;
    let at = meta.synced_sequence.saturating_add(position as u64 + 1);
    if meta.rebase_sequence == at {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "a rebase candidate is not an editable command",
        ));
    }
    // An earlier resolution already renounces this operation and everything after it; a second
    // receipt for the same work has no retained operation to name, so refuse it up front. A
    // renounced range must also stay complete: an evidence-less revision inside it (a legacy
    // undo or redo) leaves the interval unverifiable later, so refuse before anything is
    // written instead of recording a resolution that cannot keep its promise.
    for sequence in meta.synced_sequence.saturating_add(1)..=info.sequence() {
        let revision = workspace.open_at(sequence)?;
        let Some(bytes) = revision.operation_evidence()? else {
            if sequence >= at {
                return Err(PlateError::new(
                    ErrorCode::InvalidArgument,
                    "the renounced range contains a revision without command evidence; redo or recommit it first",
                ));
            }
            continue;
        };
        if bytes.starts_with(b"beamcraft-workspace-discard ")
            && DiscardReceipt::decode(&bytes)?.at <= at
        {
            return Err(PlateError::new(
                ErrorCode::InvalidArgument,
                "the named operation is already renounced by a recorded resolution",
            ));
        }
    }
    let discarded = workspace.open_at(at)?;
    let captured = evidence::read(&discarded)?.ok_or_else(|| {
        PlateError::new(
            ErrorCode::InvalidArgument,
            "the named operation carries no command evidence",
        )
    })?;
    if captured.op_id != request.discarded {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "the named operation is not an editable command of this draft",
        ));
    }
    let content = head.manifest().content();
    let receipt = DiscardReceipt {
        op_id: request.op_id,
        discarded: request.discarded,
        at,
        head: RevisionPin::from_info(info),
        workspace: meta.workspace_id.clone(),
        project: content.project().clone(),
        schema: content.schema(),
    };
    let bytes = receipt.encode()?;
    let (digest, _) = workspace
        .blobs()
        .publish(&bytes)
        .map_err(|e| PlateError::new(e.code(), "discard receipt blob could not be stored"))?;
    let pin = EvidencePin::new(digest, bytes.len() as u64)?;
    let restored = workspace.open_at(at.saturating_sub(1))?;
    // The resolution revision is an undo boundary: the marker lands in the undo log before the
    // commit, so the stacks derived after it never reach across the renounced work. A marker
    // without its revision (a crash here) is ignored by the derivation, and the retry with the
    // same identity appends it again harmlessly.
    crate::history::append_resolved(dir, &op)?;
    let restored_content = restored.manifest().content();
    let mut entries = Vec::new();
    for entry in restored_content.entries() {
        let layer_bytes = restored.layer_bytes(entry.name())?;
        entries.push(LayerEntry::from_bytes(
            entry.name(),
            entry.role(),
            &layer_bytes,
        )?);
    }
    let generation = content
        .workspace_generation()
        .checked_add(1)
        .ok_or_else(|| PlateError::new(ErrorCode::LimitExceeded, "generation exhausted"))?;
    let next = RevisionDraft {
        project: content.project().clone(),
        workspace: content.workspace().clone(),
        revision: op.clone(),
        op: op.clone(),
        workspace_generation: generation,
        schema: content.schema(),
        root_layer: restored_content.root_layer().to_owned(),
        entries,
    };
    let committed = crate::publication::commit_verified_with_evidence(
        workspace,
        &next,
        &ExpectedBase::Revision(info.revision().clone()),
        Some(pin),
        hook,
    )?;
    Ok(Resolved {
        replayed: false,
        revision: summarize(&committed.info),
        renounced: unsent
            .get(position..)
            .unwrap_or_default()
            .iter()
            .map(|operation| operation.op_id)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(n: usize) -> RevisionPin {
        RevisionPin {
            revision: Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap(),
            manifest: "1111111111111111111111111111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            sequence: n as u64,
        }
    }

    fn receipt(n: usize) -> DiscardReceipt {
        DiscardReceipt {
            op_id: OpId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap(),
            discarded: OpId::parse(&format!("019a7bf0-0000-7000-8000-{:012x}", n + 1)).unwrap(),
            at: n as u64 + 2,
            head: pin(n + 3),
            workspace: Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{:012x}", n + 4)).unwrap(),
            project: Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{:012x}", n + 5)).unwrap(),
            schema: "2222222222222222222222222222222222222222222222222222222222222222"
                .parse()
                .unwrap(),
        }
    }

    #[test]
    fn the_receipt_round_trips_canonically_and_refuses_other_bytes() {
        let receipt = receipt(1);
        let bytes = receipt.encode().unwrap();
        assert_eq!(DiscardReceipt::decode(&bytes).unwrap(), receipt);
        assert!(DiscardReceipt::decode(&bytes[..bytes.len() - 1]).is_err());
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(DiscardReceipt::decode(&format!("{text}extra\n").into_bytes()).is_err());
        assert!(DiscardReceipt::decode(text.replace("at 3", "at 03").as_bytes()).is_err());
        let mut other = receipt.clone();
        other.discarded = other.op_id;
        assert!(other.encode().is_err());
        let mut zero = receipt.clone();
        zero.at = 0;
        assert!(zero.encode().is_err());
        assert!(
            DiscardReceipt::decode(
                &text
                    .replace(
                        "beamcraft-workspace-discard 1",
                        "beamcraft-workspace-discard 2"
                    )
                    .into_bytes()
            )
            .is_err()
        );
    }
}
