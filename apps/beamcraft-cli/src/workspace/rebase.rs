// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C03.11: explicit, guarded rebase of an unsent draft, without publication.

use super::{
    WorkspaceMeta, WorkspaceState, published_base,
    rebase_receipt::{RebaseReceipt, RevisionPin},
    unusable,
};
use crate::{
    command_evidence as evidence,
    plate::{self, PlateError, RevisionSummary, integrity, summarize},
};
use beamcraft_engine::rebase::{Conflict, Decision, plan};
use beamcraft_host::{
    lease::PublicationLease,
    ledger::Outcome,
    revision::{
        CommitHook, ExpectedBase, LayerEntry, Revision, RevisionDraft, RevisionStore, Uuid7,
    },
};
use beamcraft_layer_engine::{
    Reader,
    authored::{EditTargets, rebase as materializer, schema_identity},
};
use beamcraft_model::{
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
    id::OpId,
};
use std::path::Path;

/// Immutable caller inputs. The CLI actor is trusted local context; a coordinator must derive it
/// from authentication. Reusing an identity with another payload is refused, including after a
/// failed attempt. Both expected revision identities are mandatory.
pub struct RebaseRequest {
    /// Reusable operation and candidate revision identity.
    pub op_id: OpId,
    /// Trusted acting user, required to own the workspace.
    pub actor: Uuid7,
    /// Exact observed accepted revision.
    pub expected_head: Uuid7,
    /// Exact original personal draft revision.
    pub expected_draft: Uuid7,
}

/// One completed rebase, or its original historical result on replay.
pub struct Rebased {
    /// True when this operation already committed; no new revision is written.
    pub replayed: bool,
    /// The candidate revision, not necessarily the current head after later edits.
    pub revision: RevisionSummary,
    /// The accepted basis this operation used.
    pub accepted: RevisionPin,
    /// Ordered retained unsent commands/administrative receipts in the original request.
    pub retained_operations: usize,
}

/// An automatic rebase never chooses a conflict winner.
pub enum RebaseResult {
    /// The complete merged draft committed, or the exact original result replayed.
    Rebased(Box<Rebased>),
    /// Complete bounded semantic diagnostics; both heads and metadata remain unchanged.
    Conflicted(Vec<Conflict>),
}

fn reuse() -> PlateError {
    PlateError::new(
        ErrorCode::OpIdReuse,
        "rebase identity is bound to another request",
    )
}
fn mismatch() -> PlateError {
    PlateError::new(ErrorCode::BaseRevisionMismatch, "a rebase input head moved")
}

fn fingerprint(
    request: &RebaseRequest,
    meta: &WorkspaceMeta,
    project: &Uuid7,
    schema: ContentDigest,
) -> Result<ContentDigest, PlateError> {
    // Bind caller arguments and immutable context. A later observed head changes preconditions,
    // not the original call. The receipt separately binds every actual input pin and metadata.
    let text = format!(
        "beamcraft-rebase-call 1\nworkspace {}\nactor {}\nop {}\nproject {project}\nschema {schema}\nexpected {} {}\n",
        meta.workspace_id,
        request.actor,
        request.op_id,
        request.expected_head,
        request.expected_draft
    );
    Sha256::digest(text.as_bytes())
        .map_err(|_| integrity("rebase request digest could not be computed"))
}

fn replay(
    store: &RevisionStore,
    state: &WorkspaceState,
    request: &RebaseRequest,
) -> Result<Option<Rebased>, PlateError> {
    let op = plate::host_uuid(&request.op_id.to_string())?;
    let sequence = match store.query_outcome(&op)? {
        Outcome::Committed { sequence, .. } => Some(sequence),
        Outcome::Unknown => Some(store.sequence_of(&op)?.ok_or_else(|| {
            PlateError::new(
                ErrorCode::OutcomeUnknown,
                "rebase outcome is unknown; recover the revision store",
            )
        })?),
        Outcome::NotFound | Outcome::Aborted { .. } => {
            if store.changeset_claim(&op)?.is_some() {
                store.sequence_of(&op)?
            } else {
                None
            }
        }
    };
    let Some(sequence) = sequence else {
        return Ok(None);
    };
    let revision = store.open_at(sequence)?;
    let receipt = RebaseReceipt::decode(&revision.operation_evidence()?.ok_or_else(reuse)?)
        .map_err(|_| reuse())?;
    let meta = state.meta().ok_or_else(unusable)?;
    super::pending::verify_source(store, &receipt)?;
    if receipt.op_id != request.op_id
        || receipt.old_meta.user_id != request.actor
        || receipt.old_meta.workspace_id != meta.workspace_id
        || receipt.accepted.revision != request.expected_head
        || receipt.draft.revision != request.expected_draft
        || revision.info().revision() != &op
        || revision.manifest().content().op() != &op
        || revision.manifest().index().is_none()
        || meta.rebase_sequence < sequence
        || sequence != receipt.draft.sequence.saturating_add(1)
        || revision.manifest().content().project() != &receipt.project
        || revision.manifest().content().workspace() != &receipt.old_meta.workspace_id
        || revision.manifest().content().schema() != receipt.schema
        || revision.manifest().base().is_none_or(|base| {
            base.revision != receipt.draft.revision || base.manifest != receipt.draft.manifest
        })
        || store.changeset_claim(&op)?
            != Some(fingerprint(
                request,
                &receipt.old_meta,
                &receipt.project,
                receipt.schema,
            )?)
    {
        return Err(reuse());
    }
    Ok(Some(Rebased {
        replayed: true,
        revision: summarize(revision.info()),
        accepted: receipt.accepted,
        retained_operations: receipt.operations.len(),
    }))
}

fn opened(revision: &Revision) -> Result<beamcraft_layer_engine::OpenRequest, PlateError> {
    plate::open_base(&plate::base_of(revision)?)
}

/// Rebase a personal draft onto an exact accepted head, retaining every independent unsent edit.
/// Holds metadata and accepted publication/maintenance guards through planning and the draft CAS.
/// Ordinary local edits remain possible; a winning edit makes this CAS refuse. All intervening
/// footprints participate, including reverted writes and indirect reads. Missing proof refuses.
/// A successful candidate passes the kernel/quantity verifier before intent staging and again
/// during the normal atomic commit. Recovery finalizes metadata without publishing the draft.
pub fn rebase_workspace(
    published: &RevisionStore,
    workspace: &RevisionStore,
    dir: &Path,
    request: &RebaseRequest,
    hook: &mut dyn CommitHook,
) -> Result<RebaseResult, PlateError> {
    let canonical = std::fs::canonicalize(dir).map_err(|_| unusable())?;
    if canonical != workspace.blobs().canonical_root()
        || canonical == published.blobs().canonical_root()
    {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "rebase stores must be distinct and match the workspace directory",
        ));
    }
    let mut state = WorkspaceState::open(dir)?;
    let meta = state.meta().cloned().ok_or_else(unusable)?;
    if request.actor != meta.user_id {
        return Err(PlateError::new(
            ErrorCode::PermissionDenied,
            "rebase actor does not own the workspace",
        ));
    }
    if let Some(prior) = replay(workspace, &state, request)? {
        return Ok(RebaseResult::Rebased(Box::new(prior)));
    }
    let _accepted_guard = PublicationLease::exclusive(published.blobs())
        .map_err(|error| PlateError::new(error.code(), "accepted store is busy or unavailable"))?;
    let accepted = published.open()?;
    let draft = workspace.open()?;
    let (draft_pin, accepted_pin) = (
        RevisionPin::from_info(draft.info()),
        RevisionPin::from_info(accepted.info()),
    );
    let content = draft.manifest().content();
    let op = plate::host_uuid(&request.op_id.to_string())?;
    workspace.claim_changeset(
        &op,
        &fingerprint(request, &meta, content.project(), content.schema())?,
    )?;
    if request.expected_draft != draft_pin.revision
        || request.expected_head != accepted_pin.revision
    {
        return Err(mismatch());
    }
    if meta.base_revision == accepted_pin.revision || draft_pin.sequence <= meta.synced_sequence {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "rebase requires unsent work and a newer accepted basis",
        ));
    }
    if meta.rebase_sequence > draft_pin.sequence {
        return Err(integrity("rebase boundary is beyond the draft head"));
    }
    let base = published_base(published, &meta)?;
    let a = accepted.manifest().content();
    let b = base.manifest().content();
    if content.project() != b.project()
        || content.project() != a.project()
        || content.schema() != b.schema()
        || content.schema() != a.schema()
        || content.workspace() != &meta.workspace_id
        || b.workspace() != a.workspace()
    {
        return Err(integrity("rebase input contexts differ"));
    }
    let operations = evidence::interval(workspace, meta.synced_sequence, draft_pin.sequence)?;
    let source = evidence::aggregate(workspace, meta.synced_sequence, &operations)?;
    let intervening = evidence::interval(published, base.info().sequence(), accepted_pin.sequence)?;
    let remote = evidence::aggregate(published, base.info().sequence(), &intervening)?;
    let footprint = source.footprint.ok_or_else(|| {
        PlateError::new(
            ErrorCode::DependencyUnavailable,
            "draft has no complete command evidence",
        )
    })?;
    if remote.footprint.is_none() {
        return Err(PlateError::new(
            ErrorCode::DependencyUnavailable,
            "accepted interval has no complete command evidence",
        ));
    }
    if source.head.manifest_digest() != draft_pin.manifest
        || remote.head.manifest_digest() != accepted_pin.manifest
    {
        return Err(mismatch());
    }
    if let Decision::Conflicted { conflicts } = plan(&source.commands, &remote.commands)? {
        // A conflicted rebase writes this record and nothing else; the same inputs reproduce
        // the same bytes, so a retry after any interruption reports identically.
        super::conflicts::store(
            dir,
            &super::conflicts::PendingConflicts {
                base_revision: meta.base_revision.clone(),
                base_manifest: meta.base_manifest,
                draft_head: draft_pin.revision,
                draft_manifest: draft_pin.manifest,
                conflicts: conflicts.clone(),
            },
        )?;
        return Ok(RebaseResult::Conflicted(conflicts));
    }
    let candidate = materializer::prepare(
        &opened(&base)?,
        &opened(&draft)?,
        &opened(&accepted)?,
        &evidence::dependencies(&footprint),
        &EditTargets {
            authored: plate::AUTHORED_LAYER.into(),
            derived: plate::DERIVED_LAYER.into(),
        },
        &plate::control(),
    )?;
    let mut reader = Reader::default();
    let control = plate::control();
    let (view, _) = reader.open_validated(candidate.clone(), schema_identity()?, &control)?;
    let decoded = plate::decode(&reader, &view, &control)?;
    let (plates, members) = plate::evaluate_decoded(&decoded)?;
    plate::verify_derived(&plates, &members, &decoded, true)?;
    let mut entries = Vec::with_capacity(candidate.assets.len());
    for asset in candidate.assets {
        workspace
            .blobs()
            .publish(&asset.bytes)
            .map_err(|_| integrity("candidate blob could not be stored"))?;
        let role = a
            .entries()
            .iter()
            .find(|e| e.name() == asset.name)
            .ok_or_else(|| integrity("candidate layer has no accepted role"))?
            .role();
        entries.push(LayerEntry::from_bytes(&asset.name, role, &asset.bytes)?);
    }
    let receipt = RebaseReceipt {
        op_id: request.op_id,
        project: content.project().clone(),
        schema: content.schema(),
        old_meta: meta,
        old_base_sequence: base.info().sequence(),
        draft: draft_pin,
        accepted: accepted_pin,
        operations,
        footprint,
    };
    let next =
        RevisionDraft {
            project: receipt.project.clone(),
            workspace: receipt.old_meta.workspace_id.clone(),
            revision: op.clone(),
            op,
            workspace_generation: content.workspace_generation().checked_add(1).ok_or_else(
                || PlateError::new(ErrorCode::LimitExceeded, "draft generation exhausted"),
            )?,
            schema: receipt.schema,
            root_layer: content.root_layer().into(),
            entries,
        };
    let pin = state.stage_rebase(&receipt)?;
    let committed = crate::publication::commit_verified_with_evidence(
        workspace,
        &next,
        &ExpectedBase::Revision(receipt.draft.revision.clone()),
        Some(pin),
        hook,
    )?;
    state
        .finish_rebase()?
        .ok_or_else(|| integrity("committed rebase was not finalized"))?;
    Ok(RebaseResult::Rebased(Box::new(Rebased {
        replayed: false,
        revision: summarize(&committed.info),
        accepted: receipt.accepted,
        retained_operations: receipt.operations.len(),
    })))
}
