// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V09 deterministic cameras normal to an admitted plane or vertical through two points.
use crate::work_plane::WorkPlane;
use beamcraft_model::{
    command::{CommandError, Placement},
    error::ErrorCode,
    saved_view::{Camera, Projection},
};
/// Source geometry for a plane view, kept independently of any model or renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaneSource {
    /// Explicit immutable world frame.
    Plane(WorkPlane),
    /// Vertical plane through these two world points.
    TwoPoints([f64; 3], [f64; 3]),
}
/// Complete plane-camera construction parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneView {
    /// Source plane.
    pub source: PlaneSource,
    /// Look from the opposite normal side while preserving the up axis.
    pub negative_side: bool,
    /// Full depth band around the construction plane, millimetres.
    pub depth_mm: f64,
    /// Horizontal orthographic span, millimetres.
    pub width_mm: f64,
    /// Horizontal/vertical aspect.
    pub aspect: f64,
}
impl PlaneView {
    /// Return a camera and its matching tool work plane; no model writes occur.
    pub fn resolve(self) -> Result<(Camera, WorkPlane), CommandError> {
        if !self.depth_mm.is_finite() || !(0.001..=1e9).contains(&self.depth_mm) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "depth",
                "view depth must be finite and positive within the coordinate cap",
            ));
        }
        let original = match self.source {
            PlaneSource::Plane(p) => p,
            PlaneSource::TwoPoints(a, b) => WorkPlane::vertical(a, b)?,
        };
        let p = if self.negative_side {
            WorkPlane::new(original.origin(), original.x().map(|v| -v), original.y())?
        } else {
            original
        };
        let [x, y, z] = p.to_world([0., 0., self.depth_mm])?;
        let camera = Camera {
            placement: Placement {
                translation_mm: [x, y, z],
                orientation_xyzw: p.placement()?.orientation_xyzw,
            },
            projection: Projection::Orthographic,
            width_mm: self.width_mm,
            aspect: self.aspect,
            fov_rad: std::f64::consts::FRAC_PI_3,
        };
        camera.validate()?;
        Ok((camera, p))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_sides_share_the_plane_and_face_opposite_directions() {
        let p = PlaneView {
            source: PlaneSource::TwoPoints([10., 20., 30.], [110., 20., 60.]),
            negative_side: false,
            depth_mm: 100.,
            width_mm: 1000.,
            aspect: 2.,
        };
        let (a, ap) = p.resolve().unwrap();
        let (b, bp) = PlaneView {
            negative_side: true,
            ..p
        }
        .resolve()
        .unwrap();
        assert_eq!(ap.origin(), bp.origin());
        assert_eq!(ap.y(), bp.y());
        for (a, b) in a
            .placement
            .translation_mm
            .into_iter()
            .zip(b.placement.translation_mm)
            .zip(ap.origin())
            .map(|((a, b), o)| (a + b, 2. * o))
        {
            assert!((a - b).abs() < 1e-10);
        }
    }
    #[test]
    fn degenerate_points_and_invalid_depth_are_refused() {
        let p = PlaneView {
            source: PlaneSource::TwoPoints([0.; 3], [0., 0., 100.]),
            negative_side: false,
            depth_mm: 100.,
            width_mm: 1000.,
            aspect: 1.,
        };
        assert!(p.resolve().is_err());
        assert!(
            PlaneView {
                source: PlaneSource::Plane(WorkPlane::default()),
                depth_mm: 0.,
                ..p
            }
            .resolve()
            .is_err()
        );
    }
}
