// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T03 rigid transforms and arrays: a rotation about a world axis, a mirror about a world
//! plane and the placements of a linear or polar array.
//!
//! A rotation composes with the placement alone: the local shape is untouched. A mirror is not
//! rigid, so it is re-encoded through a linear symmetry `L` of the element's local body:
//! `R' = A·R·L`, `t' = A·t + c`, with `A` the world mirror's linear part and `c` its offset.
//! `L·(local body) = local body'` stays expressible in the authored fields, which makes the
//! world result exact:
//!
//! | Body | `L` | Local fields the mirror rewrites |
//! |------|-----|----------------------------------|
//! | Plate | reflection in the local `z = 0` plane | `thickness_side` flips (top and bottom trade places, middle is its own image); the plane cuts reflect |
//! | Straight member | reflection across the plane perpendicular to the axis | the path reflects (two points), the cuts reflect; the section placement re-encodes for the reversed sweep (offsets stay, the across-width pair negates, the roll negates) |
//!
//! Contours, holes and corner treatments are through-thickness or in-plane and need no change.
//! A polybeam or curved member, and a plate with bend records (A10), have no expressible
//! symmetry and refuse a mirror with `UNSUPPORTED_GEOMETRY`. Honesty limit: the profile sweep
//! keeps the unmirrored profile definition, so a member mirror is exact for mirror-symmetric
//! sections and preserves the profile handedness of a chiral one (L, Z); there is no
//! mirrored-profile definition yet. Arrays reuse the copy rules (fresh identities, freshly
//! minted records) and give each copy the stepped placement.

use crate::engine::command::AuthoredPlate;
use crate::engine::member::AuthoredMember;
use crate::model::command::{ArrayElements, ArrayStep, ElementMirror, ElementRotate};
use crate::model::command::{CutRecord, PlateSide};
use beamcraft_model::command::{CommandError, Placement};
use beamcraft_model::error::ErrorCode;

/// A 3x3 matrix, row-major.
type Mat3 = [[f64; 3]; 3];

/// The Hamilton product `a * b`: rotate by `b` first, then by `a`.
fn quat_multiply(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// Scales a quaternion to unit length. The Hamilton product of two unit quaternions can push a
/// component past 1.0 by an ulp (a second quarter turn about Z gives z = `2·sin(45°)·cos(45°)`),
/// and platform libm rounding decides on which side of 1.0 that lands. The drift is rounding,
/// not rotation: renormalizing keeps every composed placement inside the stored `[-1, 1]`
/// component bound (DV-LIM-003) that [`Placement::validate`] checks.
fn quat_normalized(q: [f64; 4]) -> [f64; 4] {
    let [x, y, z, w] = q;
    let norm = ((x * x + y * y) + (z * z + w * w)).sqrt();
    [x / norm, y / norm, z / norm, w / norm]
}

/// The rotation matrix of a unit quaternion (row-major; maps a local vector to world).
fn quat_to_matrix(q: [f64; 4]) -> Mat3 {
    let [x, y, z, w] = q;
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

/// The unit quaternion of a proper rotation matrix (Shepperd's method); the input must be
/// numerically orthogonal with determinant one, as every matrix this module builds is.
fn quat_from_matrix(m: &Mat3) -> [f64; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let (x, y, z, w) = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        (
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
            0.25 * s,
        )
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        (
            0.25 * s,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[2][1] - m[1][2]) / s,
        )
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        (
            (m[0][1] + m[1][0]) / s,
            0.25 * s,
            (m[1][2] + m[2][1]) / s,
            (m[0][2] - m[2][0]) / s,
        )
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        (
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            0.25 * s,
            (m[1][0] - m[0][1]) / s,
        )
    };
    let norm = ((x * x + y * y) + (z * z + w * w)).sqrt();
    if norm > 0.0 {
        [x / norm, y / norm, z / norm, w / norm]
    } else {
        // Unreachable for an orthogonal input; the caller validates the result anyway.
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// Applies a matrix to a vector.
fn apply(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Matrix product `a * b`, unrolled over the fixed 3x3 shape.
fn multiply(a: &Mat3, b: &Mat3) -> Mat3 {
    [
        [
            a[0][0] * b[0][0] + a[0][1] * b[1][0] + a[0][2] * b[2][0],
            a[0][0] * b[0][1] + a[0][1] * b[1][1] + a[0][2] * b[2][1],
            a[0][0] * b[0][2] + a[0][1] * b[1][2] + a[0][2] * b[2][2],
        ],
        [
            a[1][0] * b[0][0] + a[1][1] * b[1][0] + a[1][2] * b[2][0],
            a[1][0] * b[0][1] + a[1][1] * b[1][1] + a[1][2] * b[2][1],
            a[1][0] * b[0][2] + a[1][1] * b[1][2] + a[1][2] * b[2][2],
        ],
        [
            a[2][0] * b[0][0] + a[2][1] * b[1][0] + a[2][2] * b[2][0],
            a[2][0] * b[0][1] + a[2][1] * b[1][1] + a[2][2] * b[2][1],
            a[2][0] * b[0][2] + a[2][1] * b[1][2] + a[2][2] * b[2][2],
        ],
    ]
}

/// Unit quaternion of a rotation by `angle_rad` about the unit `axis`.
fn quat_from_axis_angle(axis: [f64; 3], angle_rad: f64) -> [f64; 4] {
    let half = angle_rad / 2.0;
    let s = half.sin();
    [axis[0] * s, axis[1] * s, axis[2] * s, half.cos()]
}

/// Normalizes a direction to unit length; falls back to the Z axis for a zero or non-finite
/// direction so the engine never divides by zero (the specs validate the input first).
fn unit_or_z(direction: [f64; 3]) -> [f64; 3] {
    let norm = ((direction[0] * direction[0] + direction[1] * direction[1])
        + direction[2] * direction[2])
        .sqrt();
    if norm > 0.0 && norm.is_finite() {
        [
            direction[0] / norm,
            direction[1] / norm,
            direction[2] / norm,
        ]
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn degenerate(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, field, message)
}

/// The placement rotated by `angle_rad` about the world axis through `pivot_mm` along `axis`.
/// The rotation composes on the world side: `R' = Q·R` and `t' = pivot + Q·(t - pivot)`.
pub fn rotate_placement(
    placement: &Placement,
    rotate: &ElementRotate,
) -> Result<Placement, CommandError> {
    rotate.validate()?;
    let axis = unit_or_z(rotate.axis);
    let step = quat_from_axis_angle(axis, rotate.angle_rad);
    let next = Placement {
        translation_mm: {
            let offset = [
                placement.translation_mm[0] - rotate.pivot_mm[0],
                placement.translation_mm[1] - rotate.pivot_mm[1],
                placement.translation_mm[2] - rotate.pivot_mm[2],
            ];
            let turned = beamcraft_engine::solid::rotate_vector(offset, step);
            [
                rotate.pivot_mm[0] + turned[0],
                rotate.pivot_mm[1] + turned[1],
                rotate.pivot_mm[2] + turned[2],
            ]
        },
        orientation_xyzw: quat_normalized(quat_multiply(step, placement.orientation_xyzw)),
    };
    next.validate()?;
    Ok(next)
}

/// The world mirror of `mirror`: the linear part `A` and the offset `c`, with `M(x) = A·x + c`.
fn world_mirror(mirror: &ElementMirror) -> Result<(Mat3, [f64; 3]), CommandError> {
    mirror.validate()?;
    if mirror.normal.iter().all(|v| *v == 0.0) {
        return Err(degenerate("mirror", "the mirror normal is the zero vector"));
    }
    let n = unit_or_z(mirror.normal);
    let a = [
        [
            1.0 - 2.0 * n[0] * n[0],
            -2.0 * n[0] * n[1],
            -2.0 * n[0] * n[2],
        ],
        [
            -2.0 * n[1] * n[0],
            1.0 - 2.0 * n[1] * n[1],
            -2.0 * n[1] * n[2],
        ],
        [
            -2.0 * n[2] * n[0],
            -2.0 * n[2] * n[1],
            1.0 - 2.0 * n[2] * n[2],
        ],
    ];
    let dot = mirror.point_mm[0] * n[0] + mirror.point_mm[1] * n[1] + mirror.point_mm[2] * n[2];
    let c = [2.0 * dot * n[0], 2.0 * dot * n[1], 2.0 * dot * n[2]];
    Ok((a, c))
}

/// The placement of the mirrored element: `R' = A·R·L`, `t' = A·t + c`, where `L` is the local
/// body symmetry (see the module table). The result is validated, so a mirror that would leave
/// the coordinate range is `LIMIT_EXCEEDED`.
pub fn mirror_placement(
    placement: &Placement,
    mirror: &ElementMirror,
    local_symmetry: &Mat3,
) -> Result<Placement, CommandError> {
    let (a, c) = world_mirror(mirror)?;
    let rotation = quat_to_matrix(placement.orientation_xyzw);
    let next = Placement {
        translation_mm: {
            let moved = apply(&a, placement.translation_mm);
            [moved[0] + c[0], moved[1] + c[1], moved[2] + c[2]]
        },
        orientation_xyzw: quat_from_matrix(&multiply(&a, &multiply(&rotation, local_symmetry))),
    };
    next.validate()?;
    Ok(next)
}

/// The plate's local body symmetry: the reflection in the local `z = 0` plane.
pub const PLATE_SYMMETRY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];

/// The straight member's local body symmetry: the reflection across the plane perpendicular to
/// the axis through the local origin. `None` when the path is not exactly two points.
pub fn member_symmetry(axis_points_mm: &[[f64; 3]]) -> Option<Mat3> {
    let (start, end) = (axis_points_mm.first()?, axis_points_mm.get(1)?);
    if axis_points_mm.len() != 2 {
        return None;
    }
    let u = unit_or_z([end[0] - start[0], end[1] - start[1], end[2] - start[2]]);
    Some([
        [
            1.0 - 2.0 * u[0] * u[0],
            -2.0 * u[0] * u[1],
            -2.0 * u[0] * u[2],
        ],
        [
            -2.0 * u[1] * u[0],
            1.0 - 2.0 * u[1] * u[1],
            -2.0 * u[1] * u[2],
        ],
        [
            -2.0 * u[2] * u[0],
            -2.0 * u[2] * u[1],
            1.0 - 2.0 * u[2] * u[2],
        ],
    ])
}

/// The cuts reflected by the local symmetry `L`: reflected points and normals, the kept side
/// unchanged (a reflection maps the kept half-space onto the reflected plane's own side), the
/// record identities untouched.
pub fn reflect_cuts(cuts: &[CutRecord], local_symmetry: &Mat3) -> Vec<CutRecord> {
    cuts.iter()
        .map(|record| CutRecord {
            point_mm: apply(local_symmetry, record.point_mm),
            normal: apply(local_symmetry, record.normal),
            ..*record
        })
        .collect()
}

/// The plate's mirrored authored state: the thickness side flips (the solid about the contour
/// plane trades sides; the middle is its own image) and the cuts reflect. The contour, holes
/// and corner treatments are through-thickness or in-plane and stay. A plate with bend
/// records (A10) refuses: the curl of a fold or roll about the mid-plane reflection is not
/// expressible in the authored fields (`UNSUPPORTED_GEOMETRY`).
pub fn mirror_plate_local(plate: &mut AuthoredPlate) -> Result<(), CommandError> {
    if !plate.bends.is_empty() {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "mirror",
            "a bent plate's folds and rolls have no expressible mirrored curl",
        ));
    }
    plate.thickness_side = match plate.thickness_side {
        Some(PlateSide::Middle) => Some(PlateSide::Middle),
        Some(PlateSide::Bottom) => Some(PlateSide::Top),
        // The absent side reads as the top, so its image is the explicit bottom.
        Some(PlateSide::Top) | None => Some(PlateSide::Bottom),
    };
    plate.cuts = reflect_cuts(&plate.cuts, &PLATE_SYMMETRY);
    Ok(())
}

/// The straight member's mirrored authored state: the path and the cuts reflect, and the
/// section placement is re-encoded for the reversed sweep direction: the end offsets stay at
/// their ends (the kernel's net length is asymmetric in them and the reflected path keeps the
/// start anchor), the across-width position and offset negate as a pair (the sweep frame's
/// `X_m` flips when the path direction reverses), and the roll negates (it is defined about the
/// sweep direction). A polybeam or a curved member refuses: its path has no expressible mirror
/// symmetry (`UNSUPPORTED_GEOMETRY`).
pub fn mirror_member_local(member: &mut AuthoredMember) -> Result<(), CommandError> {
    if member.curved || member.axis_points_mm.len() != 2 {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "mirror",
            "a polybeam or curved member has no expressible mirrored path",
        ));
    }
    // A04.2 member holes are polygons in a face's own frame; their mirrored placement is not
    // expressible without a face-aware encoding, so a member with hole records refuses.
    if !member.hole_records.is_empty() {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "mirror",
            "a member with hole records has no expressible mirrored holes",
        ));
    }
    let symmetry = member_symmetry(&member.axis_points_mm)
        .ok_or_else(|| degenerate("mirror", "the member path must have two points"))?;
    member.axis_points_mm = member
        .axis_points_mm
        .iter()
        .map(|p| apply(&symmetry, *p))
        .collect();
    member.cuts = reflect_cuts(&member.cuts, &symmetry);
    if let Some(section) = &mut member.section_placement {
        section.plane = match section.plane {
            crate::model::command::SectionPlanePosition::Left => {
                crate::model::command::SectionPlanePosition::Right
            }
            crate::model::command::SectionPlanePosition::Right => {
                crate::model::command::SectionPlanePosition::Left
            }
            crate::model::command::SectionPlanePosition::Centre => {
                crate::model::command::SectionPlanePosition::Centre
            }
        };
        section.plane_offset_mm = -section.plane_offset_mm;
    }
    member.roll_rad = member.roll_rad.map(|roll| -roll);
    Ok(())
}

/// The placement of array copy `index` (counting from one) of a source placement.
fn stepped_placement(
    source: &Placement,
    step: &ArrayStep,
    index: usize,
) -> Result<Placement, CommandError> {
    let distance = index as f64;
    match *step {
        ArrayStep::Linear {
            direction,
            spacing_mm,
        } => {
            let offset = [
                source.translation_mm[0] + direction[0] * spacing_mm * distance,
                source.translation_mm[1] + direction[1] * spacing_mm * distance,
                source.translation_mm[2] + direction[2] * spacing_mm * distance,
            ];
            let next = Placement {
                translation_mm: offset,
                orientation_xyzw: source.orientation_xyzw,
            };
            next.validate()?;
            Ok(next)
        }
        ArrayStep::Polar {
            point_mm,
            axis,
            angle_rad,
        } => {
            // The cumulative angle is derived, not authored input: the step cap bounds one
            // angle, so a sweep past a full turn wraps instead of refusing a valid placement.
            let turns = angle_rad * distance;
            let wrapped = turns % core::f64::consts::TAU;
            rotate_placement(
                source,
                &ElementRotate {
                    pivot_mm: point_mm,
                    axis,
                    angle_rad: wrapped,
                },
            )
        }
    }
}

/// The array's copy placements: copy `i` takes the step `i` times. A copy whose placement
/// leaves the coordinate range refuses the whole array with `LIMIT_EXCEEDED`.
pub fn array_placements(
    source: &Placement,
    step: &ArrayStep,
    count: usize,
) -> Result<Vec<Placement>, CommandError> {
    (1..=count)
        .map(|index| stepped_placement(source, step, index))
        .collect()
}

/// The authored plates of the array: independent duplicates with fresh identities and freshly
/// minted records, each at its stepped placement. A linear array moves the copies; a polar
/// array rotates them about the array axis.
pub fn array_plate_copies(
    source: &AuthoredPlate,
    command: &ArrayElements,
) -> Result<Vec<AuthoredPlate>, CommandError> {
    let placements = array_placements(&source.placement, &command.step, command.new_ids.len())?;
    command
        .new_ids
        .iter()
        .zip(placements)
        .map(|(new_id, placement)| {
            let mut copy = crate::engine::copy::copy_plate(
                source,
                &crate::model::command::CopyElement {
                    source_id: command.source_id,
                    new_id: *new_id,
                    name: command.name.clone(),
                    offset_mm: [0.0; 3],
                },
            )?;
            copy.placement = placement;
            Ok(copy)
        })
        .collect()
}

/// The authored members of the array; see [`array_plate_copies`].
pub fn array_member_copies(
    source: &AuthoredMember,
    command: &ArrayElements,
) -> Result<Vec<AuthoredMember>, CommandError> {
    let placements = array_placements(&source.placement, &command.step, command.new_ids.len())?;
    command
        .new_ids
        .iter()
        .zip(placements)
        .map(|(new_id, placement)| {
            let mut copy = crate::engine::copy::copy_member(
                source,
                &crate::model::command::CopyElement {
                    source_id: command.source_id,
                    new_id: *new_id,
                    name: command.name.clone(),
                    offset_mm: [0.0; 3],
                },
            )?;
            copy.placement = placement;
            Ok(copy)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::command::{BendRecord, BendSpec, KeepSide};
    use beamcraft_model::id::{ElementId, OpId};

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn rotate(pivot: [f64; 3], axis: [f64; 3], angle: f64) -> ElementRotate {
        ElementRotate {
            pivot_mm: pivot,
            axis,
            angle_rad: angle,
        }
    }

    fn mirror(point: [f64; 3], normal: [f64; 3]) -> ElementMirror {
        ElementMirror {
            point_mm: point,
            normal,
        }
    }

    #[test]
    fn a_quarter_turn_about_z_moves_the_translation_and_turns_the_orientation() {
        let placed = Placement {
            translation_mm: [100.0, 0.0, 5.0],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        };
        let turned = rotate_placement(
            &placed,
            &rotate([0.0; 3], [0.0, 0.0, 1.0], core::f64::consts::FRAC_PI_2),
        )
        .unwrap();
        // The translation turns with the element; Z stays.
        assert!((turned.translation_mm[0]).abs() < 1e-9);
        assert!((turned.translation_mm[1] - 100.0).abs() < 1e-9);
        assert!((turned.translation_mm[2] - 5.0).abs() < 1e-9);
        // The orientation is the quarter turn, up to the sign convention of q and -q.
        let [x, y, z, w] = turned.orientation_xyzw;
        assert!(z.abs() - core::f64::consts::FRAC_1_SQRT_2 < 1e-9);
        assert!(w.abs() - core::f64::consts::FRAC_1_SQRT_2 < 1e-9);
        assert!(x.abs() < 1e-9 && y.abs() < 1e-9);
    }

    #[test]
    fn a_second_quarter_turn_stays_inside_the_unit_quaternion_bound() {
        // An element already at the quarter turn, composed with another quarter turn: the raw
        // Hamilton product has z = `2·(√2/2)²` = 1.0000000000000002 in f64, which the strict
        // stored component bound (DV-LIM-003) must reject as stored data but the rotation step
        // must never produce. The input constants are literals, so the overshoot is exact on
        // every platform regardless of the libm behind the step quaternion.
        let quarter = Placement {
            translation_mm: [0.0, 1000.0, 0.0],
            orientation_xyzw: [
                0.0,
                0.0,
                core::f64::consts::FRAC_1_SQRT_2,
                core::f64::consts::FRAC_1_SQRT_2,
            ],
        };
        let turned = rotate_placement(
            &quarter,
            &rotate([0.0; 3], [0.0, 0.0, 1.0], core::f64::consts::FRAC_PI_2),
        )
        .unwrap();
        Placement::validate_orientation(turned.orientation_xyzw).unwrap();
        let [x, y, z, w] = turned.orientation_xyzw;
        assert!(x.abs() < 1e-9 && y.abs() < 1e-9);
        assert!((z - 1.0).abs() < 1e-9);
        assert!(w.abs() < 1e-9);
        // The translation turns with it: (0, 1000, 0) becomes (-1000, 0, 0).
        assert!((turned.translation_mm[0] + 1000.0).abs() < 1e-6);
        assert!(turned.translation_mm[1].abs() < 1e-6);
    }

    #[test]
    fn a_rotation_about_a_pivot_keeps_the_pivot_fixed() {
        let quarter = 0.25_f64;
        let norm = (0.3 * 0.3 + 0.4 * 0.4 + 0.8 * 0.8 + quarter * quarter).sqrt();
        let placed = Placement {
            translation_mm: [2000.0, -300.0, 40.0],
            orientation_xyzw: [0.3 / norm, 0.4 / norm, 0.8 / norm, quarter / norm],
        };
        let pivot = [1500.0, 0.0, 0.0];
        for angle in [0.5, -1.25, 2.0] {
            let turned = rotate_placement(&placed, &rotate(pivot, [0.0, 1.0, 0.0], angle)).unwrap();
            // The pivot is on the axis: a point at the pivot stays there. The translation two
            // pivot lengths away turns about it, so only the distance is preserved here.
            let d = |p: [f64; 3]| {
                (p[0] - pivot[0]).powi(2) + (p[1] - pivot[1]).powi(2) + (p[2] - pivot[2]).powi(2)
            };
            assert!((d(turned.translation_mm) - d(placed.translation_mm)).abs() < 1e-6);
        }
        turned_zero_axis_refuses(placed);
    }

    fn turned_zero_axis_refuses(placed: Placement) {
        assert_eq!(
            rotate_placement(&placed, &rotate([0.0; 3], [0.0; 3], 1.0))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            rotate_placement(&placed, &rotate([f64::NAN; 3], [0.0, 0.0, 1.0], 1.0))
                .unwrap_err()
                .code,
            ErrorCode::NonFiniteNumber
        );
    }

    #[test]
    fn a_mirror_of_an_identity_placement_mirrors_the_translation_and_flips_the_orientation() {
        let placed = Placement {
            translation_mm: [100.0, 20.0, 30.0],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        };
        let mirrored =
            mirror_placement(&placed, &mirror([0.0; 3], [1.0, 0.0, 0.0]), &PLATE_SYMMETRY).unwrap();
        assert!((mirrored.translation_mm[0] + 100.0).abs() < 1e-9);
        assert!((mirrored.translation_mm[1] - 20.0).abs() < 1e-9);
        assert!((mirrored.translation_mm[2] - 30.0).abs() < 1e-9);
        // The mirrored identity is the X-facing half turn composed with the local flip: the
        // matrix path rebuilds a valid quaternion either way.
        Placement::validate_orientation(mirrored.orientation_xyzw).unwrap();
        // Mirroring twice is the identity.
        let back = mirror_placement(
            &mirrored,
            &mirror([0.0; 3], [1.0, 0.0, 0.0]),
            &PLATE_SYMMETRY,
        )
        .unwrap();
        assert!((back.translation_mm[0] - 100.0).abs() < 1e-6);
        assert!((back.translation_mm[1] - 20.0).abs() < 1e-6);
    }

    #[test]
    fn a_mirror_about_an_offorigin_plane_keeps_plane_points_fixed() {
        let placed = Placement {
            translation_mm: [600.0, 0.0, 0.0],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        };
        // The plane x = 500: a point on it stays, one 100 short of it moves 100 past it.
        let mirrored = mirror_placement(
            &placed,
            &mirror([500.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            &PLATE_SYMMETRY,
        )
        .unwrap();
        assert!((mirrored.translation_mm[0] - 400.0).abs() < 1e-9);
        let on_plane = mirror_placement(
            &Placement {
                translation_mm: [500.0, 7000.0, -20.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
            &mirror([500.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            &PLATE_SYMMETRY,
        )
        .unwrap();
        assert!((on_plane.translation_mm[0] - 500.0).abs() < 1e-9);
    }

    #[test]
    fn a_zero_mirror_normal_refuses() {
        let placed = Placement::IDENTITY;
        assert_eq!(
            mirror_placement(&placed, &mirror([0.0; 3], [0.0; 3]), &PLATE_SYMMETRY)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            mirror_placement(
                &placed,
                &mirror([f64::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]),
                &PLATE_SYMMETRY
            )
            .unwrap_err()
            .code,
            ErrorCode::NonFiniteNumber
        );
    }

    #[test]
    fn reflected_cuts_keep_their_side_and_identity_and_reflect_their_plane() {
        let record = CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000aa").unwrap(),
            ordinal: 2,
            point_mm: [250.0, 0.0, 7.0],
            normal: [0.0, 0.0, 1.0],
            keep: KeepSide::Negative,
        };
        let reflected = reflect_cuts(&[record], &PLATE_SYMMETRY);
        assert_eq!(reflected.len(), 1);
        assert_eq!(reflected[0].operation_id, record.operation_id);
        assert_eq!(reflected[0].keep, KeepSide::Negative);
        assert_eq!(reflected[0].point_mm, [250.0, 0.0, -7.0]);
        assert_eq!(reflected[0].normal, [0.0, 0.0, -1.0]);
    }

    #[test]
    fn a_linear_array_steps_the_translation_and_validates_the_range() {
        let step = ArrayStep::Linear {
            direction: [1.0, 0.0, 0.0],
            spacing_mm: 500.0,
        };
        let placements = array_placements(&Placement::IDENTITY, &step, 3).unwrap();
        assert_eq!(placements.len(), 3);
        assert_eq!(placements[0].translation_mm, [500.0, 0.0, 0.0]);
        assert_eq!(placements[2].translation_mm, [1500.0, 0.0, 0.0]);
        // A copy that leaves the coordinate range refuses the whole array.
        let far = ArrayStep::Linear {
            direction: [1.0, 0.0, 0.0],
            spacing_mm: 900_000_000.0,
        };
        assert_eq!(
            array_placements(&Placement::IDENTITY, &far, 3)
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn a_polar_array_turns_each_copy_about_the_axis() {
        let step = ArrayStep::Polar {
            point_mm: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            angle_rad: core::f64::consts::FRAC_PI_2,
        };
        let source = Placement {
            translation_mm: [1000.0, 0.0, 0.0],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        };
        let placements = array_placements(&source, &step, 4).unwrap();
        assert!((placements[0].translation_mm[1] - 1000.0).abs() < 1e-9);
        assert!((placements[1].translation_mm[0] + 1000.0).abs() < 1e-9);
        assert!((placements[3].translation_mm[0] - 1000.0).abs() < 1e-6);
        assert!((placements[3].translation_mm[1]).abs() < 1e-6);
        // A sweep past a full turn is valid: the cumulative angle wraps, it is not capped as
        // authored input would be (5 x 80 degrees would refuse without the wrap).
        let wide = ArrayStep::Polar {
            point_mm: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            angle_rad: 80.0_f64.to_radians(),
        };
        let placements = array_placements(&source, &wide, 5).unwrap();
        let fifth = &placements[4];
        // 5 x 80 = 400 degrees = 40 degrees past the start.
        let (x, y) = (fifth.translation_mm[0], fifth.translation_mm[1]);
        assert!(
            (x - 1000.0 * 40.0_f64.to_radians().cos()).abs() < 1e-6,
            "{x}"
        );
        assert!(
            (y - 1000.0 * 40.0_f64.to_radians().sin()).abs() < 1e-6,
            "{y}"
        );
    }

    fn plate() -> AuthoredPlate {
        AuthoredPlate {
            id: id(5),
            name: "Plate".into(),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        }
    }

    #[test]
    fn a_mirrored_plate_flips_the_side_and_keeps_the_records() {
        let mut source = plate();
        source.cuts = vec![CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000aa").unwrap(),
            ordinal: 0,
            point_mm: [500.0, 0.0, 3.0],
            normal: [0.0, 0.0, 1.0],
            keep: KeepSide::Positive,
        }];
        mirror_plate_local(&mut source).unwrap();
        // The default top side becomes the explicit bottom; the contour is untouched.
        assert_eq!(source.thickness_side, Some(PlateSide::Bottom));
        assert_eq!(source.contour_points_mm, plate().contour_points_mm);
        assert_eq!(source.cuts[0].point_mm, [500.0, 0.0, -3.0]);
        assert_eq!(source.cuts[0].normal, [0.0, 0.0, -1.0]);
        assert_eq!(source.cuts[0].keep, KeepSide::Positive);
        // The middle side is its own image.
        let mut middle = plate();
        middle.thickness_side = Some(PlateSide::Middle);
        mirror_plate_local(&mut middle).unwrap();
        assert_eq!(middle.thickness_side, Some(PlateSide::Middle));
        // An explicit bottom flips back to the explicit top.
        let mut bottom = plate();
        bottom.thickness_side = Some(PlateSide::Bottom);
        mirror_plate_local(&mut bottom).unwrap();
        assert_eq!(bottom.thickness_side, Some(PlateSide::Top));
        // A bent plate refuses: the fold and roll curl has no expressible mirrored form.
        let mut bent = plate();
        bent.bends = vec![BendRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000ac").unwrap(),
            spec: BendSpec::Fold {
                a_mm: [250.0, 0.0, 0.0],
                b_mm: [250.0, 500.0, 0.0],
                radius_mm: 5.0,
                angle_rad: 1.2,
            },
        }];
        assert_eq!(
            mirror_plate_local(&mut bent).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
    }

    fn member() -> AuthoredMember {
        use crate::model::command::MemberKind;
        AuthoredMember {
            curved: false,
            id: id(6),
            kind: MemberKind::Beam,
            name: "Beam".into(),
            class: 0,
            axis_points_mm: vec![[0.0; 3], [1000.0, 0.0, 0.0]],
            profile: beamcraft_model::digest::Sha256::digest(b"profile").unwrap(),
            roll_rad: None,
            section_placement: None,
            placement: Placement::IDENTITY,
            material: None,
            cuts: Vec::new(),
            hole_records: Vec::new(),
        }
    }

    #[test]
    fn a_mirrored_straight_member_reflects_its_path_and_cuts() {
        let mut source = member();
        source.axis_points_mm = vec![[100.0, 0.0, 0.0], [1100.0, 0.0, 0.0]];
        source.cuts = vec![CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000ab").unwrap(),
            ordinal: 0,
            point_mm: [600.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        }];
        mirror_member_local(&mut source).unwrap();
        // The path reflects across the plane through the local origin perpendicular to the
        // axis: the far end goes to the near side.
        assert_eq!(
            source.axis_points_mm,
            vec![[-100.0, 0.0, 0.0], [-1100.0, 0.0, 0.0]]
        );
        assert_eq!(source.cuts[0].point_mm, [-600.0, 0.0, 0.0]);
        assert_eq!(source.cuts[0].normal, [-1.0, 0.0, 0.0]);
        assert_eq!(source.cuts[0].keep, KeepSide::Negative);
    }

    #[test]
    fn a_member_mirror_reencodes_the_section_for_the_reversed_sweep() {
        use crate::model::command::{SectionDepthPosition, SectionPlacement, SectionPlanePosition};
        let mut source = member();
        source.roll_rad = Some(0.4);
        source.section_placement = Some(SectionPlacement {
            plane: SectionPlanePosition::Left,
            plane_offset_mm: 5.0,
            depth: SectionDepthPosition::Centre,
            depth_offset_mm: 3.0,
            start_offset_mm: 100.0,
            end_offset_mm: -50.0,
        });
        mirror_member_local(&mut source).unwrap();
        let section = source.section_placement.unwrap();
        // The net length is asymmetric in the end offsets and the reflected path keeps the
        // start anchor, so the offsets stay at their ends.
        assert_eq!(section.start_offset_mm, 100.0);
        assert_eq!(section.end_offset_mm, -50.0);
        // The sweep frame's X_m flips with the direction: the across-width pair negates and
        // the depth (Y_m) stays.
        assert_eq!(section.plane, SectionPlanePosition::Right);
        assert_eq!(section.plane_offset_mm, -5.0);
        assert_eq!(section.depth_offset_mm, 3.0);
        // The roll is defined about the sweep direction, which reverses.
        assert_eq!(source.roll_rad, Some(-0.4));
    }

    #[test]
    fn a_polybeam_and_a_curved_member_refuse_the_mirror() {
        let mut bent = member();
        bent.axis_points_mm = vec![[0.0; 3], [1000.0, 0.0, 0.0], [1000.0, 500.0, 0.0]];
        assert_eq!(
            mirror_member_local(&mut bent).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
        let mut curved = member();
        curved.curved = true;
        assert_eq!(
            mirror_member_local(&mut curved).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
        // A04.2 member holes have no expressible mirrored placement.
        let mut drilled = member();
        drilled.hole_records = vec![crate::model::command::MemberHoleRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000ad").unwrap(),
            ordinal: 0,
            axis_point_mm: [500.0, 20.0, 0.0],
            axis_direction: [0.0, 1.0, 0.0],
            extent: crate::model::command::HoleExtent::Through,
            polygon_mm: vec![[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]],
        }];
        assert_eq!(
            mirror_member_local(&mut drilled).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
    }

    #[test]
    fn the_array_copies_are_independent_duplicates_at_stepped_placements() {
        let command = ArrayElements {
            source_id: id(5),
            name: Some("Rung".into()),
            new_ids: vec![id(6), id(7)],
            step: ArrayStep::Linear {
                direction: [0.0, 1.0, 0.0],
                spacing_mm: 600.0,
            },
        };
        let copies = array_plate_copies(&plate(), &command).unwrap();
        assert_eq!(copies.len(), 2);
        assert_eq!(copies[0].id, id(6));
        assert_eq!(copies[1].id, id(7));
        assert_eq!(copies[0].placement.translation_mm, [0.0, 600.0, 0.0]);
        assert_eq!(copies[1].placement.translation_mm, [0.0, 1200.0, 0.0]);
        assert_eq!(copies[0].name, "Rung");
        // The copies share the shape inputs and stay independent duplicates.
        assert_eq!(copies[0].contour_points_mm, plate().contour_points_mm);
        assert_eq!(copies[1].contour_points_mm, plate().contour_points_mm);
        let members = array_member_copies(&member(), &command).unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[1].id, id(7));
        assert_eq!(members[1].placement.translation_mm, [0.0, 1200.0, 0.0]);
    }
}
