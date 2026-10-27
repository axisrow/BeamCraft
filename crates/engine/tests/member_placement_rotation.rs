// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Rigid placement rotation of a member: the composed section frame, its admission and the
//! invariants, headless over the public engine API. The expected vertices are derived by hand
//! from the documented composition order, independently of the engine's quaternion helper.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_engine::command::{create_plate, no_materials, origin_rectangle_contour};
use beamcraft_engine::member::{ProfileInfo, create_straight_member, no_profiles};
use beamcraft_model::command::{
    CommandError, CreatePlate, CreateStraightMember, MemberKind, Placement,
};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;

const MEMBER_ID: &str = "019a7bf0-0000-7000-8000-000000000007";
const PLATE_ID: &str = "019a7bf0-0000-7000-8000-000000000005";

fn digest(text: &[u8]) -> ContentDigest {
    Sha256::digest(text).unwrap()
}

fn member_command(placement: Placement, roll_rad: Option<f64>) -> CreateStraightMember {
    CreateStraightMember {
        new_id: ElementId::parse(MEMBER_ID).unwrap(),
        kind: MemberKind::Beam,
        name: "Rotated beam".into(),
        class: 0,
        axis_points_mm: [[0.0, 0.0, 0.0], [4000.0, 0.0, 0.0]],
        profile_digest: digest(b"rectangle 200x300"),
        placement,
        roll_rad,
        material_digest: None,
    }
}

fn rectangle_profile(d: &ContentDigest) -> Result<ProfileInfo, CommandError> {
    if *d == digest(b"rectangle 200x300") {
        Ok(ProfileInfo::Rectangle {
            width_mm: 200.0,
            height_mm: 300.0,
            declared_area_mm2: None,
        })
    } else {
        no_profiles(d)
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

/// The frozen F06-POS-12 pose: R_z(90 degrees) as `(x, y, z, w)`.
fn quarter_about_z() -> Placement {
    let half = core::f64::consts::FRAC_1_SQRT_2;
    Placement {
        translation_mm: [-50.0, 70.0, 1000.0],
        orientation_xyzw: [0.0, 0.0, half, half],
    }
}

#[test]
fn a_rotated_member_mesh_is_the_analytically_rotated_local_one() {
    let identity = create_straight_member(
        &member_command(Placement::IDENTITY, None),
        &|_| false,
        &rectangle_profile,
        &no_materials,
    )
    .unwrap();
    let rotated = create_straight_member(
        &member_command(quarter_about_z(), None),
        &|_| false,
        &rectangle_profile,
        &no_materials,
    )
    .unwrap();
    // R_z(90 degrees): (x, y, z) -> (-y, x, z). The rotation is part of the body; the
    // translation is not, and the vertex order is unchanged.
    for (v, r) in identity
        .body
        .solid
        .vertices()
        .iter()
        .zip(rotated.body.solid.vertices())
    {
        assert!(
            close(-v[1], r[0]) && close(v[0], r[1]) && close(v[2], r[2]),
            "local {v:?} vs rotated {r:?}"
        );
    }
    // The blob and digest follow the rotated vertices.
    assert_ne!(rotated.body.solid.digest(), identity.body.solid.digest());
}

#[test]
fn a_rotation_keeps_the_axis_length_volume_and_section_area() {
    let identity = create_straight_member(
        &member_command(Placement::IDENTITY, None),
        &|_| false,
        &rectangle_profile,
        &no_materials,
    )
    .unwrap();
    let rotated = create_straight_member(
        &member_command(quarter_about_z(), None),
        &|_| false,
        &rectangle_profile,
        &no_materials,
    )
    .unwrap();
    // 200 x 300 x 4000 closed forms, exactly as without the rotation.
    assert_eq!(rotated.body.axis_length_mm, 4000.0);
    assert_eq!(rotated.body.section_area_mm2, 60_000.0);
    assert_eq!(rotated.body.volume_mm3, 240_000_000.0);
    assert_eq!(
        rotated.body.solid.volume_mm3(),
        identity.body.solid.volume_mm3()
    );
    assert_eq!(rotated.body.width_mm, 200.0);
    assert_eq!(rotated.body.height_mm, 300.0);
    assert_eq!(rotated.quantities, identity.quantities);
}

/// Composition order, pinned by hand. The local frame of the `+X` axis is `X_m = +Y`,
/// `Y_m = +Z`; a roll of `+pi/2` about the axis maps the frame to `X_m = +Z`, `Y_m = -Y`
/// (right-handed, about the member axis *before* the placement). The placement then maps
/// `R_z(90)`, so a local vertex `(a, b, c)` becomes `(c, a, b)`: height along world `X`,
/// the axis along world `Y`, width along world `Z`.
#[test]
fn the_roll_composes_in_the_member_frame_before_the_placement() {
    let local = create_straight_member(
        &member_command(Placement::IDENTITY, None),
        &|_| false,
        &rectangle_profile,
        &no_materials,
    )
    .unwrap();
    let placed = create_straight_member(
        &member_command(quarter_about_z(), Some(core::f64::consts::FRAC_PI_2)),
        &|_| false,
        &rectangle_profile,
        &no_materials,
    )
    .unwrap();
    for (v, r) in local
        .body
        .solid
        .vertices()
        .iter()
        .zip(placed.body.solid.vertices())
    {
        let (a, b, c) = (v[0], v[1], v[2]);
        assert!(
            close(c, r[0]) && close(a, r[1]) && close(b, r[2]),
            "{v:?} vs {r:?}"
        );
    }
}

#[test]
fn a_non_normalized_or_degenerate_quaternion_is_an_invalid_argument() {
    let error_of = |orientation_xyzw: [f64; 4]| {
        let mut placement = Placement::IDENTITY;
        placement.orientation_xyzw = orientation_xyzw;
        create_straight_member(
            &member_command(placement, None),
            &|_| false,
            &rectangle_profile,
            &no_materials,
        )
        .unwrap_err()
    };
    for orientation in [
        [0.0, 0.0, 0.0, 0.5],   // F04-NEG-22: squared norm far from one.
        [0.0, 0.0, 0.0, 0.0],   // Degenerate: no rotation at all.
        [1.0, 1.0, 1.0, 1.0],   // A component outside [-1, 1].
        [0.0, 0.0, 0.0, 1.001], // Norm above one by more than the tolerance.
    ] {
        let error = error_of(orientation);
        assert_eq!(error.code, ErrorCode::InvalidArgument, "{orientation:?}");
        assert_eq!(error.field, "placement.orientation_xyzw");
    }
    let error = error_of([f64::NAN, 0.0, 0.0, 1.0]);
    assert_eq!(error.code, ErrorCode::NonFiniteNumber);
    assert_eq!(error.field, "placement.orientation_xyzw");
}

#[test]
fn a_plate_accepts_a_rigid_placement_without_changing_its_local_body() {
    let command = CreatePlate {
        new_id: ElementId::parse(PLATE_ID).unwrap(),
        name: "Rotated plate".into(),
        class: 0,
        contour_points_mm: origin_rectangle_contour(1000.0, 500.0).unwrap(),
        thickness_mm: 10.0,
        placement: quarter_about_z(),
        material_digest: None,
        holes_mm: Vec::new(),
    };
    let placed = create_plate(&command, &|_| false, &no_materials).unwrap();
    let unplaced = create_plate(
        &CreatePlate {
            placement: Placement::IDENTITY,
            ..command
        },
        &|_| false,
        &no_materials,
    )
    .unwrap();
    assert_eq!(placed.body.solid, unplaced.body.solid);
    assert_eq!(placed.quantities, unplaced.quantities);
}
