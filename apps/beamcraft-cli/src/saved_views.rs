// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! W01 typed personal-view adapter. Storage never enters model publication or undo.
use crate::plate::{self, PlateError};
use beamcraft_host::{personal_views, revision::RevisionStore, storage::BlobStore};
use beamcraft_layer_engine::authored::personal_views::{self as layer, Document};
use beamcraft_model::{
    digest::ContentDigest,
    error::ErrorCode,
    id::{ElementId, RevisionId, WorkspaceId},
    saved_view::{Camera, SavedView},
};
use std::path::Path;
/// A validated personal document and its compare-and-swap version.
#[derive(Debug, Clone)]
pub struct Views {
    /// Typed cameras and opaque auxiliary view records.
    pub document: Document,
    /// Exact previous file digest; None means no file yet.
    pub version: Option<ContentDigest>,
}
fn host(e: personal_views::ViewsError) -> PlateError {
    PlateError::new(e.code, e.message)
}
/// Read views from a genuine personal workspace; a shared model store is refused.
pub fn read(dir: &Path) -> Result<Views, PlateError> {
    let meta = crate::workspace::read_meta(dir)?;
    let workspace = WorkspaceId::parse(meta.workspace_id.as_str())
        .map_err(|_| plate::integrity("invalid personal workspace identity"))?;
    let bytes = personal_views::read(dir).map_err(host)?;
    let document = match bytes.bytes {
        Some(b) => layer::decode(&b, workspace, &plate::control())?,
        None => Document {
            workspace,
            views: Vec::new(),
            records: Default::default(),
        },
    };
    beamcraft_engine::saved_view::validate(&document.views)?;
    Ok(Views {
        document,
        version: bytes.digest,
    })
}
/// Capture the current verified model pin, personal author and clock along with camera intent.
pub fn capture(
    dir: &Path,
    id: ElementId,
    name: String,
    camera: Camera,
    work_plane: [[f64; 3]; 3],
    comment: String,
) -> Result<SavedView, PlateError> {
    let meta = crate::workspace::read_meta(dir)?;
    let store = RevisionStore::new(
        BlobStore::new(dir)
            .map_err(|e| PlateError::new(e.code(), "workspace root is not usable"))?,
    );
    let head = store.open()?;
    let summary = plate::summarize(head.info());
    let saved_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .ok_or_else(|| {
            PlateError::new(ErrorCode::BackendUnavailable, "save time is unavailable")
        })?;
    let view = SavedView {
        id,
        name,
        camera,
        work_plane,
        comment,
        author: beamcraft_model::id::Uuid7::parse(meta.user_id.as_str())
            .map_err(|_| plate::integrity("invalid workspace author"))?,
        saved_ms,
        workspace: WorkspaceId::parse(meta.workspace_id.as_str())
            .map_err(|_| plate::integrity("invalid workspace identity"))?,
        source_revision: RevisionId::parse(&summary.revision_id)
            .map_err(|_| plate::integrity("invalid model revision identity"))?,
        source_manifest: summary.manifest_digest,
    };
    view.validate()?;
    Ok(view)
}
/// Save a complete admitted personal document iff the observed bytes still match.
pub fn write(dir: &Path, views: &Views) -> Result<ContentDigest, PlateError> {
    let meta = crate::workspace::read_meta(dir)?;
    if meta.workspace_id.as_str() != views.document.workspace.to_string() {
        return Err(PlateError::new(
            ErrorCode::PermissionDenied,
            "views belong to another workspace",
        ));
    }
    beamcraft_engine::saved_view::validate(&views.document.views)?;
    let bytes = layer::encode(&views.document)?;
    layer::decode(&bytes, views.document.workspace, &plate::control())?;
    personal_views::compare_and_swap(dir, views.version, &bytes).map_err(host)
}
/// Restore a camera with its current model anchors and explicit unresolved diagnostics.
pub fn restore(dir: &Path, id: ElementId) -> Result<crate::plane_views::Restored, PlateError> {
    let views = read(dir)?;
    let view = views
        .document
        .views
        .iter()
        .find(|v| v.id == id)
        .ok_or_else(|| PlateError::new(ErrorCode::NotFound, "saved view not found"))?;
    crate::plane_views::restore(dir, &views, view)
}
