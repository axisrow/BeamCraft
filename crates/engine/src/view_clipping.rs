// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V15 non-destructive half-space clipping and a conservative read-time element mask.
use crate::{plane_view::WorkArea, query::snapshot::ViewSnapshot};
use beamcraft_model::{
    command::{CommandError, Placement},
    error::ErrorCode,
    id::ElementId,
};
/// Maximum user-supplied clipping planes, before work-area/depth expansion.
pub const MAX_PLANES: usize = 8;
/// The visible half-space is normal dot (world point - point_mm) >= 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPlane {
    /// A finite point on the plane, millimetres.
    pub point_mm: [f64; 3],
    /// A finite admitted unit normal into the visible half-space.
    pub normal: [f64; 3],
}
impl ClipPlane {
    /// Admit and normalize a caller's plane.
    pub fn new(point_mm: [f64; 3], normal: [f64; 3]) -> Result<Self, CommandError> {
        Placement {
            translation_mm: point_mm,
            ..Placement::IDENTITY
        }
        .validate()?;
        if normal.iter().any(|n| !n.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "normal",
                "clip normal must be finite",
            ));
        }
        let [x, y, z] = normal;
        let n = x.hypot(y).hypot(z);
        if !n.is_finite() || n < 1e-12 {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "clip_plane",
                "clip normal must be finite and nonzero",
            ));
        }
        Ok(Self {
            point_mm,
            normal: normal.map(|v| v / n),
        })
    }
    /// Conservative signed distance, used only for display.
    pub fn distance(self, p: [f64; 3]) -> f64 {
        p.into_iter()
            .zip(self.point_mm)
            .zip(self.normal)
            .map(|((p, o), n)| (p - o) * n)
            .sum()
    }
}
/// One bounded explicit clip setting, stored in personal view state.
#[derive(Debug, Clone, PartialEq)]
pub struct Clipping {
    /// User planes; omitted planes mean none.
    pub planes: Vec<ClipPlane>,
    /// Use a constructed view's saved depth band.
    pub depth_enabled: bool,
}
impl Default for Clipping {
    fn default() -> Self {
        Self {
            planes: Vec::new(),
            depth_enabled: true,
        }
    }
}
impl Clipping {
    /// Admit all planes before publication or a display query.
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.planes.len() > MAX_PLANES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "planes",
                "at most eight explicit clip planes are allowed",
            ));
        }
        for p in &self.planes {
            let normalized = ClipPlane::new(p.point_mm, p.normal)?;
            if normalized
                .normal
                .into_iter()
                .zip(p.normal)
                .any(|(a, b)| (a - b).abs() > 1e-12)
            {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "normal",
                    "clip normal is not unit length",
                ));
            }
        }
        Ok(())
    }
    /// Expand custom planes, a world box and the optional construction depth into <=16 planes.
    pub fn effective(
        &self,
        area: Option<WorkArea>,
        depth: Option<([[f64; 3]; 3], f64)>,
    ) -> Result<Vec<ClipPlane>, CommandError> {
        self.validate()?;
        let mut planes = self.planes.clone();
        if let Some(area) = area {
            area.validate()?;
            for (axis, (min, max)) in area.min.into_iter().zip(area.max).enumerate() {
                let mut normal = [0.; 3];
                if let Some(n) = normal.get_mut(axis) {
                    *n = 1.;
                }
                let mut lo = [0.; 3];
                let mut hi = [0.; 3];
                if let (Some(a), Some(b)) = (lo.get_mut(axis), hi.get_mut(axis)) {
                    *a = min;
                    *b = max;
                }
                planes.push(ClipPlane::new(lo, normal)?);
                planes.push(ClipPlane::new(hi, normal.map(|v| -v))?);
            }
        }
        if self.depth_enabled
            && let Some(([origin, x, y], depth)) = depth
        {
            if !depth.is_finite() || depth <= 0. || depth > 1e9 {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "depth",
                    "invalid display depth",
                ));
            }
            let n = crate::work_plane::WorkPlane::new(origin, x, y)?.normal();
            let mut a = origin;
            let mut b = origin;
            for ((a, b), n) in a.iter_mut().zip(b.iter_mut()).zip(n) {
                *a -= n * depth / 2.;
                *b += n * depth / 2.;
            }
            planes.push(ClipPlane::new(a, n)?);
            planes.push(ClipPlane::new(b, n.map(|v| -v))?);
        }
        Ok(planes)
    }
}
/// A conservative display mask: partially intersecting bodies remain, with their original meshes.
/// Planes are also returned to the consumer for actual triangle clipping; quantities never read it.
pub fn visible_elements(
    snapshot: &ViewSnapshot,
    planes: &[ClipPlane],
) -> Result<Vec<ElementId>, CommandError> {
    if planes.len() > 16 {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "planes",
            "effective plane budget exceeded",
        ));
    }
    for p in planes {
        let normalized = ClipPlane::new(p.point_mm, p.normal)?;
        if normalized
            .normal
            .into_iter()
            .zip(p.normal)
            .any(|(a, b)| (a - b).abs() > 1e-12)
        {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "normal",
                "effective clip normal is not unit length",
            ));
        }
    }
    let mut ids = Vec::new();
    for e in snapshot.elements() {
        let b = e.full.world_bounds;
        let outside = planes.iter().any(|p| {
            let mut corner = b.min;
            for ((c, max), n) in corner.iter_mut().zip(b.max).zip(p.normal) {
                if n >= 0. {
                    *c = max;
                }
            }
            let bound = 64.
                * f64::EPSILON
                * (corner
                    .iter()
                    .chain(p.point_mm.iter())
                    .map(|v| v.abs())
                    .sum::<f64>()
                    + 1.);
            p.distance(corner) < -bound
        });
        if !outside {
            ids.push(e.id);
        }
    }
    Ok(ids)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn box_and_depth_are_half_spaces_without_geometry_mutation() {
        let c = Clipping::default();
        let area = WorkArea {
            min: [-1.; 3],
            max: [1.; 3],
        };
        let p = c
            .effective(
                Some(area),
                Some(([[0.; 3], [1., 0., 0.], [0., 1., 0.]], 1.)),
            )
            .unwrap();
        assert_eq!(p.len(), 8);
        assert!(p.iter().all(|p| p.distance([0.; 3]) >= 0.));
        assert!(p.iter().any(|p| p.distance([0., 0., 2.]) < 0.));
        assert!(ClipPlane::new([0.; 3], [0.; 3]).is_err());
    }
}
