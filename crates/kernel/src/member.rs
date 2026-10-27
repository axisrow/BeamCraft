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
use beamcraft_model::command::SectionPlacement;

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
    /// Position of the section about the axis (P14). The default keeps the centred body and the
    /// original vertex expressions bit for bit.
    pub placement: SectionPlacement,
}

/// The physical extent of the solid along the axis and its failure: `Err` is the F06 finding for
/// a non-positive or non-finite net length after the P14 end offsets.
pub(crate) fn solid_length(
    axis_length_mm: f64,
    placement: &SectionPlacement,
) -> Result<f64, (Code, Field, Stage)> {
    let length = axis_length_mm - placement.start_offset_mm + placement.end_offset_mm;
    if !length.is_finite() {
        return Err((Code::NonFinite, Field::SectionPlacement, Stage::Finiteness));
    }
    if length <= 0.0 {
        return Err((Code::AxisTooShort, Field::SectionPlacement, Stage::Ranges));
    }
    Ok(length)
}

/// Admits the P14 offsets themselves: finite and within the dimension cap. The frame's `locate`
/// re-checks the same values, but it only runs when the frame is refused, and the kernel never
/// trusts a caller's admission (a committed definition or store is re-evaluated without the
/// model layer's argument checks), so every builder runs this first.
pub(crate) fn check_placement(placement: &SectionPlacement) -> Result<(), (Code, Field, Stage)> {
    for value in [
        placement.plane_offset_mm,
        placement.depth_offset_mm,
        placement.start_offset_mm,
        placement.end_offset_mm,
    ] {
        if !value.is_finite() {
            return Err((Code::NonFinite, Field::SectionPlacement, Stage::Finiteness));
        }
        if value.abs() > DIMENSION_CAP_MM {
            return Err((Code::AboveLimit, Field::SectionPlacement, Stage::Caps));
        }
    }
    Ok(())
}

pub(crate) fn locate(input: &StraightMember, error: MathError) -> (Code, Field, Stage) {
    let placement = &input.placement;
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
        (
            placement.plane_offset_mm,
            Field::SectionPlacement,
            DIMENSION_CAP_MM,
        ),
        (
            placement.depth_offset_mm,
            Field::SectionPlacement,
            DIMENSION_CAP_MM,
        ),
        (
            placement.start_offset_mm,
            Field::SectionPlacement,
            DIMENSION_CAP_MM,
        ),
        (
            placement.end_offset_mm,
            Field::SectionPlacement,
            DIMENSION_CAP_MM,
        ),
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
/// Volume is `(width * height) * solid_length`, where the solid length is the axis length moved
/// by the P14 end offsets. The default placement keeps the centred body and the original vertex
/// expressions bit for bit; a shifted placement moves the box origin in the rolled local section
/// plane and scales the extent along `Z_m`.
pub fn straight_member(input: StraightMember) -> Result<PlateResult, Failure> {
    let numeric = || Failure::one(Code::NumericUncertain, Field::Mesh, Stage::Build);
    check_placement(&input.placement)
        .map_err(|(code, field, stage)| Failure::one(code, field, stage))?;
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
    let placement = input.placement;
    let shifted = !placement.is_default();
    let solid_length_mm = if shifted {
        solid_length(scale.axis_length_mm(), &placement)
            .map_err(|(code, field, stage)| Failure::one(code, field, stage))?
    } else {
        scale.axis_length_mm()
    };
    // The default keeps `p2 - p1` as the extent; a shifted placement derives it from the frame
    // axis and the net offsets, which differ in the last bits from a negated addition.
    let e3 = if shifted {
        z.scaled(solid_length_mm).map_err(|_| numeric())?
    } else {
        input.start.vector_to(input.end).map_err(|_| numeric())?
    };
    let half_x = x.scaled(-(input.width_mm / 2.0)).map_err(|_| numeric())?;
    let half_y = y.scaled(-(input.height_mm / 2.0)).map_err(|_| numeric())?;
    let origin = if shifted {
        let shift_x = placement.plane.half_shift_mm(input.width_mm) + placement.plane_offset_mm;
        let shift_y = placement.depth.half_shift_mm(input.height_mm) + placement.depth_offset_mm;
        let mut origin = input.start;
        for (axis, mm) in [
            (z, placement.start_offset_mm),
            (x, shift_x),
            (y, shift_y),
            (x, -input.width_mm / 2.0),
            (y, -input.height_mm / 2.0),
        ] {
            let step = axis.scaled(mm).map_err(|_| numeric())?;
            origin = origin.translated(step).map_err(|_| numeric())?;
        }
        origin
    } else {
        input
            .start
            .translated(half_x)
            .and_then(|p| p.translated(half_y))
            .map_err(|_| numeric())?
    };
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
    let volume_mm3 = (input.width_mm * input.height_mm) * solid_length_mm;
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
    use beamcraft_model::command::{SectionDepthPosition, SectionPlanePosition};

    fn input() -> StraightMember {
        StraightMember {
            start: Point3::ORIGIN,
            end: Point3::new(0.0, 0.0, 1000.0),
            width_mm: 200.0,
            height_mm: 300.0,
            roll_rad: None,
            placement: SectionPlacement::default(),
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

    /// The eight vertices of the built body, in the F06 section 6 order.
    fn vertices_of(result: &PlateResult) -> [Point3; 8] {
        let mut corners = [Point3::ORIGIN; 8];
        for (slot, vertex) in corners.iter_mut().zip(result.mesh.vertices) {
            *slot = vertex;
        }
        corners
    }

    #[test]
    fn the_default_placement_keeps_the_centred_p03_body() {
        let result = straight_member(input()).unwrap();
        // Corner o = p1 - (w/2)X_m - (h/2)Y_m and e3 = p2 - p1: the centred 200 x 300 box over
        // the 1000 mm axis, volume 0.06 m3.
        let v = vertices_of(&result);
        assert_eq!(v[0], Point3::new(-100.0, -150.0, 0.0));
        assert_eq!(v[3], Point3::new(100.0, 150.0, 0.0));
        assert_eq!(v[7], Point3::new(100.0, 150.0, 1000.0));
        assert_eq!(result.volume_mm3, 200.0 * 300.0 * 1000.0);
    }

    #[test]
    fn every_plane_and_depth_position_moves_the_box_its_half_width_or_height() {
        // The axis runs along +Z, so X_m and Y_m are the world X and Y axes: each shift is a
        // plain coordinate offset of the bounding-box centre.
        let cases = [
            (SectionPlanePosition::Left, -100.0, 0.0),
            (SectionPlanePosition::Centre, 0.0, 0.0),
            (SectionPlanePosition::Right, 100.0, 0.0),
        ];
        for (plane, dx, _) in cases {
            let mut input = input();
            input.placement.plane = plane;
            let result = straight_member(input).unwrap();
            let v = vertices_of(&result);
            assert_eq!(v[0], Point3::new(-100.0 + dx, -150.0, 0.0), "{plane:?}");
            assert_eq!(v[7], Point3::new(100.0 + dx, 150.0, 1000.0), "{plane:?}");
            assert_eq!(result.volume_mm3, 200.0 * 300.0 * 1000.0);
        }
        let cases = [
            (SectionDepthPosition::Front, -150.0),
            (SectionDepthPosition::Centre, 0.0),
            (SectionDepthPosition::Back, 150.0),
        ];
        for (depth, dy) in cases {
            let mut input = input();
            input.placement.depth = depth;
            let result = straight_member(input).unwrap();
            let v = vertices_of(&result);
            assert_eq!(v[0], Point3::new(-100.0, -150.0 + dy, 0.0), "{depth:?}");
            assert_eq!(v[7], Point3::new(100.0, 150.0 + dy, 1000.0), "{depth:?}");
        }
    }

    #[test]
    fn explicit_offsets_add_to_the_positions_and_move_the_faces_along_the_axis() {
        // Left + 40 mm plane offset: the centre sits at -w/2 + 40 = -60; front + 50 mm depth
        // offset: at -h/2 + 50 = -100. A 200 mm start offset and a -700 mm end offset leave a
        // 100 mm solid that runs from z = 200 to z = 300.
        let mut input = input();
        input.placement = SectionPlacement {
            plane: SectionPlanePosition::Left,
            plane_offset_mm: 40.0,
            depth: SectionDepthPosition::Front,
            depth_offset_mm: 50.0,
            start_offset_mm: 200.0,
            end_offset_mm: -700.0,
        };
        let result = straight_member(input).unwrap();
        let v = vertices_of(&result);
        assert_eq!(v[0], Point3::new(-160.0, -250.0, 200.0));
        assert_eq!(v[7], Point3::new(40.0, 50.0, 300.0));
        assert_eq!(result.volume_mm3, 200.0 * 300.0 * 100.0);
    }

    #[test]
    fn an_extended_end_offset_lengthens_the_solid_beyond_the_axis() {
        let mut input = input();
        input.placement.start_offset_mm = -100.0;
        input.placement.end_offset_mm = 250.0;
        let result = straight_member(input).unwrap();
        let v = vertices_of(&result);
        assert_eq!(v[0].z, -100.0);
        assert_eq!(v[7].z, 1250.0);
        assert_eq!(result.volume_mm3, 200.0 * 300.0 * 1350.0);
    }

    #[test]
    fn a_placement_that_consumes_the_whole_axis_refuses_the_member() {
        // Net length zero and negative: GEO_AXIS_TOO_SHORT at the range stage, field
        // `section_placement`; a non-finite offset is a finiteness finding.
        let finding = |input: StraightMember| {
            let failure = straight_member(input).unwrap_err();
            let d = failure.diagnostics().next().unwrap();
            (d.code, d.field, d.stage)
        };
        let mut consumed = input();
        consumed.placement.end_offset_mm = -1000.0;
        assert_eq!(
            finding(consumed),
            (Code::AxisTooShort, Field::SectionPlacement, Stage::Ranges)
        );
        let mut past = input();
        past.placement.end_offset_mm = -2000.0;
        assert_eq!(
            finding(past),
            (Code::AxisTooShort, Field::SectionPlacement, Stage::Ranges)
        );
        let mut non_finite = input();
        non_finite.placement.start_offset_mm = f64::NAN;
        assert_eq!(
            finding(non_finite),
            (Code::NonFinite, Field::SectionPlacement, Stage::Finiteness)
        );
    }

    #[test]
    fn an_over_cap_finite_offset_is_refused_by_the_builder_itself() {
        // The frame is fine, so the frame-error mapping never runs: the builder checks the P14
        // caps itself, or a direct caller (or an untrusted committed store) builds past 1e6 mm.
        for mutate in [
            |p: &mut SectionPlacement| p.plane_offset_mm = 1_000_000.5,
            |p: &mut SectionPlacement| p.depth_offset_mm = -1_000_001.0,
            |p: &mut SectionPlacement| p.start_offset_mm = 2_000_000.0,
            |p: &mut SectionPlacement| p.end_offset_mm = 1e9,
        ] {
            let mut input = input();
            mutate(&mut input.placement);
            let failure = straight_member(input).unwrap_err();
            let d = failure.diagnostics().next().unwrap();
            assert_eq!(
                (d.code, d.field, d.stage),
                (Code::AboveLimit, Field::SectionPlacement, Stage::Caps)
            );
        }
        // Within the cap it builds.
        let mut input = input();
        input.placement.end_offset_mm = 1_000_000.0;
        assert!(straight_member(input).is_ok());
    }
}
