// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 admission and closest-point snaps on authored construction geometry.
use crate::footprint::Footprint;
use beamcraft_model::{
    command::CommandError,
    error::ErrorCode,
    helper::{Helper, HelperGeometry},
    id::ElementId,
};
use std::collections::{BTreeMap, BTreeSet};

/// All helper records and used identities of one pinned model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Helpers {
    records: BTreeMap<ElementId, Helper>,
    used: BTreeSet<ElementId>,
}
impl Helpers {
    /// Records an identity, including non-helper elements.
    pub fn insert_other(&mut self, id: ElementId) {
        self.used.insert(id);
    }
    /// Records a validated helper.
    pub fn insert(&mut self, helper: Helper) -> Result<(), CommandError> {
        helper.validate()?;
        if self.contains(&helper.id) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "helper",
                "duplicate helper identity",
            ));
        }
        if self.records.len() >= beamcraft_model::helper::MAX_HELPERS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "helper",
                "too many helpers",
            ));
        }
        self.used.insert(helper.id);
        self.records.insert(helper.id, helper);
        Ok(())
    }
    /// Whether any element has the supplied identity.
    pub fn contains(&self, id: &ElementId) -> bool {
        self.used.contains(id)
    }
    /// A helper by stable identity.
    pub fn helper(&self, id: &ElementId) -> Option<&Helper> {
        self.records.get(id)
    }
    /// Helpers in stable identity order.
    pub fn helpers(&self) -> impl Iterator<Item = &Helper> {
        self.records.values()
    }
    /// Admits creation without modifying the model.
    pub fn create(&self, helper: &Helper) -> Result<Footprint, CommandError> {
        helper.validate()?;
        if self.contains(&helper.id) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "new_id",
                "the identity is already in use",
            ));
        }
        if self.records.len() >= beamcraft_model::helper::MAX_HELPERS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "helper",
                "too many helpers",
            ));
        }
        Footprint::for_create(helper.id, None)
    }
    /// Finds the nearest helper snap within a finite world-distance tolerance. Ties use identity
    /// order. A plane projects orthogonally; a line is a finite segment; a circle uses its exact
    /// circumference and center. Nothing is tessellated for this query.
    pub fn snap(
        &self,
        point: [f64; 3],
        tolerance_mm: f64,
    ) -> Result<Option<HelperSnap>, CommandError> {
        HelperGeometry::Point(point).validate()?;
        if !tolerance_mm.is_finite() || !(0.0..=1e6).contains(&tolerance_mm) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "tolerance",
                "invalid helper snap tolerance",
            ));
        }
        let mut best: Option<HelperSnap> = None;
        for helper in self.helpers() {
            for (role, target) in candidates(&helper.geometry, point) {
                if HelperGeometry::Point(target).validate().is_err() {
                    continue;
                }
                let distance_mm = norm(sub(target, point));
                if distance_mm <= tolerance_mm
                    && best.as_ref().is_none_or(|b| distance_mm < b.distance_mm)
                {
                    best = Some(HelperSnap {
                        id: helper.id,
                        role,
                        point_mm: target,
                        distance_mm,
                    });
                }
            }
        }
        Ok(best)
    }
}
/// The nearest snap on one helper. Invalid geometry and query inputs are refused.
pub fn snap_helper(
    helper: &Helper,
    point: [f64; 3],
    tolerance_mm: f64,
) -> Result<Option<HelperSnap>, CommandError> {
    let mut helpers = Helpers::default();
    helpers.insert(helper.clone())?;
    helpers.snap(point, tolerance_mm)
}

/// A qualified construction snap, never a solid or quantity result.
#[derive(Debug, Clone, PartialEq)]
pub struct HelperSnap {
    /// Owning helper.
    pub id: ElementId,
    /// Stable semantic role within that helper.
    pub role: &'static str,
    /// World-space snap point.
    pub point_mm: [f64; 3],
    /// Distance from the supplied point.
    pub distance_mm: f64,
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a.get(i).copied().unwrap_or(0.0) - b.get(i).copied().unwrap_or(0.0))
}
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a.get(i).copied().unwrap_or(0.0) + b.get(i).copied().unwrap_or(0.0))
}
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|v| v * s)
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
fn candidates(g: &HelperGeometry, p: [f64; 3]) -> Vec<(&'static str, [f64; 3])> {
    match *g {
        HelperGeometry::Point(a) => vec![("point", a)],
        HelperGeometry::Line(a, b) => {
            let d = sub(b, a);
            let t = (dot(sub(p, a), d) / dot(d, d)).clamp(0.0, 1.0);
            vec![
                ("start", a),
                ("end", b),
                ("midpoint", add(a, scale(d, 0.5))),
                ("nearest", add(a, scale(d, t))),
            ]
        }
        HelperGeometry::Plane { origin, normal } => {
            let n = scale(normal, 1.0 / norm(normal));
            vec![("plane", sub(p, scale(n, dot(sub(p, origin), n))))]
        }
        HelperGeometry::Circle {
            center,
            normal,
            radius_mm,
        } => {
            let n = scale(normal, 1.0 / norm(normal));
            let v = sub(p, center);
            let radial = sub(v, scale(n, dot(v, n)));
            let length = norm(radial);
            let mut out = vec![("center", center)];
            if length > 1e-12 {
                out.push((
                    "circumference",
                    add(center, scale(radial, radius_mm / length)),
                ));
            }
            out
        }
    }
}

/// Bounded world point parser shared by the headless and native helper tools.
pub fn parse_point(text: &str) -> Result<[f64; 3], CommandError> {
    let invalid = || {
        CommandError::new(
            ErrorCode::InvalidArgument,
            "point",
            "expected three bounded coordinates",
        )
    };
    if text.len() > 128 {
        return Err(invalid());
    }
    let mut parts = text.split(',');
    let mut point = [0.0; 3];
    for slot in &mut point {
        *slot = parts
            .next()
            .ok_or_else(invalid)?
            .trim()
            .parse()
            .map_err(|_| invalid())?;
    }
    if parts.next().is_some() {
        return Err(invalid());
    }
    HelperGeometry::Point(point).validate()?;
    Ok(point)
}

/// Bounded display polylines, independent of exact snap calculations. A plane is shown by a
/// 1000 mm square marker; its snap surface remains infinite. Circles use 64 display segments.
pub fn display_guides(helper: &Helper) -> Result<Vec<Vec<[f64; 3]>>, CommandError> {
    helper.validate()?;
    Ok(match helper.geometry {
        HelperGeometry::Point(p) => vec![vec![p]],
        HelperGeometry::Line(a, b) => vec![vec![a, b]],
        HelperGeometry::Circle {
            center,
            normal,
            radius_mm,
        } => {
            let (x, y) = basis(normal);
            let points = (0..=64)
                .map(|i| {
                    let angle = f64::from(i) * std::f64::consts::TAU / 64.0;
                    add(
                        center,
                        add(
                            scale(x, radius_mm * angle.cos()),
                            scale(y, radius_mm * angle.sin()),
                        ),
                    )
                })
                .collect();
            vec![vec![center], points]
        }
        HelperGeometry::Plane { origin, normal } => {
            let (x, y) = basis(normal);
            vec![
                [
                    [-1.0, -1.0],
                    [1.0, -1.0],
                    [1.0, 1.0],
                    [-1.0, 1.0],
                    [-1.0, -1.0],
                ]
                .into_iter()
                .map(|[a, b]| add(origin, add(scale(x, a * 500.0), scale(y, b * 500.0))))
                .collect(),
            ]
        }
    })
}
fn basis(normal: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let n = scale(normal, 1.0 / norm(normal));
    let seed = if n.first().is_some_and(|x| x.abs() < 0.9) {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let x = sub(seed, scale(n, dot(seed, n)));
    let x = scale(x, 1.0 / norm(x));
    let [nx, ny, nz] = n;
    let [xx, xy, xz] = x;
    (x, [ny * xz - nz * xy, nz * xx - nx * xz, nx * xy - ny * xx])
}
