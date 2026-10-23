// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Verify original images that exactly account for a direct or single-operation C01 publication.
//!
//! This adapter changes no model. It does not compensate from a caller-supplied inverse or a
//! final-state diff. A subsequent inverse adapter must compare all intervening dependencies and
//! verify its current preconditions before materializing and committing a new personal revision.

use crate::{
    changeset::read_changeset,
    command_evidence::{self, CapturedFootprint},
    plate::{PlateError, integrity},
    workspace::{WorkspaceMeta, WorkspaceState},
};
use beamcraft_host::{
    lease::StoreLease,
    revision::{MAX_MANIFEST_BYTES, Manifest, Revision, RevisionStore, Uuid7},
};
use beamcraft_model::error::ErrorCode;
use std::path::Path;

pub mod runtime;

fn source_result(
    source: &RevisionStore,
    accepted: &Revision,
    previous: &Revision,
    publication: &CapturedFootprint,
    meta: &WorkspaceMeta,
) -> Result<(Revision, Option<beamcraft_model::changeset::Changeset>), PlateError> {
    let a = accepted.manifest().content();
    if a.op() == accepted.info().revision()
        && let Some(sequence) = source.sequence_of(accepted.info().revision())?
    {
        let after = source.open_at(sequence)?;
        if after.info().revision() == accepted.info().revision()
            && publication.fingerprint == after.info().manifest_digest()
        {
            return Ok((after, None));
        }
    }
    // A C01/C02 accepted operation binds the canonical payload digest, not the draft manifest.
    // Verify the exact pinned payload and its single original result in actual source history.
    let changeset = read_changeset(source, &publication.fingerprint)
        .map_err(|_| unavailable())?
        .changeset;
    if changeset.operations.len() != 1
        || changeset.changeset_id.to_string() != a.op().as_str()
        || changeset.revision.to_string() != accepted.info().revision().as_str()
        || changeset.project.to_string() != a.project().as_str()
        || changeset.workspace.to_string() != meta.workspace_id.as_str()
        || changeset.actor.to_string() != meta.user_id.as_str()
        || changeset.base_revision.to_string() != previous.info().revision().as_str()
        || changeset.base_manifest != previous.info().manifest_digest()
        || changeset.root_layer != a.root_layer()
        || changeset.layers.len() != a.entries().len()
    {
        return Err(unavailable());
    }
    for (layer, entry) in changeset.layers.iter().zip(a.entries()) {
        let role = match layer.role {
            beamcraft_model::changeset::LayerRole::Authored => {
                beamcraft_host::revision::LayerRole::Authored
            }
            beamcraft_model::changeset::LayerRole::Derived => {
                beamcraft_host::revision::LayerRole::Derived
            }
            beamcraft_model::changeset::LayerRole::External => {
                beamcraft_host::revision::LayerRole::External
            }
        };
        let base = previous
            .manifest()
            .content()
            .entries()
            .iter()
            .find(|e| e.name() == layer.name)
            .map(|e| e.digest());
        if layer.name != entry.name()
            || layer.digest != entry.digest()
            || layer.size != entry.size()
            || role != entry.role()
            || layer.base != base
        {
            return Err(integrity(
                "original changeset layers disagree with the accepted record",
            ));
        }
    }
    let operation = changeset.operations.first().ok_or_else(unavailable)?;
    let bytes = source
        .blobs()
        .read_capped(&operation.result, MAX_MANIFEST_BYTES)
        .map_err(|_| unavailable())?;
    let manifest = Manifest::parse(&bytes)?;
    let after = source.open_at(manifest.sequence())?;
    if after.info().manifest_digest() != operation.result
        || after.manifest().content().op().as_str() != operation.op_id.to_string()
    {
        return Err(integrity(
            "original changeset operation differs from source history",
        ));
    }
    Ok((after, Some(changeset)))
}

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
    /// Exact accepted revision whose complete delta is accounted for by this command.
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
/// Supported: a terminal personal command accounting for a `workspace-publish` delta, or a C01/C02
/// publication with exactly one verified source operation. The direct path uses
/// the source revision identity as its published identity and pins the source manifest as its
/// request fingerprint. The immediately preceding source layers must equal the immediately
/// preceding accepted layers; the resulting complete layer sets and footprints must also match.
/// C01 requires the canonical payload, its exact accepted operation/base/layers and the pinned
/// source result in actual history. This refuses any additional published delta/dependency,
/// rebase administration and missing/compacted proof. Earlier commands reverting to the exact prior
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
    let accepted_sequence = published
        .sequence_of(revision)?
        .filter(|s| *s > 1)
        .ok_or_else(unavailable)?;
    let accepted = published.open_at(accepted_sequence)?;
    let accepted_before = published.open_at(accepted_sequence - 1)?;
    let publication =
        CapturedFootprint::decode(&accepted.operation_evidence()?.ok_or_else(unavailable)?)
            .map_err(|_| unavailable())?;
    let (after, payload) = source_result(source, &accepted, &accepted_before, &publication, meta)?;
    let source_sequence = after.info().sequence();
    if source_sequence <= 1 {
        return Err(unavailable());
    }
    let before = source.open_at(source_sequence - 1)?;
    let s = after.manifest().content();
    let a = accepted.manifest().content();
    let b = before.manifest().content();
    let p = accepted_before.manifest().content();
    if accepted.info().revision() != revision
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
    if command.op_id.to_string() != s.op().as_str()
        || publication.op_id.to_string() != a.op().as_str()
        || publication.footprint != command.footprint
        || payload
            .as_ref()
            .is_some_and(|cs| cs.dependencies != command_evidence::dependencies(&command.footprint))
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
