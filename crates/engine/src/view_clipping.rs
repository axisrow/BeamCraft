// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V15 non-destructive cut plane: per-view clip planes and a work-area box.
//!
//! A cut is a read-time display filter over a [`ViewSnapshot`], never model data: the record
//! holds half-spaces, the snapshot and its token are untouched, and quantities never read it.
//! The conservative element mask keeps every body that partially intersects the visible side;
//! real triangle clipping happens at the display edge.
use crate::query::snapshot::ViewSnapshot;
use beamcraft_model::{
    command::{CommandError, Placement},
    error::ErrorCode,
    id::ElementId,
};
/// Most user-supplied clip planes in one cut record, before work-area expansion.
pub const MAX_PLANES: usize = 8;
/// Most planes [`Clipping::effective`] may return (user planes plus the six box faces).
pub const MAX_EFFECTIVE: usize = 16;
/// The visible half-space is `normal · (world point - point_mm) >= 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPlane {
    /// A finite point on the plane, millimetres.
    pub point_mm: [f64; 3],
    /// A finite nonzero normal, normalized on admission, into the visible half-space.
    pub normal: [f64; 3],
}
impl ClipPlane {
    /// Admits and normalizes a caller's plane; rejects non-finite input and a near-zero normal.
    pub fn new(point_mm: [f64; 3], normal: [f64; 3]) -> Result<Self, CommandError> {
        Placement {
            translation_mm: point_mm,
            ..Placement::IDENTITY
        }
        .validate()?;
        if normal.iter().any(|n| !n.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "cut_plane.normal",
                "cut normal must be finite",
            ));
        }
        let [x, y, z] = normal;
        let len = x.hypot(y).hypot(z);
        if !(len.is_finite() && len >= 1e-12) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "cut_plane.normal",
                "cut normal must be nonzero",
            ));
        }
        Ok(Self {
            point_mm,
            normal: normal.map(|v| v / len),
        })
    }
    /// Signed distance of a world point into the visible half-space.
    pub fn distance(self, p: [f64; 3]) -> f64 {
        p.into_iter()
            .zip(self.point_mm)
            .zip(self.normal)
            .map(|((p, o), n)| (p - o) * n)
            .sum()
    }
}
/// A closed world-space box, millimetres, that bounds the visible part of a view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    /// Smallest corner.
    pub min: [f64; 3],
    /// Largest corner.
    pub max: [f64; 3],
}
impl WorkArea {
    /// Admits a finite box with `min <= max` per axis inside the coordinate cap.
    pub fn new(min: [f64; 3], max: [f64; 3]) -> Result<Self, CommandError> {
        for (lo, hi) in min.into_iter().zip(max) {
            for v in [lo, hi] {
                if !v.is_finite() {
                    return Err(CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "work_area",
                        "work-area bound must be finite",
                    ));
                }
                if v.abs() > beamcraft_geom::tolerance::COORDINATE_CAP_MM {
                    return Err(CommandError::new(
                        ErrorCode::LimitExceeded,
                        "work_area",
                        "work-area bound exceeds the coordinate cap",
                    ));
                }
            }
            if lo > hi {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "work_area",
                    "work-area min exceeds max",
                ));
            }
        }
        Ok(Self { min, max })
    }
    /// The box's six inward half-spaces: +X, -X, +Y, -Y, +Z, -Z.
    pub fn planes(self) -> Result<Vec<ClipPlane>, CommandError> {
        let unit = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut planes = Vec::with_capacity(6);
        // The axis unit vector scaled by the bound is the face point; negating the normal
        // turns the far face into an inward half-space too.
        for ((n, lo), hi) in unit.into_iter().zip(self.min).zip(self.max) {
            planes.push(ClipPlane::new(n.map(|v| v * lo), n)?);
            planes.push(ClipPlane::new(n.map(|v| v * hi), n.map(|v| -v))?);
        }
        Ok(planes)
    }
}
/// One cut setting: user clip planes plus an optional work-area box.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clipping {
    /// User planes; an empty list means none.
    pub planes: Vec<ClipPlane>,
    /// The optional work-area box.
    pub area: Option<WorkArea>,
}
impl Clipping {
    /// Admits the whole record before it is applied or displayed.
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.planes.len() > MAX_PLANES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "cut_plane.planes",
                "at most eight clip planes are allowed",
            ));
        }
        for plane in &self.planes {
            ClipPlane::new(plane.point_mm, plane.normal)?;
        }
        if let Some(area) = self.area {
            WorkArea::new(area.min, area.max)?;
        }
        Ok(())
    }
    /// The effective half-spaces: the user planes first, then the box's six faces, at most
    /// [`MAX_EFFECTIVE`].
    pub fn effective(&self) -> Result<Vec<ClipPlane>, CommandError> {
        self.validate()?;
        let mut planes = self.planes.clone();
        if let Some(area) = self.area {
            planes.extend(area.planes()?);
        }
        if planes.len() > MAX_EFFECTIVE {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "cut_plane.planes",
                "the effective plane budget is exceeded",
            ));
        }
        Ok(planes)
    }
}
/// The conservative display mask: a body stays when any part of its world bounds may be
/// visible, so partially intersecting bodies keep their original meshes and only bodies
/// provably entirely outside every plane are hidden.
///
/// The snapshot is only read; its token, bounds and entries are unchanged.
pub fn visible_elements(
    snapshot: &ViewSnapshot,
    planes: &[ClipPlane],
) -> Result<Vec<ElementId>, CommandError> {
    if planes.len() > MAX_EFFECTIVE {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "cut_plane.planes",
            "the effective plane budget is exceeded",
        ));
    }
    for plane in planes {
        ClipPlane::new(plane.point_mm, plane.normal)?;
    }
    let mut ids = Vec::new();
    for entry in snapshot.elements() {
        let bounds = entry.full.world_bounds;
        // The best case for hiding is the bounds corner deepest into the visible side; a
        // relative epsilon keeps marginal bodies visible rather than flickering out.
        let outside = planes.iter().any(|p| {
            let corner = [
                if p.normal[0] >= 0.0 {
                    bounds.min[0]
                } else {
                    bounds.max[0]
                },
                if p.normal[1] >= 0.0 {
                    bounds.min[1]
                } else {
                    bounds.max[1]
                },
                if p.normal[2] >= 0.0 {
                    bounds.min[2]
                } else {
                    bounds.max[2]
                },
            ];
            let scale = corner
                .into_iter()
                .chain(p.point_mm)
                .map(|v| v.abs())
                .sum::<f64>()
                + 1.0;
            p.distance(corner) < -64.0 * f64::EPSILON * scale
        });
        if !outside {
            ids.push(entry.id);
        }
    }
    Ok(ids)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::plate_scene;
    use crate::{Limits, ViewPin};
    use beamcraft_model::digest::Sha256;
    fn snapshot() -> ViewSnapshot {
        let pin = ViewPin::new(Sha256::digest(b"beamcraft-synthetic-scene").unwrap(), 1);
        ViewSnapshot::build(pin, plate_scene(6, false).unwrap(), &Limits::default()).unwrap()
    }
    #[test]
    fn planes_admit_only_finite_input_and_normalize() {
        let plane = ClipPlane::new([0.0, 0.0, 1.0], [0.0, 0.0, 2.0]).unwrap();
        assert_eq!(plane.normal, [0.0, 0.0, 1.0]);
        assert!(plane.distance([0.0, 0.0, 5.0]) == 4.0);
        assert!(ClipPlane::new([0.0; 3], [0.0; 3]).is_err());
        assert!(ClipPlane::new([f64::NAN; 3], [0.0, 0.0, 1.0]).is_err());
        assert!(ClipPlane::new([0.0; 3], [f64::INFINITY, 0.0, 0.0]).is_err());
        assert!(ClipPlane::new([1e12; 3], [0.0, 0.0, 1.0]).is_err());
    }
    #[test]
    fn the_work_area_expands_into_six_half_spaces() {
        let area = WorkArea::new([-100.0; 3], [100.0; 3]).unwrap();
        let planes = area.planes().unwrap();
        assert_eq!(planes.len(), 6);
        let inside = [0.0; 3];
        let outside = [150.0, 0.0, 0.0];
        assert!(planes.iter().all(|p| p.distance(inside) >= 0.0));
        assert!(planes.iter().any(|p| p.distance(outside) < 0.0));
        assert!(WorkArea::new([100.0; 3], [-100.0; 3]).is_err());
        assert!(WorkArea::new([f64::NAN; 3], [100.0; 3]).is_err());
    }
    #[test]
    fn the_record_validates_bounds_and_expands_to_the_budget() {
        let mut clip = Clipping {
            planes: Vec::new(),
            area: Some(WorkArea::new([-1.0; 3], [1.0; 3]).unwrap()),
        };
        assert_eq!(clip.effective().unwrap().len(), 6);
        clip.planes = (0..MAX_PLANES)
            .map(|i| {
                let x = i as f64 * 1000.0;
                ClipPlane::new([x, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap()
            })
            .collect();
        assert_eq!(clip.effective().unwrap().len(), MAX_PLANES + 6);
        clip.planes
            .push(ClipPlane::new([0.0; 3], [1.0, 0.0, 0.0]).unwrap());
        assert!(clip.effective().is_err());
        assert!(Clipping::default().effective().unwrap().is_empty());
    }
    #[test]
    fn the_mask_hides_only_bodies_entirely_outside_and_never_touches_the_snapshot() {
        let snapshot = snapshot();
        let token = snapshot.token().as_str().to_owned();
        let bounds = snapshot.engineering_bounds().unwrap();
        // Everything visible without a cut.
        let all = visible_elements(&snapshot, &[]).unwrap();
        assert_eq!(all.len(), snapshot.len());
        // A plane left of every body hides nothing; one right of everything hides it all.
        let right = bounds.max[0] + 1000.0;
        let none = visible_elements(
            &snapshot,
            &[ClipPlane::new([right, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap()],
        )
        .unwrap();
        assert!(none.is_empty());
        let some = visible_elements(
            &snapshot,
            &[ClipPlane::new([68.0, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap()],
        )
        .unwrap();
        assert!(some.len() < all.len() && !some.is_empty());
        // Every hidden body lies entirely behind the plane; every kept one still reaches it.
        let hidden: Vec<_> = all
            .iter()
            .filter(|id| !some.contains(id))
            .collect::<Vec<_>>();
        for id in &hidden {
            let e = snapshot.element(id).unwrap();
            assert!(e.full.world_bounds.max[0] < 68.0);
        }
        for id in &some {
            let e = snapshot.element(id).unwrap();
            assert!(e.full.world_bounds.max[0] > 67.0);
        }
        // The cut is display state: the snapshot is byte-identical after masking.
        assert_eq!(snapshot.token().as_str(), token);
        assert_eq!(snapshot.engineering_bounds().unwrap(), bounds);
    }
}
