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
    /// Plan view anchored to a persistent level identity.
    Level(beamcraft_model::id::ElementId),
    /// Vertical view anchored to persistent grid and axis identities.
    GridLine {
        /// Persistent grid UUID.
        grid: beamcraft_model::id::ElementId,
        /// Persistent line UUID, independent of its current label.
        axis: beamcraft_model::id::ElementId,
    },
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
        self.resolve_with(&[], &crate::level::Levels::default())
    }
    /// Validate view properties independently of model availability.
    pub fn validate(self) -> Result<(), CommandError> {
        Camera {
            width_mm: self.width_mm,
            aspect: self.aspect,
            ..Camera::default()
        }
        .validate()?;
        if !self.depth_mm.is_finite() || !(0.001..=1e9).contains(&self.depth_mm) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "depth",
                "invalid view depth",
            ));
        }
        if let PlaneSource::TwoPoints(a, b) = self.source {
            WorkPlane::vertical(a, b)?;
        }
        Ok(())
    }
    /// Re-resolve model anchors against the caller's single verified model context.
    pub fn resolve_with(
        self,
        grids: &[beamcraft_model::grid::Grid],
        levels: &crate::level::Levels,
    ) -> Result<(Camera, WorkPlane), CommandError> {
        self.validate()?;
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
            PlaneSource::Level(id) => WorkPlane::parallel(
                "xy",
                levels
                    .level(&id)
                    .ok_or_else(|| {
                        CommandError::new(ErrorCode::NotFound, "level", "view level is unresolved")
                    })?
                    .elevation_mm,
            )?,
            PlaneSource::GridLine { grid, axis } => {
                let grid = grids.iter().find(|g| g.id == grid).ok_or_else(|| {
                    CommandError::new(ErrorCode::NotFound, "grid", "view grid is unresolved")
                })?;
                let line = crate::grid_query::segments(grid)?
                    .into_iter()
                    .find(|l| l.axis_id == axis)
                    .ok_or_else(|| {
                        CommandError::new(
                            ErrorCode::NotFound,
                            "axis",
                            "view grid axis is unresolved",
                        )
                    })?;
                let plane = crate::grid_query::grid_line_plane(grid, &line.label)?;
                let [x, y, _] = plane.normal_world;
                WorkPlane::new(plane.point_world_mm, [-y, x, 0.], [0., 0., 1.])?
            }
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

/// Finite world work-area box, independent of engineering bounds or quantities.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    /// Minimum world coordinates.
    pub min: [f64; 3],
    /// Maximum world coordinates.
    pub max: [f64; 3],
}
impl WorkArea {
    /// Check finite coordinates, model caps and ordered nonzero dimensions.
    pub fn validate(self) -> Result<(), CommandError> {
        Placement {
            translation_mm: self.min,
            ..Placement::IDENTITY
        }
        .validate()?;
        Placement {
            translation_mm: self.max,
            ..Placement::IDENTITY
        }
        .validate()?;
        if self.min.into_iter().zip(self.max).any(|(a, b)| a >= b) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "work_area",
                "work area needs strictly ordered bounds",
            ));
        }
        Ok(())
    }
}
/// Default named 3D camera, isometric about the world origin with the requested span/aspect.
pub fn three_d(width_mm: f64, aspect: f64) -> Result<Camera, CommandError> {
    let p = PlaneView {
        source: PlaneSource::Plane(WorkPlane::face([0.; 3], [1., -1., 1.])?),
        negative_side: false,
        depth_mm: 10000.,
        width_mm,
        aspect,
    };
    Ok(p.resolve()?.0)
}

#[cfg(test)]
mod named_tests {
    use super::*;
    #[test]
    fn a_level_view_follows_elevation_and_a_missing_level_refuses() {
        let id =
            beamcraft_model::id::ElementId::parse("019a7bf0-0000-7000-8000-000000c00003").unwrap();
        let mut levels = crate::level::Levels::default();
        levels.insert_level(
            id,
            crate::level::LevelRecord {
                name: "L".into(),
                elevation_mm: 3000.,
            },
        );
        let p = PlaneView {
            source: PlaneSource::Level(id),
            negative_side: false,
            depth_mm: 1000.,
            width_mm: 1000.,
            aspect: 1.,
        };
        assert_eq!(
            p.resolve_with(&[], &levels).unwrap().1.origin(),
            [0., 0., 3000.]
        );
        levels.insert_level(
            id,
            crate::level::LevelRecord {
                name: "L".into(),
                elevation_mm: 6000.,
            },
        );
        assert_eq!(
            p.resolve_with(&[], &levels).unwrap().1.origin(),
            [0., 0., 6000.]
        );
        assert!(p.resolve().is_err());
        three_d(1000., 1.).unwrap().validate().unwrap();
        assert!(
            WorkArea {
                min: [0.; 3],
                max: [0.; 3]
            }
            .validate()
            .is_err()
        );
    }
}
