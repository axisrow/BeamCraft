// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Straight member with a centred rectangle section (F06 section 2, P-2/P-3, section 6 box).
//!
//! The member is a box in the body frame `B = [X_m Y_m Z_m]` of `geom::MemberFrame` (default
//! zero-roll frame plus explicit roll). Its corner is `o = (p1 - (w/2)*X_m) - (h/2)*Y_m`, its
//! edges `e1 = w*X_m`, `e2 = h*Y_m`, `e3 = p2 - p1`, and its vertices, triangles and face order
//! are the section 6 layout shared with the plate. The result is the same sealed
//! [`PlateResult`] type: 8 vertices, 12 triangles, 6 faces, one BCMESH01 blob. Faces carry the
//! F06 tokens `f.x0 .. f.z1`, so `z0` is the end face at `p1` and `z1` the one at `p2`.
//!
//! No rotation of the element (placement) and no other profile kind live here.

use crate::PlateResult;
use crate::diagnostic::{Code, Failure, Field, Stage};
use crate::mesh::{PLATE_TRIANGLES, PlateMesh, encode, self_check};
use beamcraft_geom::tolerance::{COORDINATE_CAP_MM, DIMENSION_CAP_MM, ROLL_CAP_RAD};
use beamcraft_geom::{MathError, MemberFrame, Point3, Vec3};

/// Raw member input in element-local millimetres and radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StraightMember {
    /// Axis start `p1`.
    pub start: Point3,
    /// Axis end `p2`.
    pub end: Point3,
    /// Section width, along the rolled `X_m`.
    pub width_mm: f64,
    /// Section height, along the rolled `Y_m`.
    pub height_mm: f64,
    /// Explicit roll about `+Z_m`; absent is zero.
    pub roll_rad: Option<f64>,
}

pub(crate) fn locate(input: &StraightMember, error: MathError) -> (Code, Field, Stage) {
    let scalars = [
        (input.start.x, Field::Axis, COORDINATE_CAP_MM),
        (input.start.y, Field::Axis, COORDINATE_CAP_MM),
        (input.start.z, Field::Axis, COORDINATE_CAP_MM),
        (input.end.x, Field::Axis, COORDINATE_CAP_MM),
        (input.end.y, Field::Axis, COORDINATE_CAP_MM),
        (input.end.z, Field::Axis, COORDINATE_CAP_MM),
        (input.width_mm, Field::Section, DIMENSION_CAP_MM),
        (input.height_mm, Field::Section, DIMENSION_CAP_MM),
        (input.roll_rad.unwrap_or(0.0), Field::Roll, ROLL_CAP_RAD),
    ];
    match error {
        MathError::NonFinite => {
            let field = scalars
                .iter()
                .find(|(v, _, _)| !v.is_finite())
                .map_or(Field::Mesh, |(_, f, _)| *f);
            (Code::NonFinite, field, Stage::Finiteness)
        }
        MathError::AboveLimit => {
            let field = scalars
                .iter()
                .find(|(v, f, cap)| {
                    if *f == Field::Section {
                        *v > *cap
                    } else {
                        v.abs() > *cap
                    }
                })
                .map_or(Field::LocalSize, |(_, f, _)| *f);
            (Code::AboveLimit, field, Stage::Caps)
        }
        MathError::NotPositive => (Code::NotPositive, Field::Section, Stage::Ranges),
        MathError::BelowMinimum => (Code::BelowMinimum, Field::Section, Stage::Ranges),
        MathError::AxisTooShort | MathError::ZeroAxis => {
            (Code::AxisTooShort, Field::Axis, Stage::Ranges)
        }
        // F06 section 4 rule 2: a failed frame certificate is reported at the build stage.
        MathError::InvalidFrame => (Code::NumericUncertain, Field::LocalSize, Stage::Build),
        // F06 section 2 stage 3: a non-finite derived local size is a caps-stage finding.
        MathError::NumericUncertain | MathError::InvalidPointCount => {
            (Code::NumericUncertain, Field::LocalSize, Stage::Caps)
        }
    }
}

/// Builds a closed straight member, validating every submitted value.
///
/// The frame, local scale and minima come from `geom::MemberFrame` (F06 section 4 rules 1 to 3
/// and the frame certificate); a frame that fails its certificate is `GEO_NUMERIC_UNCERTAIN`.
/// Volume is `(width * height) * axis_length`.
pub fn straight_member(input: StraightMember) -> Result<PlateResult, Failure> {
    let numeric = || Failure::one(Code::NumericUncertain, Field::Mesh, Stage::Build);
    let frame = MemberFrame::new(
        input.start,
        input.end,
        input.width_mm,
        input.height_mm,
        input.roll_rad,
    )
    .map_err(|error| {
        let (code, field, stage) = locate(&input, error);
        Failure::one(code, field, stage)
    })?;
    let scale = frame.scale();
    let basis = frame.frame();
    let (x, y, z) = (basis.x(), basis.y(), basis.z());
    let e1 = x.scaled(input.width_mm).map_err(|_| numeric())?;
    let e2 = y.scaled(input.height_mm).map_err(|_| numeric())?;
    let e3 = input.start.vector_to(input.end).map_err(|_| numeric())?;
    let half_x = x.scaled(-(input.width_mm / 2.0)).map_err(|_| numeric())?;
    let half_y = y.scaled(-(input.height_mm / 2.0)).map_err(|_| numeric())?;
    let origin = input
        .start
        .translated(half_x)
        .and_then(|p| p.translated(half_y))
        .map_err(|_| numeric())?;
    let mut vertices = [Point3::ORIGIN; 8];
    for (i, slot) in vertices.iter_mut().enumerate() {
        let xb = f64::from(u8::from(i & 1 != 0));
        let yb = f64::from(u8::from(i & 2 != 0));
        let zb = f64::from(u8::from(i & 4 != 0));
        // F06 section 6: ((o + xb*e1) + yb*e2) + zb*e3.
        *slot = origin
            .translated(e1.scaled(xb).map_err(|_| numeric())?)
            .and_then(|p| p.translated(e2.scaled(yb)?))
            .and_then(|p| p.translated(e3.scaled(zb)?))
            .map_err(|_| numeric())?;
    }
    let negate = |v: Vec3| v.scaled(-1.0).map_err(|_| numeric());
    let normals = [negate(x)?, x, negate(y)?, y, negate(z)?, z];
    let (blob, digest) = encode(&vertices)?;
    self_check(&vertices, &PLATE_TRIANGLES, &normals)?;
    let volume_mm3 = (input.width_mm * input.height_mm) * scale.axis_length_mm();
    if !volume_mm3.is_finite() || volume_mm3 <= 0.0 {
        return Err(Failure::one(
            Code::NumericUncertain,
            Field::Volume,
            Stage::Quantities,
        ));
    }
    Ok(PlateResult {
        mesh: PlateMesh {
            face_ids: &crate::feature_id::MEMBER_FACES,
            vertices,
            normals,
            blob,
            digest,
        },
        volume_mm3,
        tolerance: scale.tolerance(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> StraightMember {
        StraightMember {
            start: Point3::ORIGIN,
            end: Point3::new(0.0, 0.0, 1000.0),
            width_mm: 200.0,
            height_mm: 300.0,
            roll_rad: None,
        }
    }

    #[test]
    fn every_math_error_has_the_f06_code_and_stage() {
        // F06 sections 2 and 4: axis findings are GEO_AXIS_TOO_SHORT at the range stage, a
        // failed frame certificate is GEO_NUMERIC_UNCERTAIN at the build stage (F06-NEG-31,
        // F06-NEG-35) and a non-finite derived intermediate is the same code at the caps stage
        // (F06-NEG-50).
        let table = [
            (MathError::AxisTooShort, Code::AxisTooShort, Stage::Ranges),
            (MathError::ZeroAxis, Code::AxisTooShort, Stage::Ranges),
            (
                MathError::InvalidFrame,
                Code::NumericUncertain,
                Stage::Build,
            ),
            (
                MathError::NumericUncertain,
                Code::NumericUncertain,
                Stage::Caps,
            ),
            (MathError::NotPositive, Code::NotPositive, Stage::Ranges),
            (MathError::BelowMinimum, Code::BelowMinimum, Stage::Ranges),
        ];
        for (error, code, stage) in table {
            let (got_code, _, got_stage) = locate(&input(), error);
            assert_eq!((got_code, got_stage), (code, stage), "{error:?}");
        }
    }
}
