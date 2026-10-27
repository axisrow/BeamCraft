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

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
    }

    fn helper(n: u32, geometry: HelperGeometry) -> Helper {
        Helper {
            id: id(n),
            name: "helper".to_owned(),
            geometry,
        }
    }

    #[test]
    fn a_segment_snaps_to_start_end_midpoint_and_clamped_nearest() {
        let mut helpers = Helpers::default();
        helpers
            .insert(helper(1, HelperGeometry::Line([0.; 3], [100., 0., 0.])))
            .unwrap();
        // Near the far end the projection beats the endpoint and the midpoint.
        let snap = helpers.snap([90., 4., 0.], 10.).unwrap().unwrap();
        assert_eq!(snap.role, "nearest");
        assert_eq!(snap.point_mm, [90., 0., 0.]);
        assert!((snap.distance_mm - 4.).abs() < 1e-12);
        // At the midpoint the midpoint role is recorded first and wins the tie.
        let snap = helpers.snap([50., 3., 4.], 10.).unwrap().unwrap();
        assert_eq!(snap.role, "midpoint");
        assert_eq!(snap.point_mm, [50., 0., 0.]);
        // Past the ends the projection clamps to the endpoints.
        let snap = helpers.snap([-50., 0., 0.], 1e3).unwrap().unwrap();
        assert_eq!(snap.role, "start");
        assert_eq!(snap.point_mm, [0., 0., 0.]);
    }

    #[test]
    fn a_plane_projects_onto_its_normalized_frame() {
        let mut helpers = Helpers::default();
        // A non-unit normal must not scale the projection distance.
        helpers
            .insert(helper(
                1,
                HelperGeometry::Plane {
                    origin: [0.; 3],
                    normal: [0., 0., 2.],
                },
            ))
            .unwrap();
        let snap = helpers.snap([30., 40., 500.], 1e3).unwrap().unwrap();
        assert_eq!(snap.role, "plane");
        assert_eq!(snap.point_mm, [30., 40., 0.]);
        assert!((snap.distance_mm - 500.).abs() < 1e-9);
        // An offset, tilted plane: the projection stays orthogonal to the unit normal.
        let mut helpers = Helpers::default();
        helpers
            .insert(helper(
                1,
                HelperGeometry::Plane {
                    origin: [10., 0., 0.],
                    normal: [1., 1., 0.],
                },
            ))
            .unwrap();
        let snap = helpers.snap([10., 7., 7.], 1e3).unwrap().unwrap();
        for (a, b) in snap.point_mm.into_iter().zip([6.5, 3.5, 7.]) {
            assert!((a - b).abs() < 1e-12);
        }
        assert!((snap.distance_mm - 7.0 / std::f64::consts::SQRT_2).abs() < 1e-12);
    }

    #[test]
    fn a_circle_snaps_to_center_and_exact_circumference() {
        let mut helpers = Helpers::default();
        helpers
            .insert(helper(
                1,
                HelperGeometry::Circle {
                    center: [0.; 3],
                    normal: [0., 0., 3.],
                    radius_mm: 100.,
                },
            ))
            .unwrap();
        // Off axis the exact circumference point beats the center.
        let snap = helpers.snap([200., 0., 0.], 250.).unwrap().unwrap();
        assert_eq!(snap.role, "circumference");
        assert_eq!(snap.point_mm, [100., 0., 0.]);
        assert!((snap.distance_mm - 100.).abs() < 1e-12);
        // On the axis every circumference point is equally far, so only the center qualifies.
        let snap = helpers.snap([0., 0., 50.], 100.).unwrap().unwrap();
        assert_eq!(snap.role, "center");
        assert_eq!(snap.point_mm, [0., 0., 0.]);
    }

    #[test]
    fn equal_distance_ties_are_broken_by_identity() {
        let mut helpers = Helpers::default();
        // The larger identity is inserted first and must still lose the tie.
        helpers
            .insert(helper(2, HelperGeometry::Point([50.; 3])))
            .unwrap();
        helpers
            .insert(helper(1, HelperGeometry::Point([50.; 3])))
            .unwrap();
        let snap = helpers.snap([50.; 3], 1.).unwrap().unwrap();
        assert_eq!(snap.id, id(1));
    }

    #[test]
    fn snap_refuses_unbounded_queries_and_tolerances() {
        let mut helpers = Helpers::default();
        helpers
            .insert(helper(1, HelperGeometry::Point([0.; 3])))
            .unwrap();
        for point in [[f64::NAN; 3], [f64::INFINITY, 0., 0.], [2e9, 0., 0.]] {
            assert!(helpers.snap(point, 1.).is_err(), "{point:?}");
        }
        for tolerance in [f64::NAN, f64::INFINITY, -1., 2e6] {
            assert!(helpers.snap([0.; 3], tolerance).is_err(), "{tolerance}");
        }
        // Zero tolerance is valid: a hit is exact, a miss is none.
        assert!(matches!(helpers.snap([0.; 3], 0.), Ok(Some(_))));
        assert!(matches!(helpers.snap([1.; 3], 0.), Ok(None)));
    }

    #[test]
    fn degenerate_and_unbounded_geometry_is_refused_everywhere() {
        let cases = [
            HelperGeometry::Line([0.; 3], [0.; 3]),
            HelperGeometry::Line([0.; 3], [2e6; 3]),
            HelperGeometry::Point([f64::NAN; 3]),
            HelperGeometry::Point([1e10, 0., 0.]),
            HelperGeometry::Circle {
                center: [0.; 3],
                normal: [0.; 3],
                radius_mm: 100.,
            },
            HelperGeometry::Circle {
                center: [0.; 3],
                normal: [0., 0., 1.],
                radius_mm: 0.,
            },
            HelperGeometry::Plane {
                origin: [0.; 3],
                normal: [0.; 3],
            },
        ];
        for geometry in cases {
            let h = helper(1, geometry);
            assert!(h.validate().is_err(), "{:?}", h.geometry);
            assert!(snap_helper(&h, [0.; 3], 1.).is_err());
        }
    }

    #[test]
    fn helper_geometry_round_trips_through_its_parameters() {
        let cases = [
            HelperGeometry::Point([1., -2., 3.]),
            HelperGeometry::Line([0.; 3], [100., 0., 0.]),
            HelperGeometry::Circle {
                center: [10., 20., 30.],
                normal: [0., 0., 2.],
                radius_mm: 50.,
            },
            HelperGeometry::Plane {
                origin: [10., 20., 30.],
                normal: [1., 1., 0.],
            },
        ];
        for geometry in cases {
            let reread =
                HelperGeometry::from_parameters(geometry.kind(), &geometry.parameters()).unwrap();
            assert_eq!(reread, geometry);
        }
        // Unknown kinds and wrong arities are refused.
        assert!(HelperGeometry::from_parameters("point", &[1., 2.]).is_err());
        assert!(HelperGeometry::from_parameters("line", &[0.; 7]).is_err());
        assert!(HelperGeometry::from_parameters("polyline", &[]).is_err());
        // A well-shaped circle with a degenerate normal is still refused.
        assert!(HelperGeometry::from_parameters("circle", &[0., 0., 0., 0., 0., 0., 50.]).is_err());
    }

    #[test]
    fn duplicate_and_overflowing_identities_are_refused() {
        let mut helpers = Helpers::default();
        helpers
            .insert(helper(1, HelperGeometry::Point([0.; 3])))
            .unwrap();
        // Both admission paths refuse a known identity, including a non-helper one.
        assert_eq!(
            helpers
                .insert(helper(1, HelperGeometry::Point([10.; 3])))
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        assert_eq!(
            helpers
                .create(&helper(1, HelperGeometry::Point([10.; 3])))
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        // Filling the project cap makes both paths refuse with the limit code.
        let mut full = Helpers::default();
        for n in 0..u32::try_from(beamcraft_model::helper::MAX_HELPERS).unwrap() {
            full.insert(helper(n, HelperGeometry::Point([0.; 3])))
                .unwrap();
        }
        let extra = helper(
            u32::try_from(beamcraft_model::helper::MAX_HELPERS).unwrap(),
            HelperGeometry::Point([0.; 3]),
        );
        assert_eq!(
            full.insert(extra.clone()).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        assert_eq!(
            full.create(&extra).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }
}
