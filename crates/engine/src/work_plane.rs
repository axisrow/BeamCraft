// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V08 session work planes. Only resolved element placements enter the authored model.

use beamcraft_geom::{Frame3, Point3, Vec3};
use beamcraft_model::command::{CommandError, Placement};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::limits::COORDINATE_CAP_MM;

fn invalid() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidArgument,
        "plane",
        "work plane needs finite, nondegenerate orthogonal axes",
    )
}
fn point(p: [f64; 3]) -> Result<Point3, CommandError> {
    if p.iter().any(|v| !v.is_finite()) {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            "plane",
            "plane coordinates must be finite",
        ));
    }
    if p.iter().any(|v| v.abs() > COORDINATE_CAP_MM) {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "plane",
            "plane coordinate exceeds the model cap",
        ));
    }
    let [x, y, z] = p;
    Point3::try_new(x, y, z).map_err(|_| invalid())
}
fn vector([x, y, z]: [f64; 3]) -> Result<Vec3, CommandError> {
    if [x, y, z].iter().any(|v| !v.is_finite()) {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            "plane",
            "plane axes must be finite",
        ));
    }
    Vec3::new(x, y, z).map_err(|_| invalid())
}
fn components(p: Point3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

/// An admitted right-handed session frame; units are millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkPlane {
    origin: Point3,
    frame: Frame3,
}

impl Default for WorkPlane {
    fn default() -> Self {
        Self {
            origin: Point3::ORIGIN,
            frame: Frame3::IDENTITY,
        }
    }
}

impl WorkPlane {
    /// Normalize the two axes, refusing parallel or materially non-orthogonal input.
    pub fn new(origin: [f64; 3], x: [f64; 3], y: [f64; 3]) -> Result<Self, CommandError> {
        let origin = point(origin)?;
        let x = vector(x)?.normalized().map_err(|_| invalid())?;
        let y = vector(y)?.normalized().map_err(|_| invalid())?;
        if x.dot(y).map_err(|_| invalid())?.abs() > 1e-12 {
            return Err(invalid());
        }
        let z = x
            .cross(y)
            .and_then(Vec3::normalized)
            .map_err(|_| invalid())?;
        let y = z
            .cross(x)
            .and_then(Vec3::normalized)
            .map_err(|_| invalid())?;
        let frame = Frame3::try_new(x, y, z).map_err(|_| invalid())?;
        Ok(Self { origin, frame })
    }

    /// A plane parallel to XY, XZ or YZ, offset along the omitted positive model axis.
    pub fn parallel(name: &str, offset: f64) -> Result<Self, CommandError> {
        match name {
            "xy" => Self::new([0., 0., offset], [1., 0., 0.], [0., 1., 0.]),
            "xz" => Self::new([0., offset, 0.], [1., 0., 0.], [0., 0., 1.]),
            "yz" => Self::new([offset, 0., 0.], [0., 1., 0.], [0., 0., 1.]),
            _ => Err(invalid()),
        }
    }

    /// Origin, a point on +X, and a third non-collinear point on the positive-Y side.
    pub fn three_points(
        origin: [f64; 3],
        on_x: [f64; 3],
        on_y: [f64; 3],
    ) -> Result<Self, CommandError> {
        let o = point(origin)?;
        let x = o
            .vector_to(point(on_x)?)
            .and_then(Vec3::normalized)
            .map_err(|_| invalid())?;
        let towards_y = o.vector_to(point(on_y)?).map_err(|_| invalid())?;
        let z = x
            .cross(towards_y)
            .and_then(Vec3::normalized)
            .map_err(|_| invalid())?;
        let y = z
            .cross(x)
            .and_then(Vec3::normalized)
            .map_err(|_| invalid())?;
        Self::new(origin, x.components(), y.components())
    }

    /// A face frame with deterministic in-plane X and the face's outward normal as +Z.
    pub fn face(origin: [f64; 3], normal: [f64; 3]) -> Result<Self, CommandError> {
        let z = vector(normal)?.normalized().map_err(|_| invalid())?;
        let seed = if z.x().abs() < 0.9 {
            vector([1., 0., 0.])?
        } else {
            vector([0., 1., 0.])?
        };
        let y = z
            .cross(seed)
            .and_then(Vec3::normalized)
            .map_err(|_| invalid())?;
        let x = y
            .cross(z)
            .and_then(Vec3::normalized)
            .map_err(|_| invalid())?;
        Self::new(origin, x.components(), y.components())
    }

    /// Vertical through two points: local X follows their horizontal projection, local Y is +Z.
    pub fn vertical(a: [f64; 3], b: [f64; 3]) -> Result<Self, CommandError> {
        point(a)?;
        point(b)?;
        let [ax, ay, _] = a;
        let [bx, by, _] = b;
        Self::new(a, [bx - ax, by - ay, 0.], [0., 0., 1.])
    }

    /// Parse `OX,OY,OZ,XX,XY,XZ,YX,YY,YZ`; at most nine bounded scalar fields.
    pub fn parse(text: &str) -> Result<Self, CommandError> {
        if text.len() > 384 {
            return Err(invalid());
        }
        let values = text
            .split(',')
            .take(10)
            .map(|p| p.trim().parse::<f64>().map_err(|_| invalid()))
            .collect::<Result<Vec<_>, _>>()?;
        let [ox, oy, oz, xx, xy, xz, yx, yy, yz] = values.as_slice() else {
            return Err(invalid());
        };
        Self::new([*ox, *oy, *oz], [*xx, *xy, *xz], [*yx, *yy, *yz])
    }

    /// The CLI expression for the admitted frame.
    pub fn expression(self) -> String {
        let [ox, oy, oz] = self.origin();
        let [xx, xy, xz] = self.x();
        let [yx, yy, yz] = self.y();
        format!("{ox:?},{oy:?},{oz:?},{xx:?},{xy:?},{xz:?},{yx:?},{yy:?},{yz:?}")
    }
    /// Model-space origin.
    pub fn origin(self) -> [f64; 3] {
        components(self.origin)
    }
    /// Unit local X in model space.
    pub fn x(self) -> [f64; 3] {
        self.frame.x().components()
    }
    /// Unit local Y in model space.
    pub fn y(self) -> [f64; 3] {
        self.frame.y().components()
    }
    /// Unit normal in model space.
    pub fn normal(self) -> [f64; 3] {
        self.frame.z().components()
    }

    /// Resolves a local numeric coordinate, including XY shorthand, through the T04 resolver.
    /// Polar angles and direction locks use this plane's local axes.
    pub fn resolve_coordinate(
        self,
        text: &str,
        reference: [f64; 3],
        lock: crate::coordinate::AxisLock,
    ) -> Result<[f64; 3], CommandError> {
        if text.len() > 128 {
            return Err(invalid());
        }
        if !text.contains('<') && text.split(',').count() == 2 {
            crate::coordinate::resolve(&format!("{text},0"), reference, lock)
        } else {
            crate::coordinate::resolve(text, reference, lock)
        }
    }

    /// Constrains a world-space point along the active plane's local axis through a world reference.
    pub fn constrain_world(
        self,
        reference: [f64; 3],
        point: [f64; 3],
        lock: crate::coordinate::AxisLock,
    ) -> Result<[f64; 3], CommandError> {
        self.to_world(lock.constrain(self.to_local(reference)?, self.to_local(point)?)?)
    }

    /// Map a local coordinate to model space, checking the final model coordinate cap.
    pub fn to_world(self, local: [f64; 3]) -> Result<[f64; 3], CommandError> {
        point(local)?;
        let v = self
            .frame
            .map_vector(vector(local)?)
            .map_err(|_| invalid())?;
        let world = self.origin.translated(v).map_err(|_| invalid())?;
        let values = components(world);
        point(values)?;
        Ok(values)
    }
    /// Map a model point to local coordinates.
    pub fn to_local(self, world: [f64; 3]) -> Result<[f64; 3], CommandError> {
        let delta = self
            .origin
            .vector_to(point(world)?)
            .map_err(|_| invalid())?;
        let values = [
            delta.dot(self.frame.x()),
            delta.dot(self.frame.y()),
            delta.dot(self.frame.z()),
        ];
        let [x, y, z] = values.map(|v| v.map_err(|_| invalid()));
        let p = [x?, y?, z?];
        point(p)?;
        Ok(p)
    }
    /// Intersect a forward ray with this plane. Parallel/behind/non-finite rays have no hit.
    pub fn ray_point(self, origin: [f64; 3], direction: [f64; 3]) -> Option<[f64; 3]> {
        let from = vector(origin).ok()?;
        let ray = vector(direction).ok()?.normalized().ok()?;
        let n = self.frame.z();
        let denominator = n.dot(ray).ok()?;
        if denominator.abs() < 1e-12 {
            return None;
        }
        let o = vector(self.origin()).ok()?;
        let delta = o.added(from.scaled(-1.).ok()?).ok()?;
        let t = n.dot(delta).ok()? / denominator;
        if !t.is_finite() || t < 0. {
            return None;
        }
        let hit = from.added(ray.scaled(t).ok()?).ok()?.components();
        point(hit).ok()?;
        Some(hit)
    }

    /// The resolved element pose. This does not author a plane object or change any model.
    pub fn placement(self) -> Result<Placement, CommandError> {
        let [m00, m10, m20] = self.x();
        let [m01, m11, m21] = self.y();
        let [m02, m12, m22] = self.normal();
        let trace = m00 + m11 + m22;
        let q = if trace > 0. {
            let s = (trace + 1.).sqrt() * 2.;
            [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, s / 4.]
        } else if m00 > m11 && m00 > m22 {
            let s = (1. + m00 - m11 - m22).sqrt() * 2.;
            [s / 4., (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]
        } else if m11 > m22 {
            let s = (1. + m11 - m00 - m22).sqrt() * 2.;
            [(m01 + m10) / s, s / 4., (m12 + m21) / s, (m02 - m20) / s]
        } else {
            let s = (1. + m22 - m00 - m11).sqrt() * 2.;
            [(m02 + m20) / s, (m12 + m21) / s, s / 4., (m10 - m01) / s]
        };
        let norm = q.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !norm.is_finite() || norm <= 0. {
            return Err(invalid());
        }
        let orientation_xyzw = q.map(|v| {
            let v = v / norm;
            if v == 0. { 0. } else { v }
        });
        let pose = Placement {
            translation_mm: self.origin(),
            orientation_xyzw,
        };
        pose.validate()?;
        Ok(pose)
    }
}

/// Derive a work plane from the nearest front-facing picked face without changing the snapshot.
pub fn pick_work_plane(
    snapshot: &crate::ViewSnapshot,
    origin: [f64; 3],
    direction: [f64; 3],
) -> Result<Option<WorkPlane>, CommandError> {
    use crate::query::feature::{FeatureGeometry, FeatureKind, lookup_feature};
    use crate::query::pick::{PickQuery, RayPick, pick};
    let mut query = RayPick::new(origin, direction, vec![FeatureKind::Face]);
    query.max_hits = 1;
    let result = pick(snapshot, &PickQuery::Ray(query)).map_err(|e| {
        CommandError::new(
            e.f04_code(),
            "plane.face",
            "work-plane face pick was refused",
        )
    })?;
    result
        .hits
        .first()
        .map(|hit| {
            let unsupported = || {
                CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "plane.face",
                    "work planes require a supported planar face",
                )
            };
            let id = hit.feature_id.as_ref().ok_or_else(unsupported)?;
            let feature = lookup_feature(snapshot, &id.to_string()).map_err(|_| unsupported())?;
            let normal = match feature.world {
                FeatureGeometry::Face { normal, .. }
                | FeatureGeometry::Surface {
                    normal: Some(normal),
                    ..
                } => normal,
                _ => return Err(unsupported()),
            };
            WorkPlane::face(hit.point_world_mm, normal)
        })
        .transpose()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn planes_round_trip_and_encode_rigid_poses() {
        for plane in [
            WorkPlane::default(),
            WorkPlane::new([0.; 3], [1., 0., 0.], [0., -1., 0.]).unwrap(),
            WorkPlane::new([0.; 3], [-1., 0., 0.], [0., 1., 0.]).unwrap(),
            WorkPlane::new([0.; 3], [-1., 0., 0.], [0., -1., 0.]).unwrap(),
            WorkPlane::parallel("xz", 200.).unwrap(),
            WorkPlane::parallel("yz", 300.).unwrap(),
            WorkPlane::three_points([10., 20., 30.], [11., 22., 33.], [7., 21., 34.]).unwrap(),
            WorkPlane::face([1., 2., 3.], [1., -2., 3.]).unwrap(),
            WorkPlane::vertical([10., 20., 30.], [50., 60., 90.]).unwrap(),
        ] {
            let local = [100., 200., 0.];
            let world = plane.to_world(local).unwrap();
            for (a, b) in local.into_iter().zip(plane.to_local(world).unwrap()) {
                assert!((a - b).abs() < 1e-9);
            }
            let pose = plane.placement().unwrap();
            let mapped = crate::query::snapshot::rotate_vector(local, pose.orientation_xyzw);
            for ((a, b), o) in mapped.into_iter().zip(world).zip(plane.origin()) {
                assert!((a + o - b).abs() < 1e-9);
            }
            let reread = WorkPlane::parse(&plane.expression()).unwrap();
            for (a, b) in reread.normal().into_iter().zip(plane.normal()) {
                assert!((a - b).abs() < 1e-12);
            }
        }
        let tiny = WorkPlane::new([1e-300, 1e-300, 1e-300], [1., 0., 0.], [0., 1., 0.]).unwrap();
        assert!(tiny.expression().len() <= 384);
        assert_eq!(
            WorkPlane::parse(&tiny.expression()).unwrap().origin(),
            tiny.origin()
        );
        assert_eq!(
            WorkPlane::parallel("xz", 20.)
                .unwrap()
                .to_world([2., 3., 0.])
                .unwrap(),
            [2., 20., 3.]
        );
    }
    #[test]
    fn malformed_frames_and_hostile_input_never_become_planes() {
        for text in [
            "",
            "0,0,0,1,0,0,2,0,0",
            "0,0,0,1,0,0,1,1,0",
            "NaN,0,0,1,0,0,0,1,0",
            "0,0,0,0,0,0,0,1,0",
            "0,0,0,1,0,0,0,1,0,0",
        ] {
            assert!(WorkPlane::parse(text).is_err());
        }
        assert!(WorkPlane::vertical([0.; 3], [0., 0., 1.]).is_err());
        assert!(WorkPlane::three_points([0.; 3], [1., 0., 0.], [2., 0., 0.]).is_err());
        assert!(WorkPlane::face([0.; 3], [0.; 3]).is_err());
        assert!(WorkPlane::parse(&"0".repeat(385)).is_err());
        assert!(
            WorkPlane::parallel("xy", 1e9)
                .unwrap()
                .to_world([0., 0., 1.])
                .is_err()
        );
    }
    #[test]
    fn ray_intersections_refuse_parallel_behind_and_nonfinite_rays() {
        let p = WorkPlane::parallel("xz", 20.).unwrap();
        assert_eq!(p.ray_point([2., 0., 3.], [0., 1., 0.]), Some([2., 20., 3.]));
        assert_eq!(p.ray_point([0.; 3], [1., 0., 0.]), None);
        assert_eq!(p.ray_point([0.; 3], [0., -1., 0.]), None);
        assert_eq!(p.ray_point([f64::NAN, 0., 0.], [0., 1., 0.]), None);
    }
}
