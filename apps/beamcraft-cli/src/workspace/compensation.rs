// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C06: compensation of a published operation as a new forward operation.
//!
//! A compensation never rewrites or removes accepted history. It appends one ordinary,
//! evidence-bearing draft operation that restores the target's semantic write groups to the
//! state before the target, materialized against the **current** published head with the
//! layer-engine rebase materializer (whose field checks refuse when a later accepted opinion
//! already moved the state). The planner runs first and refuses with the same bounded report
//! as C04 when any accepted operation after the target intersects the target's footprint, so
//! a later edit by another actor is never rolled back and never silently overwritten. The
//! compensation then flows through the ordinary publication path (package and publish).
//!
//! Ownership: the caller's actor must own the workspace. The accepted store records no
//! per-operation author, so "its own published operation" is enforced through the workspace
//! identity chain: a compensation enters only the caller's own draft, and its later
//! publication is that workspace's actor's changeset (C01/C02).

use super::{WorkspaceState, rebase_receipt::RevisionPin, unusable};
use crate::{
    command_evidence as evidence,
    plate::{self, PlateError, RevisionSummary, integrity, summarize},
    workspace::rebase_receipt::RebaseReceipt,
};
use beamcraft_engine::rebase::{Conflict, Decision, Operation as PlannedOperation, plan};
use beamcraft_host::revision::{
    CommitHook, ExpectedBase, LayerEntry, RevisionDraft, RevisionStore, Uuid7,
};
use beamcraft_layer_engine::{
    Reader,
    authored::{EditTargets, rebase as materializer, schema_identity},
};
use beamcraft_model::{
    changeset::MAX_OPERATIONS, digest::Sha256, error::ErrorCode, id::OpId,
    limits::MAX_FOOTPRINT_ENTRIES,
};
use std::path::Path;

/// Immutable caller inputs. The CLI actor is trusted local context; a coordinator derives it
/// from authentication. Reusing the identity with another payload is refused, including after
/// a failed attempt. Both the target revision and the observed accepted head are mandatory.
pub struct CompensationRequest {
    /// Reusable operation and candidate revision identity of the compensation itself.
    pub op_id: OpId,
    /// Trusted acting user, required to own the workspace.
    pub actor: Uuid7,
    /// The accepted revision whose effect is compensated.
    pub target_revision: Uuid7,
    /// Exact observed accepted revision; preconditions are checked against it.
    pub expected_head: Uuid7,
}

/// One completed compensation, or its original historical result on replay.
pub struct Compensated {
    /// True when this operation already committed; no new revision is written.
    pub replayed: bool,
    /// The draft revision holding the compensated state.
    pub revision: RevisionSummary,
    /// The accepted basis this compensation used.
    pub accepted: RevisionPin,
    /// The accepted revision whose effect was compensated.
    pub target: RevisionPin,
}

/// A compensation never chooses a conflict winner.
pub enum CompensationResult {
    /// The compensating draft operation committed, or the exact original result replayed.
    Compensated(Box<Compensated>),
    /// Complete bounded semantic diagnostics; both heads and metadata remain unchanged.
    Conflicted(Vec<Conflict>),
}

fn reuse() -> PlateError {
    PlateError::new(
        ErrorCode::OpIdReuse,
        "compensation identity is bound to another request",
    )
}

fn mismatch() -> PlateError {
    PlateError::new(ErrorCode::BaseRevisionMismatch, "the accepted head moved")
}

fn unavailable() -> PlateError {
    PlateError::new(
        ErrorCode::DependencyUnavailable,
        "the compensation target has no complete command evidence",
    )
}

fn compensation_fingerprint(
    request: &CompensationRequest,
    workspace_id: &Uuid7,
    target: &RevisionPin,
) -> Result<beamcraft_model::digest::ContentDigest, PlateError> {
    // Bind caller arguments and the exact accepted target pin. A later observed head changes
    // preconditions, not the original call.
    let text = format!(
        "beamcraft-compensation 1\nworkspace {workspace_id}\nactor {}\nop {}\ntarget {} {}\nexpected {}\n",
        request.actor,
        request.op_id,
        request.target_revision,
        target.manifest,
        request.expected_head
    );
    Sha256::digest(text.as_bytes())
        .map_err(|_| integrity("compensation request digest could not be computed"))
}

fn replay(
    published: &RevisionStore,
    workspace: &RevisionStore,
    request: &CompensationRequest,
    workspace_id: &Uuid7,
) -> Result<Option<Compensated>, PlateError> {
    let op = plate::host_uuid(&request.op_id.to_string())?;
    let sequence = match workspace.query_outcome(&op)? {
        beamcraft_host::ledger::Outcome::Committed { sequence, .. } => Some(sequence),
        beamcraft_host::ledger::Outcome::Unknown => {
            Some(workspace.sequence_of(&op)?.ok_or_else(|| {
                PlateError::new(
                    ErrorCode::OutcomeUnknown,
                    "compensation outcome is unknown; recover the revision store",
                )
            })?)
        }
        _ => return Ok(None),
    };
    let Some(sequence) = sequence else {
        return Ok(None);
    };
    let revision = workspace.open_at(sequence)?;
    let captured = evidence::read(&revision)?.ok_or_else(reuse)?;
    // The target and accepted pins live in the published store: a forked copy of a revision
    // carries its own manifest, so the fingerprint binds the accepted-side digests.
    let target_sequence = published
        .sequence_of(&request.target_revision)?
        .ok_or_else(reuse)?;
    let target = RevisionPin::from_info(&published.info_at(target_sequence)?);
    let accepted_sequence = published
        .sequence_of(&request.expected_head)?
        .ok_or_else(reuse)?;
    let accepted = RevisionPin::from_info(&published.info_at(accepted_sequence)?);
    if captured.op_id != request.op_id
        || captured.fingerprint != compensation_fingerprint(request, workspace_id, &target)?
        || revision.info().revision() != &op
    {
        return Err(reuse());
    }
    Ok(Some(Compensated {
        replayed: true,
        revision: summarize(revision.info()),
        accepted,
        target,
    }))
}

/// The accepted operations after the target, as planning units. Renounced work contributes
/// nothing; a rebase receipt contributes its whole source interval (a conservative superset of
/// the merge's writes); missing evidence is a refusal, never an empty proof.
fn later_operations(
    store: &RevisionStore,
    after: u64,
    through: u64,
) -> Result<Vec<PlannedOperation>, PlateError> {
    let count = through.saturating_sub(after);
    if count > MAX_OPERATIONS as u64 {
        return Err(PlateError::new(
            ErrorCode::LimitExceeded,
            "the accepted interval after the target exceeds the operation cap",
        ));
    }
    let mut operations = Vec::new();
    let mut entries = 0usize;
    for sequence in after..through {
        let next = sequence.saturating_add(1);
        let revision = store.open_at(next)?;
        let bytes = revision.operation_evidence()?.ok_or_else(unavailable)?;
        let footprint = if bytes.starts_with(b"beamcraft-workspace-rebase ") {
            RebaseReceipt::decode(&bytes)
                .map_err(|_| integrity("rebase evidence is malformed"))?
                .footprint
        } else if bytes.starts_with(b"beamcraft-workspace-discard ") {
            continue;
        } else {
            evidence::read(&revision)?
                .ok_or_else(unavailable)?
                .footprint
        };
        entries = entries
            .saturating_add(footprint.reads().len())
            .saturating_add(footprint.writes().len());
        if entries > MAX_FOOTPRINT_ENTRIES {
            return Err(PlateError::new(
                ErrorCode::LimitExceeded,
                "the accepted interval footprint exceeds 4096 entries",
            ));
        }
        let content = revision.manifest().content();
        operations.push(PlannedOperation {
            id: OpId::parse(content.op().as_str())
                .map_err(|_| integrity("accepted operation identity is invalid"))?,
            footprint,
        });
    }
    Ok(operations)
}

fn opened(
    revision: &beamcraft_host::revision::Revision,
) -> Result<beamcraft_layer_engine::OpenRequest, PlateError> {
    plate::open_base(&plate::base_of(revision)?)
}

/// Compensates one accepted operation of this workspace with a new forward draft operation,
/// restoring its semantic write groups to the accepted state before it. Preconditions are
/// checked against the current published head: the workspace must sit on it, the target must
/// carry complete evidence, and no accepted operation after the target may intersect the
/// target's footprint (the planner names both sides). The commit is expected against the draft
/// head, so a concurrent local edit loses with `BASE_REVISION_MISMATCH`; the compensation then
/// flows through the ordinary package-and-publish path.
pub fn compensate_workspace(
    published: &RevisionStore,
    workspace: &RevisionStore,
    dir: &Path,
    request: &CompensationRequest,
    hook: &mut dyn CommitHook,
) -> Result<CompensationResult, PlateError> {
    let canonical = std::fs::canonicalize(dir).map_err(|_| unusable())?;
    if canonical != workspace.blobs().canonical_root()
        || canonical == published.blobs().canonical_root()
    {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "compensation stores must be distinct and match the workspace directory",
        ));
    }
    let state = WorkspaceState::open(dir)?;
    let meta = state.meta().cloned().ok_or_else(unusable)?;
    if request.actor != meta.user_id {
        return Err(PlateError::new(
            ErrorCode::PermissionDenied,
            "compensation actor does not own the workspace",
        ));
    }
    if let Some(prior) = replay(published, workspace, request, &meta.workspace_id)? {
        return Ok(CompensationResult::Compensated(Box::new(prior)));
    }
    let accepted = published.open()?;
    let draft = workspace.open()?;
    let accepted_pin = RevisionPin::from_info(accepted.info());
    let draft_pin = RevisionPin::from_info(draft.info());
    let op = plate::host_uuid(&request.op_id.to_string())?;
    let target_sequence = published
        .sequence_of(&request.target_revision)?
        .ok_or_else(|| {
            PlateError::new(
                ErrorCode::InvalidArgument,
                "the compensation target is not in the published store",
            )
        })?;
    if target_sequence < 2 {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "the first accepted revision has no basis to restore",
        ));
    }
    let target = published.open_at(target_sequence)?;
    let target_pin = RevisionPin::from_info(target.info());
    workspace.claim_changeset(
        &op,
        &compensation_fingerprint(request, &meta.workspace_id, &target_pin)?,
    )?;
    if request.expected_head != accepted_pin.revision {
        return Err(mismatch());
    }
    if meta.base_revision != accepted_pin.revision {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "compensation requires the workspace at the current accepted head; rebase first",
        ));
    }
    // The candidate replaces the whole draft content, so a dirty draft would silently drop
    // its unsent operations. Unsent work is either published, undone or renounced through an
    // explicit recorded resolution first — the C03/C04 discipline; merging it here is a
    // follow-up, not an implicit loss.
    if draft_pin.sequence > meta.synced_sequence {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "compensation requires a clean draft; publish, undo or explicitly resolve the \
             unsent operations first",
        ));
    }
    let target_base = published.open_at(target_sequence - 1)?;
    let a = accepted.manifest().content();
    let t = target.manifest().content();
    let b = target_base.manifest().content();
    let d = draft.manifest().content();
    if a.project() != t.project()
        || a.project() != b.project()
        || a.schema() != t.schema()
        || a.schema() != b.schema()
        || a.workspace() != t.workspace()
        || a.workspace() != b.workspace()
        || d.project() != a.project()
        || d.schema() != a.schema()
        || d.workspace() != &meta.workspace_id
    {
        return Err(integrity("compensation input contexts differ"));
    }
    let captured = evidence::read(&target)?
        .filter(|c| {
            c.footprint
                .reads()
                .len()
                .saturating_add(c.footprint.writes().len())
                > 0
        })
        .ok_or_else(unavailable)?;
    let later = later_operations(published, target_sequence, accepted_pin.sequence)?;
    let inverse = PlannedOperation {
        id: request.op_id,
        footprint: captured.footprint.clone(),
    };
    if let Decision::Conflicted { conflicts } = plan(&[inverse], &later)? {
        return Ok(CompensationResult::Conflicted(conflicts));
    }
    // The materializer applies (target basis minus target) onto the current head and refuses
    // field by field when the current value is no longer the target's own result: the second
    // net behind the planner, so a moved state is never silently overwritten.
    let candidate = materializer::prepare(
        &opened(&target)?,
        &opened(&target_base)?,
        &opened(&accepted)?,
        &evidence::dependencies(&captured.footprint),
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
    let mut entries = Vec::new();
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
    // The compensation's own evidence is the target's footprint: it writes the same semantic
    // groups (in reverse), so later planning treats it like any other command.
    let pin = evidence::store(
        workspace,
        &op,
        compensation_fingerprint(request, &meta.workspace_id, &target_pin)?,
        &captured.footprint,
    )?;
    let next = RevisionDraft {
        project: d.project().clone(),
        workspace: meta.workspace_id.clone(),
        revision: op.clone(),
        op,
        workspace_generation: d.workspace_generation().checked_add(1).ok_or_else(|| {
            PlateError::new(ErrorCode::LimitExceeded, "draft generation exhausted")
        })?,
        schema: d.schema(),
        root_layer: d.root_layer().into(),
        entries,
    };
    let committed = crate::publication::commit_verified_with_evidence(
        workspace,
        &next,
        &ExpectedBase::Revision(draft_pin.revision.clone()),
        Some(pin),
        hook,
    )?;
    Ok(CompensationResult::Compensated(Box::new(Compensated {
        replayed: false,
        revision: summarize(&committed.info),
        accepted: accepted_pin,
        target: target_pin,
    })))
}
