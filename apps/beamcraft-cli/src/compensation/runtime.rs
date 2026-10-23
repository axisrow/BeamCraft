// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C06.2: explicit semantic compensation in a clean current personal workspace.

use super::verify_original;
use crate::{
    command_evidence as evidence,
    plate::{self, PlateError, RevisionSummary, integrity},
    workspace::WorkspaceState,
};
use beamcraft_engine::rebase::{Conflict, Decision, Operation, plan};
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

/// Explicit immutable caller inputs; the actor is trusted local context, not authentication.
pub struct CompensationRequest {
    /// New compensating operation and revision identity, reused verbatim on retry.
    pub op_id: OpId,
    /// Acting owner of the source and destination personal workspaces.
    pub actor: Uuid7,
    /// Direct publication accounted for by one original terminal command.
    pub original: Uuid7,
    /// Exact observed accepted revision.
    pub expected_head: Uuid7,
    /// Exact observed clean personal draft revision.
    pub expected_draft: Uuid7,
}

/// Complete original result of a compensating request; later model edits are retained on replay.
pub struct Compensated {
    /// True when no new revision was committed by this call.
    pub replayed: bool,
    /// The original compensating revision, possibly historical after a later edit.
    pub revision: RevisionSummary,
    /// Original direct publication; its immutable history was not rewritten.
    pub original: Uuid7,
    /// Accepted basis used by this personal operation; publication remains explicit.
    pub accepted: Uuid7,
}

/// Automatic compensation either preserves all later unrelated edits or refuses explicitly.
pub enum CompensationResult {
    /// Verified inverse committed once to the personal draft.
    Compensated(Box<Compensated>),
    /// Current dependencies prevent this inverse; no model or metadata is changed.
    Conflicted(Vec<Conflict>),
}

fn reuse() -> PlateError {
    PlateError::new(
        ErrorCode::OpIdReuse,
        "compensation identity is bound to another request",
    )
}
fn opened(revision: &Revision) -> Result<beamcraft_layer_engine::OpenRequest, PlateError> {
    plate::open_base(&plate::base_of(revision)?)
}

/// Compensate a proven direct publication in a clean workspace at the current accepted basis.
///
/// The inverse is the original command's semantic delta, not a snapshot restore. All accepted
/// operations after that publication participate, including unchanged reads and reverted writes.
/// The closed materializer restores only covered groups onto the current model and the ordinary
/// kernel/quantity verifier checks it before staging and during commit. Missing proof refuses.
/// Local unsent work or a stale basis requires an explicit separate workspace/rebase workflow;
/// this slice refuses those cases rather than hiding or replacing that work. Publication is a
/// subsequent explicit operation. The accepted head is pinned; a concurrent draft CAS may win.
pub fn compensate(
    published: &RevisionStore,
    workspace: &RevisionStore,
    dir: &Path,
    source: &RevisionStore,
    source_dir: &Path,
    request: &CompensationRequest,
    hook: &mut dyn CommitHook,
) -> Result<CompensationResult, PlateError> {
    // Source metadata is released before destination metadata is acquired, also when both
    // directories are the same. Original maintenance guards retain immutable proof throughout.
    let original = verify_original(
        published,
        source,
        source_dir,
        &request.actor,
        &request.original,
    )?;
    let canonical = std::fs::canonicalize(dir)
        .map_err(|_| integrity("compensation workspace is unavailable"))?;
    if canonical != workspace.blobs().canonical_root()
        || canonical == published.blobs().canonical_root()
    {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "compensation requires its own matching personal store",
        ));
    }
    let state = WorkspaceState::open(dir)?;
    let meta = state
        .meta()
        .ok_or_else(|| integrity("compensation workspace is uninitialized"))?;
    if meta.user_id != request.actor {
        return Err(PlateError::new(
            ErrorCode::PermissionDenied,
            "compensation actor does not own the destination workspace",
        ));
    }
    let c = original.after().manifest().content();
    let fingerprint: ContentDigest = Sha256::digest(format!(
        "beamcraft-compensation-call 1\nworkspace {}\nactor {}\nop {}\nproject {}\nschema {}\nsource {}\noriginal {}\nbefore {}\nafter {}\npublished {}\nexpected {} {}\n",
        meta.workspace_id, request.actor, request.op_id, c.project(), c.schema(), c.workspace(), request.original,
        original.before().info().manifest_digest(), original.after().info().manifest_digest(), original.published().info().manifest_digest(), request.expected_head, request.expected_draft
    ).as_bytes()).map_err(|_| integrity("compensation request digest failed"))?;
    let op = plate::host_uuid(&request.op_id.to_string())?;
    // Binding precedes mutable head checks. A moved head changes a precondition, not this call.
    workspace.claim_changeset(&op, &fingerprint)?;
    let sequence = match workspace.query_outcome(&op)? {
        Outcome::Committed { sequence, .. } => Some(sequence),
        Outcome::Unknown => Some(workspace.sequence_of(&op)?.ok_or_else(|| {
            PlateError::new(
                ErrorCode::OutcomeUnknown,
                "compensation outcome needs recovery",
            )
        })?),
        Outcome::NotFound | Outcome::Aborted { .. } => workspace.sequence_of(&op)?,
    };
    if let Some(sequence) = sequence {
        let recorded = workspace.open_at(sequence)?;
        let proof = evidence::read(&recorded)?.ok_or_else(reuse)?;
        if proof.op_id != request.op_id
            || proof.fingerprint != fingerprint
            || recorded.info().revision() != &op
            || recorded.manifest().content().workspace() != &meta.workspace_id
            || recorded.manifest().content().project() != c.project()
            || recorded.manifest().content().schema() != c.schema()
            || recorded
                .manifest()
                .base()
                .is_none_or(|base| base.revision != request.expected_draft)
            || recorded.manifest().index().is_none()
        {
            return Err(reuse());
        }
        return Ok(CompensationResult::Compensated(Box::new(Compensated {
            replayed: true,
            revision: plate::summarize(recorded.info()),
            original: request.original.clone(),
            accepted: request.expected_head.clone(),
        })));
    }
    let _accepted = PublicationLease::exclusive(published.blobs())
        .map_err(|e| PlateError::new(e.code(), "accepted store is busy or unavailable"))?;
    let accepted = published.open()?;
    let draft = workspace.open()?;
    if accepted.info().revision() != &request.expected_head
        || draft.info().revision() != &request.expected_draft
    {
        return Err(PlateError::new(
            ErrorCode::BaseRevisionMismatch,
            "a compensation input head moved",
        ));
    }
    let a = accepted.manifest().content();
    let d = draft.manifest().content();
    if meta.base_revision != *accepted.info().revision()
        || meta.base_manifest != accepted.info().manifest_digest()
    {
        return Err(PlateError::new(
            ErrorCode::BaseRevisionMismatch,
            "compensation workspace must use the current accepted basis",
        ));
    }
    if draft.info().sequence() != meta.synced_sequence
        || d.entries() != a.entries()
        || d.root_layer() != a.root_layer()
    {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "compensation requires a clean personal workspace; retain unsent work separately",
        ));
    }
    if d.workspace() != &meta.workspace_id
        || d.project() != c.project()
        || a.project() != c.project()
        || d.schema() != c.schema()
        || a.schema() != c.schema()
        || meta.rebase_sequence > draft.info().sequence()
    {
        return Err(integrity("compensation contexts differ"));
    }
    let start = original.published().info().sequence();
    if start > accepted.info().sequence() {
        return Err(integrity(
            "original publication is beyond the accepted head",
        ));
    }
    let later = if start == accepted.info().sequence() {
        Vec::new()
    } else {
        let operations = evidence::interval(published, start, accepted.info().sequence())?;
        let interval = evidence::aggregate(published, start, &operations)?;
        if interval.footprint.is_none() {
            return Err(PlateError::new(
                ErrorCode::DependencyUnavailable,
                "intervening accepted command proof is incomplete",
            ));
        }
        interval.commands
    };
    let footprint = original.command().footprint.clone();
    if let Decision::Conflicted { conflicts } = plan(
        &[Operation {
            id: request.op_id,
            footprint: footprint.clone(),
        }],
        &later,
    )? {
        return Ok(CompensationResult::Conflicted(conflicts));
    }
    let candidate = materializer::prepare(
        &opened(original.after())?,
        &opened(original.before())?,
        &opened(&draft)?,
        &evidence::dependencies(&footprint),
        &EditTargets {
            authored: plate::AUTHORED_LAYER.into(),
            derived: plate::DERIVED_LAYER.into(),
        },
        &plate::control(),
    )?;
    let control = plate::control();
    let mut reader = Reader::default();
    let (view, _) = reader.open_validated(candidate.clone(), schema_identity()?, &control)?;
    let decoded = plate::decode(&reader, &view, &control)?;
    let (plates, members) = plate::evaluate_decoded(&decoded)?;
    plate::verify_derived(&plates, &members, &decoded, true)?;
    let mut entries = Vec::with_capacity(candidate.assets.len());
    for asset in candidate.assets {
        let role = d
            .entries()
            .iter()
            .find(|e| e.name() == asset.name)
            .ok_or_else(|| integrity("inverse layer lacks a source role"))?
            .role();
        workspace
            .blobs()
            .publish(&asset.bytes)
            .map_err(|e| PlateError::new(e.code(), "inverse blob could not be stored"))?;
        entries.push(LayerEntry::from_bytes(&asset.name, role, &asset.bytes)?);
    }
    let pin = evidence::store(workspace, &op, fingerprint, &footprint)?;
    let candidate = RevisionDraft {
        project: d.project().clone(),
        workspace: d.workspace().clone(),
        revision: op.clone(),
        op,
        workspace_generation: d.workspace_generation().checked_add(1).ok_or_else(|| {
            PlateError::new(
                ErrorCode::LimitExceeded,
                "compensation generation exhausted",
            )
        })?,
        schema: d.schema(),
        root_layer: d.root_layer().into(),
        entries,
    };
    let committed = plate::commit_verified_with_evidence(
        workspace,
        &candidate,
        &ExpectedBase::Revision(request.expected_draft.clone()),
        Some(pin),
        hook,
    )?;
    Ok(CompensationResult::Compensated(Box::new(Compensated {
        replayed: false,
        revision: plate::summarize(&committed.info),
        original: request.original.clone(),
        accepted: request.expected_head.clone(),
    })))
}
