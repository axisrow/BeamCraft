// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! T03 reflection of extruded parts with a proper instance pose and reflected local operations.
use crate::{
    command::AuthoredPlate, member::AuthoredMember, transform::rotate, work_plane::WorkPlane,
};
use beamcraft_model::{
    command::{CommandError, Placement},
    error::ErrorCode,
};
/// Admitted world reflection plane, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MirrorPlane {
    point: [f64; 3],
    normal: [f64; 3],
}
fn invalid() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidArgument,
        "mirror",
        "reflection needs finite, nondegenerate geometry",
    )
}
fn sub([x, y, z]: [f64; 3], [a, b, c]: [f64; 3]) -> [f64; 3] {
    [x - a, y - b, z - c]
}
fn add([x, y, z]: [f64; 3], [a, b, c]: [f64; 3]) -> [f64; 3] {
    [x + a, y + b, z + c]
}
fn dot([x, y, z]: [f64; 3], [a, b, c]: [f64; 3]) -> f64 {
    x * a + y * b + z * c
}
fn scale(v: [f64; 3], s: f64) -> [f64; 3] {
    v.map(|a| a * s)
}
fn unit(v: [f64; 3]) -> Result<[f64; 3], CommandError> {
    let [x, y, z] = v;
    let n = x.hypot(y).hypot(z);
    if !n.is_finite() || n < 1e-12 {
        return Err(invalid());
    }
    Ok(scale(v, 1. / n))
}
fn reflect(v: [f64; 3], n: [f64; 3]) -> [f64; 3] {
    sub(v, scale(n, 2. * dot(v, n)))
}
/// Local reflection in the extrusion's mid-plane; the uniform base extrusion is invariant.
#[derive(Debug, Clone, Copy)]
struct LocalReflection {
    axis: [f64; 3],
    offset: [f64; 3],
}
impl LocalReflection {
    fn point(self, p: [f64; 3]) -> [f64; 3] {
        add(reflect(p, self.axis), self.offset)
    }
    fn normal(self, n: [f64; 3]) -> [f64; 3] {
        reflect(n, self.axis)
    }
}
impl MirrorPlane {
    /// Admit a plane. A scaled normal has the same normalized representation.
    pub fn new(point: [f64; 3], normal: [f64; 3]) -> Result<Self, CommandError> {
        Placement {
            translation_mm: point,
            ..Placement::IDENTITY
        }
        .validate()?;
        Ok(Self {
            point,
            normal: unit(normal)?,
        })
    }
    /// Stable admitted arguments for command fingerprinting.
    pub fn parameters(self) -> [f64; 6] {
        let [a, b, c] = self.point;
        let [x, y, z] = self.normal;
        [a, b, c, x, y, z]
    }
    /// Reflect one world point without changing the reflection plane.
    pub fn point(self, p: [f64; 3]) -> Result<[f64; 3], CommandError> {
        let out = add(self.point, reflect(sub(p, self.point), self.normal));
        Placement {
            translation_mm: out,
            ..Placement::IDENTITY
        }
        .validate()?;
        Ok(out)
    }
    fn pose(
        self,
        pose: Placement,
        axis: [f64; 3],
        midpoint: [f64; 3],
    ) -> Result<(Placement, LocalReflection), CommandError> {
        pose.validate()?;
        let axis = unit(axis)?;
        let offset = scale(axis, 2. * dot(midpoint, axis));
        // Q = M R S: two reflections give a proper rotation. T' = mirror(T) - Q*k.
        let basis = |v| reflect(rotate(pose.orientation_xyzw, reflect(v, axis)), self.normal);
        let q = WorkPlane::new([0.; 3], basis([1., 0., 0.]), basis([0., 1., 0.]))?
            .placement()?
            .orientation_xyzw;
        let placement = Placement {
            translation_mm: sub(self.point(pose.translation_mm)?, rotate(q, offset)),
            orientation_xyzw: q,
        };
        placement.validate()?;
        Ok((placement, LocalReflection { axis, offset }))
    }
    /// Mirror a plate's world solid. Contour and through holes stay on the invariant base;
    /// local cut planes reflect in the extrusion mid-plane. Operation identities are retained.
    pub fn plate(self, source: &AuthoredPlate) -> Result<AuthoredPlate, CommandError> {
        let [p0, p1, p2, ..] = source.contour_points_mm.as_slice() else {
            return Err(invalid());
        };
        let axis = if source
            .contour_points_mm
            .iter()
            .all(|p| matches!(p,[_,_,z] if *z==0.))
        {
            [0., 0., 1.]
        } else {
            let [a, b, c] = sub(*p1, *p0);
            let [x, y, z] = sub(*p2, *p0);
            unit([b * z - c * y, c * x - a * z, a * y - b * x])?
        };
        let midpoint = add(*p0, scale(axis, source.thickness_mm / 2.));
        let (placement, local) = self.pose(source.placement, axis, midpoint)?;
        let mut out = source.clone();
        out.placement = placement;
        for cut in &mut out.cuts {
            cut.point_mm = local.point(cut.point_mm);
            cut.normal = local.normal(cut.normal);
            cut.validate()?;
        }
        Ok(out)
    }
    /// Mirror a straight member, including asymmetric sections, without reflecting its profile.
    /// Axial reflection leaves any uniform cross-section invariant and reverses local cuts.
    pub fn member(self, source: &AuthoredMember) -> Result<AuthoredMember, CommandError> {
        let [a, b] = source.axis_points_mm;
        let axis = unit(sub(b, a))?;
        let midpoint = scale(add(a, b), 0.5);
        let (placement, local) = self.pose(source.placement, axis, midpoint)?;
        let mut out = source.clone();
        out.placement = placement;
        for cut in &mut out.cuts {
            cut.point_mm = local.point(cut.point_mm);
            cut.normal = local.normal(cut.normal);
            cut.validate()?;
        }
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plane_is_an_involution() {
        let m = MirrorPlane::new([10., 20., 30.], [1., 2., 3.]).unwrap();
        let p = [100., -5., 40.];
        let q = m.point(m.point(p).unwrap()).unwrap();
        for (a, b) in p.into_iter().zip(q) {
            assert!((a - b).abs() < 1e-10);
        }
    }
    #[test]
    fn paired_reflections_produce_a_valid_pose() {
        let m = MirrorPlane::new([5., 0., 0.], [1., 0., 0.]).unwrap();
        let (p, local) = m
            .pose(Placement::IDENTITY, [0., 0., 1.], [0., 0., 5.])
            .unwrap();
        p.validate().unwrap();
        let v = [2., 3., 4.];
        let world = add(rotate(p.orientation_xyzw, local.point(v)), p.translation_mm);
        let expected = m.point(v).unwrap();
        for (a, b) in world.into_iter().zip(expected) {
            assert!((a - b).abs() < 1e-10);
        }
    }
}
