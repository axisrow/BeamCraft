// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! W01 engine admission and restoring personal camera intent against a supplied model pin.
use beamcraft_model::{
    command::CommandError,
    digest::ContentDigest,
    error::ErrorCode,
    saved_view::{MAX_VIEWS, SavedView},
};
use std::collections::BTreeSet;
/// Check a complete bounded personal camera collection before saving or restoring it.
pub fn validate(views: &[SavedView]) -> Result<(), CommandError> {
    if views.len() > MAX_VIEWS {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "views",
            "too many personal views",
        ));
    }
    let mut ids = BTreeSet::new();
    for v in views {
        v.validate()?;
        if !ids.insert(v.id) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "views",
                "duplicate view UUID",
            ));
        }
    }
    Ok(())
}
/// Restore a camera without changing the supplied model; true means the model has changed.
pub fn restore(
    view: &SavedView,
    current: ContentDigest,
) -> Result<(SavedView, bool), CommandError> {
    view.validate()?;
    Ok((view.clone(), view.source_manifest != current))
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::{
        id::{ElementId, RevisionId, Uuid7, WorkspaceId},
        saved_view::Camera,
    };
    fn view() -> SavedView {
        let id = "019a7bf0-0000-7000-8000-000000b00001";
        SavedView {
            id: ElementId::parse(id).unwrap(),
            name: "Plan".into(),
            camera: Camera::default(),
            work_plane: [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
            author: Uuid7::parse(id).unwrap(),
            saved_ms: 0,
            comment: String::new(),
            workspace: WorkspaceId::parse(id).unwrap(),
            source_revision: RevisionId::parse(id).unwrap(),
            source_manifest: ContentDigest::from_bytes([1; 32]),
        }
    }
    #[test]
    fn restore_reports_model_changes_without_rewriting_the_saved_pin() {
        let v = view();
        let (same, changed) = restore(&v, ContentDigest::from_bytes([2; 32])).unwrap();
        assert!(changed);
        assert_eq!(same, v);
        assert!(!restore(&v, v.source_manifest).unwrap().1);
    }
    #[test]
    fn duplicate_cameras_and_invalid_work_planes_are_refused() {
        let mut v = view();
        assert!(validate(&[v.clone(), v.clone()]).is_err());
        v.work_plane[2] = [1., 0., 0.];
        assert!(validate(&[v]).is_err());
        let c = Camera {
            aspect: 0.,
            ..Camera::default()
        };
        assert!(c.validate().is_err());
    }
}
