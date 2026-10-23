// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C06.1: verify original images that exactly account for a direct publication.
//!
//! This adapter changes no model. It does not compensate from a caller-supplied inverse or a
//! final-state diff. A subsequent inverse adapter must compare all intervening dependencies and
//! verify its current preconditions before materializing and committing a new personal revision.

use crate::{
    command_evidence::CapturedFootprint,
    plate::{PlateError, integrity},
    workspace::WorkspaceState,
};
use beamcraft_host::{
    lease::StoreLease,
    revision::{Revision, RevisionStore, Uuid7},
};
use beamcraft_model::error::ErrorCode;
use std::path::Path;

pub mod runtime;

/// Verified immutable original images, retaining maintenance guards against history collection.
/// Fields are private: an inverse caller cannot substitute arbitrary before/after USD bytes.
pub struct Original {
    before: Revision,
    after: Revision,
    published: Revision,
    command: CapturedFootprint,
    _source_maintenance: StoreLease,
    _published_maintenance: StoreLease,
}

impl Original {
    /// Verified original command's before-image in its source workspace.
    pub fn before(&self) -> &Revision {
        &self.before
    }
    /// Verified original command's after-image in its source workspace.
    pub fn after(&self) -> &Revision {
        &self.after
    }
    /// Exact accepted revision whose complete delta is accounted for by this terminal command.
    pub fn published(&self) -> &Revision {
        &self.published
    }
    /// Canonical actual footprint and argument fingerprint pinned by the original producer.
    pub fn command(&self) -> &CapturedFootprint {
        &self.command
    }
}

fn unavailable() -> PlateError {
    PlateError::new(
        ErrorCode::DependencyUnavailable,
        "original publication proof is unavailable or unsupported",
    )
}

/// Verify source ownership, exact historical publication membership and both original images.
///
/// Supported: a terminal personal command accounting for a `workspace-publish` delta. That path uses
/// the source revision identity as its published identity and pins the source manifest as its
/// request fingerprint. The immediately preceding source layers must equal the immediately
/// preceding accepted layers; the resulting complete layer sets and footprints must also match.
/// This refuses any additional published delta/dependency, C01 payload fingerprints, rebase
/// administration and missing/compacted proof. Earlier commands reverting to the exact prior
/// accepted bytes with no additional dependency do not invalidate this terminal command's inverse.
/// It never guesses a publication boundary to extract one command from a larger accepted delta.
/// The source workspace may have advanced since publication. The supplied actor must still own
/// it; ownership of the destination workspace and current-head checks belong to the caller.
pub fn verify_original(
    published: &RevisionStore,
    source: &RevisionStore,
    source_dir: &Path,
    actor: &Uuid7,
    revision: &Uuid7,
) -> Result<Original, PlateError> {
    let canonical = std::fs::canonicalize(source_dir).map_err(|_| unavailable())?;
    if canonical != source.blobs().canonical_root()
        || canonical == published.blobs().canonical_root()
    {
        return Err(PlateError::new(
            ErrorCode::InvalidArgument,
            "original source must be its own personal workspace",
        ));
    }
    let source_maintenance = StoreLease::shared(source.blobs())
        .map_err(|e| PlateError::new(e.code(), "original source is in use or unavailable"))?;
    let published_maintenance = StoreLease::shared(published.blobs())
        .map_err(|e| PlateError::new(e.code(), "accepted history is in use or unavailable"))?;
    let state = WorkspaceState::open(source_dir)?;
    let meta = state.meta().ok_or_else(unavailable)?;
    if actor != &meta.user_id {
        return Err(PlateError::new(
            ErrorCode::PermissionDenied,
            "compensation actor does not own the original workspace",
        ));
    }
    let source_sequence = source
        .sequence_of(revision)?
        .filter(|s| *s > 1)
        .ok_or_else(unavailable)?;
    let accepted_sequence = published
        .sequence_of(revision)?
        .filter(|s| *s > 1)
        .ok_or_else(unavailable)?;
    let after = source.open_at(source_sequence)?;
    let before = source.open_at(source_sequence - 1)?;
    let accepted = published.open_at(accepted_sequence)?;
    let accepted_before = published.open_at(accepted_sequence - 1)?;
    let s = after.manifest().content();
    let a = accepted.manifest().content();
    let b = before.manifest().content();
    let p = accepted_before.manifest().content();
    if after.info().revision() != revision
        || accepted.info().revision() != revision
        || a.op() != revision
        || s.workspace() != &meta.workspace_id
        || s.project() != a.project()
        || s.schema() != a.schema()
        || b.project() != p.project()
        || b.schema() != p.schema()
        || s.entries() != a.entries()
        || b.entries() != p.entries()
        || s.root_layer() != a.root_layer()
        || b.root_layer() != p.root_layer()
        || after.manifest().index().is_none()
        || accepted.manifest().index().is_none()
    {
        return Err(unavailable());
    }
    // Use the command codec, not the polymorphic administrative-receipt decoder. A rebase
    // receipt has no independently supported modelling inverse in this proof slice.
    let command = CapturedFootprint::decode(&after.operation_evidence()?.ok_or_else(unavailable)?)
        .map_err(|_| unavailable())?;
    let publication =
        CapturedFootprint::decode(&accepted.operation_evidence()?.ok_or_else(unavailable)?)
            .map_err(|_| unavailable())?;
    if command.op_id.to_string() != s.op().as_str()
        || publication.op_id.to_string() != a.op().as_str()
        || publication.fingerprint != after.info().manifest_digest()
        || publication.footprint != command.footprint
    {
        return Err(integrity(
            "publication evidence does not prove this original command",
        ));
    }
    Ok(Original {
        before,
        after,
        published: accepted,
        command,
        _source_maintenance: source_maintenance,
        _published_maintenance: published_maintenance,
    })
}
