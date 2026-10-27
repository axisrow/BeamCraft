// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! T03 bounded, deterministic rigid group operations, independent of storage.
use beamcraft_model::{
    command::{CommandError, Placement},
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
    id::{ElementId, OpId, Uuid7},
};
/// Maximum parts affected by one group command, within the footprint budget.
pub const MAX_GROUP: usize = 128;
/// An admitted proper rigid transform in world coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidDelta {
    translation: [f64; 3],
    rotation: [f64; 4],
}
fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "transform", message)
}
/// Quaternion rotation of a vector. The caller supplies an admitted unit quaternion.
pub fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let [x, y, z, w] = q;
    let [a, b, c] = v;
    let t = [
        2.0 * (y * c - z * b),
        2.0 * (z * a - x * c),
        2.0 * (x * b - y * a),
    ];
    let [u, vv, tz] = t;
    [
        a + w * u + y * tz - z * vv,
        b + w * vv + z * u - x * tz,
        c + w * tz + x * vv - y * u,
    ]
}
/// Hamilton product, with a normalized result to control accumulated roundoff.
pub fn compose(a: [f64; 4], b: [f64; 4]) -> Result<[f64; 4], CommandError> {
    Placement::validate_orientation(a)?;
    Placement::validate_orientation(b)?;
    let [x, y, z, w] = a;
    let [u, v, t, s] = b;
    let q = [
        w * u + x * s + y * t - z * v,
        w * v - x * t + y * s + z * u,
        w * t + x * v - y * u + z * s,
        w * s - x * u - y * v - z * t,
    ];
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    let q = q.map(|v| v / n);
    Placement::validate_orientation(q)?;
    Ok(q)
}
impl RigidDelta {
    /// Translate by a finite world vector.
    pub fn translation(vector: [f64; 3]) -> Result<Self, CommandError> {
        let p = Placement {
            translation_mm: vector,
            ..Placement::IDENTITY
        };
        p.validate()?;
        Ok(Self {
            translation: vector,
            rotation: p.orientation_xyzw,
        })
    }
    /// Rotate around a world axis through a center; angle is radians.
    pub fn rotation(center: [f64; 3], axis: [f64; 3], angle: f64) -> Result<Self, CommandError> {
        Placement {
            translation_mm: center,
            ..Placement::IDENTITY
        }
        .validate()?;
        if !angle.is_finite() || axis.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "rotation",
                "rotation must be finite",
            ));
        }
        let [x, y, z] = axis;
        let n = x.hypot(y).hypot(z);
        if !n.is_finite() || n < 1e-12 || angle.abs() > std::f64::consts::TAU * 1e6 {
            return Err(invalid("rotation axis or angle is outside limits"));
        }
        let s = (angle / 2.0).sin() / n;
        let rotation = [x * s, y * s, z * s, (angle / 2.0).cos()];
        let [a, b, c] = rotate(rotation, center);
        let [x, y, z] = center;
        Ok(Self {
            translation: [x - a, y - b, z - c],
            rotation,
        })
    }
    /// Transform a placement, retaining the local geometry and its engineering quantities.
    pub fn apply(self, pose: Placement) -> Result<Placement, CommandError> {
        pose.validate()?;
        let [a, b, c] = rotate(self.rotation, pose.translation_mm);
        let [x, y, z] = self.translation;
        let next = Placement {
            translation_mm: [a + x, b + y, c + z],
            orientation_xyzw: compose(self.rotation, pose.orientation_xyzw)?,
        };
        next.validate()?;
        Ok(next)
    }
    /// Canonical command argument digest; includes selection and copy semantics.
    pub fn fingerprint(
        self,
        ids: &[ElementId],
        copy: bool,
        instance: bool,
    ) -> Result<ContentDigest, CommandError> {
        validate_ids(ids)?;
        let mut bytes = b"BeamCraft/T03/RigidGroup/v1\0".to_vec();
        bytes.extend([u8::from(copy), u8::from(instance)]);
        for v in self.translation.into_iter().chain(self.rotation) {
            bytes.extend(if v == 0.0 {
                0.0_f64.to_le_bytes()
            } else {
                v.to_le_bytes()
            });
        }
        for id in ids {
            bytes.extend(id.to_string().bytes());
        }
        Sha256::digest(&bytes).map_err(|_| invalid("request hash limit"))
    }
}
/// Admit an explicit, sorted, nonempty UUID selection; no duplicates or implicit expansion.
pub fn validate_ids(ids: &[ElementId]) -> Result<(), CommandError> {
    if ids.is_empty() || ids.len() > MAX_GROUP {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "ids",
            "selection must contain 1..128 elements",
        ));
    }
    if ids.windows(2).any(|p| matches!(p,[a,b] if a>=b)) {
        return Err(invalid("selection must be sorted and unique"));
    }
    Ok(())
}
/// Stable copy identity from operation, source and array slot; collision checked by the host.
pub fn copy_id(op: OpId, source: ElementId, slot: u32) -> Result<ElementId, CommandError> {
    let bytes = format!("BeamCraft/T03/CopyId/v1\0{op}{source}{slot}");
    let d = Sha256::digest(bytes.as_bytes()).map_err(|_| invalid("identity hash limit"))?;
    let mut out = [0u8; 16];
    for (to, from) in out.iter_mut().zip(d.as_bytes()) {
        *to = *from;
    }
    if let Some(v) = out.get_mut(6) {
        *v = (*v & 15) | 112;
    }
    if let Some(v) = out.get_mut(8) {
        *v = (*v & 63) | 128;
    }
    Ok(ElementId::from_uuid(
        Uuid7::from_bytes(out).map_err(|_| invalid("derived identity is invalid"))?,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotates_world_origin_and_composes_pose() {
        let t =
            RigidDelta::rotation([10., 0., 0.], [0., 0., 2.], std::f64::consts::FRAC_PI_2).unwrap();
        let p = t
            .apply(Placement {
                translation_mm: [20., 0., 0.],
                ..Placement::IDENTITY
            })
            .unwrap();
        assert!((p.translation_mm[0] - 10.).abs() < 1e-10);
        assert!((p.translation_mm[1] - 10.).abs() < 1e-10);
        p.validate().unwrap();
    }
    #[test]
    fn refuses_nonfinite_axis_and_overflow() {
        assert!(RigidDelta::rotation([0.; 3], [0.; 3], 1.).is_err());
        assert!(RigidDelta::rotation([0.; 3], [1., 0., 0.], f64::NAN).is_err());
        assert!(
            RigidDelta::translation([1e9, 0., 0.])
                .unwrap()
                .apply(Placement {
                    translation_mm: [1e9, 0., 0.],
                    ..Placement::IDENTITY
                })
                .is_err()
        );
    }
}
