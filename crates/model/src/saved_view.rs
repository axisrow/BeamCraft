// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! W01 personal camera intent, independent of model revisions and numbering.
use crate::{
    command::{CommandError, Placement},
    digest::ContentDigest,
    error::ErrorCode,
    id::{ElementId, RevisionId, Uuid7, WorkspaceId},
};
/// Maximum personal cameras in one workspace.
pub const MAX_VIEWS: usize = 128;
/// Native camera projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// Parallel rays.
    Orthographic,
    /// Perspective rays.
    Perspective,
}
impl Projection {
    /// USD projection token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Orthographic => "orthographic",
            Self::Perspective => "perspective",
        }
    }
    /// Parse a closed projection token.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "orthographic" => Some(Self::Orthographic),
            "perspective" => Some(Self::Perspective),
            _ => None,
        }
    }
}
/// A camera looks along local -Z, with local +Y up. Dimensions are millimetres/radians.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    /// Camera position and proper orientation.
    pub placement: Placement,
    /// Parallel or perspective.
    pub projection: Projection,
    /// Orthographic horizontal span.
    pub width_mm: f64,
    /// Horizontal/vertical aspect.
    pub aspect: f64,
    /// Horizontal perspective field of view, radians.
    pub fov_rad: f64,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            placement: Placement {
                translation_mm: [0., 0., 10000.],
                ..Placement::IDENTITY
            },
            projection: Projection::Orthographic,
            width_mm: 10000.,
            aspect: 1.,
            fov_rad: std::f64::consts::FRAC_PI_3,
        }
    }
}
/// One personally saved view; its source pin is provenance, never a model dependency.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedView {
    /// Stable UUID.
    pub id: ElementId,
    /// User label.
    pub name: String,
    /// Camera state.
    pub camera: Camera,
    /// Work-plane origin, unit X and unit Y, in world coordinates.
    pub work_plane: [[f64; 3]; 3],
    /// Author label, not an authentication claim.
    pub author: Uuid7,
    /// Last save time in Unix milliseconds.
    pub saved_ms: u64,
    /// User comment.
    pub comment: String,
    /// Personal workspace identity.
    pub workspace: WorkspaceId,
    /// Source model revision UUID.
    pub source_revision: RevisionId,
    /// Source manifest digest.
    pub source_manifest: ContentDigest,
}
fn invalid() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidArgument,
        "view",
        "view has invalid labels, camera or work-plane parameters",
    )
}
impl Camera {
    /// Admit finite proper camera parameters, before serialization or projection.
    pub fn validate(&self) -> Result<(), CommandError> {
        self.placement.validate()?;
        if !self.width_mm.is_finite()
            || !(0.001..=1e9).contains(&self.width_mm)
            || !self.aspect.is_finite()
            || !(0.001..=1000.).contains(&self.aspect)
            || !self.fov_rad.is_finite()
            || !(0.001..std::f64::consts::PI - 0.001).contains(&self.fov_rad)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
impl SavedView {
    /// Admission for untrusted saved state; all allocations have outer document caps.
    pub fn validate(&self) -> Result<(), CommandError> {
        self.camera.validate()?;
        if self.name.trim().is_empty()
            || self.name.len() > 256
            || self.name.chars().any(char::is_control)
            || self.comment.len() > 4096
            || self
                .comment
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(invalid());
        }
        let [origin, x, y] = self.work_plane;
        Placement {
            translation_mm: origin,
            ..Placement::IDENTITY
        }
        .validate()?;
        if x.iter().chain(y.iter()).any(|v| !v.is_finite()) {
            return Err(invalid());
        }
        let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
        if (dot(x, x) - 1.).abs() > 1e-10
            || (dot(y, y) - 1.).abs() > 1e-10
            || dot(x, y).abs() > 1e-10
        {
            return Err(invalid());
        }
        Ok(())
    }
}
