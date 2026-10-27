// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic members. Expected volumes and bounds are closed forms derived
// by hand from F06 section 4 (default frame) and section 6 (box layout), not from the code.

//! P03 straight-member builder: frame, mesh, volume and failures.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_geom::Point3;
use beamcraft_kernel::diagnostic::{Code, Field};
use beamcraft_kernel::{PlateResult, StraightMember, straight_member};
use beamcraft_model::command::SectionPlacement;

fn member(start: [f64; 3], end: [f64; 3], w: f64, h: f64, roll: Option<f64>) -> StraightMember {
    StraightMember {
        start: Point3::new(start[0], start[1], start[2]),
        end: Point3::new(end[0], end[1], end[2]),
        width_mm: w,
        height_mm: h,
        roll_rad: roll,
        placement: SectionPlacement::default(),
    }
}

/// Divergence-theorem volume of the triangle mesh, independent of the builder's formula.
fn mesh_volume(result: &PlateResult) -> f64 {
    let mesh = result.mesh();
    let v = mesh.vertices();
    mesh.triangles()
        .iter()
        .map(|[a, b, c]| {
            let (a, b, c) = (v[*a as usize], v[*b as usize], v[*c as usize]);
            let cross = [
                b.y * c.z - b.z * c.y,
                b.z * c.x - b.x * c.z,
                b.x * c.y - b.y * c.x,
            ];
            (a.x * cross[0] + a.y * cross[1] + a.z * cross[2]) / 6.0
        })
        .sum()
}

fn bounds(result: &PlateResult) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in result.mesh().vertices() {
        for (i, c) in [p.x, p.y, p.z].into_iter().enumerate() {
            lo[i] = lo[i].min(c);
            hi[i] = hi[i].max(c);
        }
    }
    (lo, hi)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

#[test]
fn a_vertical_member_is_the_section_extruded_along_z() {
    // Near-vertical rule: Y = (0,1,0), X = (1,0,0): width along X, height along Y.
    let r = straight_member(member(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 3000.0],
        200.0,
        300.0,
        None,
    ))
    .unwrap();
    assert_eq!(r.volume_mm3(), 200.0 * 300.0 * 3000.0);
    assert!(close(mesh_volume(&r), 200.0 * 300.0 * 3000.0));
    assert_eq!(bounds(&r), ([-100.0, -150.0, 0.0], [100.0, 150.0, 3000.0]));
    assert_eq!(r.mesh().vertices().len(), 8);
    assert_eq!(r.mesh().triangles().len(), 12);
    assert_eq!(r.mesh().orientation(), "rightHanded");
    assert_eq!(r.mesh().subdivision_scheme(), "none");
}

#[test]
fn a_horizontal_member_has_width_along_y_and_height_along_z() {
    // Horizontal rule along +X: Z_m = (1,0,0), Y_m = (0,0,1), X_m = Y_m x Z_m = (0,1,0).
    let r = straight_member(member(
        [100.0, 0.0, 50.0],
        [6100.0, 0.0, 50.0],
        200.0,
        300.0,
        None,
    ))
    .unwrap();
    assert_eq!(r.volume_mm3(), 200.0 * 300.0 * 6000.0);
    assert!(close(mesh_volume(&r), 200.0 * 300.0 * 6000.0));
    assert_eq!(
        bounds(&r),
        ([100.0, -100.0, -100.0], [6100.0, 100.0, 200.0])
    );
}

#[test]
fn a_quarter_turn_roll_swaps_the_section_axes() {
    let plain = straight_member(member(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1000.0],
        200.0,
        300.0,
        None,
    ))
    .unwrap();
    let rolled = straight_member(member(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1000.0],
        200.0,
        300.0,
        Some(core::f64::consts::FRAC_PI_2),
    ))
    .unwrap();
    let (lo, hi) = bounds(&rolled);
    // The quarter turn takes X to Y: the 200 mm width now spans Y and the 300 mm height X.
    for (got, want) in lo.iter().zip([-150.0, -100.0, 0.0]) {
        assert!((got - want).abs() < 1e-9, "{lo:?}");
    }
    for (got, want) in hi.iter().zip([150.0, 100.0, 1000.0]) {
        assert!((got - want).abs() < 1e-9, "{hi:?}");
    }
    assert!(close(rolled.volume_mm3(), plain.volume_mm3()));
    assert_ne!(rolled.mesh().mesh_digest(), plain.mesh().mesh_digest());
    // A zero roll and an absent roll are the same member.
    let zero = straight_member(member(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1000.0],
        200.0,
        300.0,
        Some(0.0),
    ))
    .unwrap();
    assert_eq!(zero.mesh().mesh_digest(), plain.mesh().mesh_digest());
}

#[test]
fn a_tilted_member_is_a_certified_closed_box() {
    let r = straight_member(member(
        [0.0, 0.0, 0.0],
        [2000.0, 3000.0, 6000.0],
        150.0,
        250.0,
        Some(0.3),
    ))
    .unwrap();
    // |d| = sqrt(2000^2 + 3000^2 + 6000^2) = 7000.
    assert!(close(r.volume_mm3(), 150.0 * 250.0 * 7000.0));
    assert!((mesh_volume(&r) - r.volume_mm3()).abs() <= 1e-6 * r.volume_mm3());
}

#[test]
fn the_same_input_gives_the_same_mesh_digest() {
    let m = member([0.0, 0.0, 0.0], [2500.0, 0.0, 0.0], 120.0, 240.0, None);
    assert_eq!(
        straight_member(m).unwrap().mesh().mesh_digest(),
        straight_member(m).unwrap().mesh().mesh_digest()
    );
}

fn failure(m: StraightMember) -> (Code, Field) {
    let failure = straight_member(m).unwrap_err();
    let d = failure.diagnostics().next().unwrap();
    (d.code, d.field)
}

#[test]
fn invalid_members_are_typed_failures_with_no_partial_result() {
    let ok = member([0.0; 3], [0.0, 0.0, 1000.0], 200.0, 300.0, None);
    assert_eq!(
        failure(StraightMember {
            end: ok.start,
            ..ok
        }),
        (Code::AxisTooShort, Field::Axis)
    );
    assert_eq!(
        failure(StraightMember {
            width_mm: 0.0,
            ..ok
        }),
        (Code::NotPositive, Field::Section)
    );
    assert_eq!(
        failure(StraightMember {
            height_mm: -5.0,
            ..ok
        }),
        (Code::NotPositive, Field::Section)
    );
    assert_eq!(
        failure(StraightMember {
            width_mm: f64::NAN,
            ..ok
        }),
        (Code::NonFinite, Field::Section)
    );
    assert_eq!(
        failure(StraightMember {
            end: Point3::new(0.0, f64::INFINITY, 1.0),
            ..ok
        }),
        (Code::NonFinite, Field::Axis)
    );
    assert_eq!(
        failure(StraightMember {
            height_mm: 1_000_001.0,
            ..ok
        }),
        (Code::AboveLimit, Field::Section)
    );
    assert_eq!(
        failure(StraightMember {
            roll_rad: Some(7.0),
            ..ok
        }),
        (Code::AboveLimit, Field::Roll)
    );
    assert_eq!(
        failure(StraightMember {
            end: Point3::new(0.0, 0.0, 2e9),
            ..ok
        }),
        (Code::AboveLimit, Field::Axis)
    );
    assert_eq!(
        failure(StraightMember {
            width_mm: 0.0009,
            ..ok
        }),
        (Code::BelowMinimum, Field::Section)
    );
}

#[test]
fn a_member_far_from_the_origin_keeps_its_volume() {
    // Absolute coordinates near 5e8 mm used to cancel in the signed-volume self-check.
    let far = 500_000_000.0;
    let r = straight_member(member(
        [far, far, far],
        [far, far, far + 6000.0],
        200.0,
        300.0,
        None,
    ))
    .unwrap();
    assert_eq!(r.volume_mm3(), 200.0 * 300.0 * 6000.0);
}
