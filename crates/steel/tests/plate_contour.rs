// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Contributor-original ordered contour regressions; references are analytic rectangles.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_model::error::ErrorCode;
use steel::kernel::plate_contour::{admit, rectangular_body};

#[test]
fn ordered_rectangle_frame_and_vertices_follow_the_first_edge() {
    let points = [
        [10., 20., 30.],
        [10., 1020., 30.],
        [-490., 1020., 30.],
        [-490., 20., 30.],
    ];
    let contour = admit(&points, 10.).unwrap();
    let frame = contour.frame.unwrap();
    assert_eq!(frame.x().components(), [0., 1., 0.]);
    assert_eq!(frame.y().components(), [-1., 0., 0.]);
    assert_eq!(frame.z().components(), [0., 0., 1.]);
    assert_eq!(contour.rectangle_mm, Some([1000., 500.]));
    let result = rectangular_body(&points, 10.).unwrap();
    let actual: Vec<_> = result
        .mesh()
        .vertices()
        .iter()
        .map(|p| [p.x, p.y, p.z])
        .collect();
    assert_eq!(
        actual,
        vec![
            [10., 20., 30.],
            [10., 1020., 30.],
            [-490., 20., 30.],
            [-490., 1020., 30.],
            [10., 20., 40.],
            [10., 1020., 40.],
            [-490., 20., 40.],
            [-490., 1020., 40.]
        ]
    );
    assert_eq!(result.volume_mm3(), 5_000_000.);
    assert_eq!(result, rectangular_body(&points, 10.).unwrap());
}

#[test]
fn oblique_and_vertical_rectangles_extrude_along_the_ordered_normal() {
    for points in [
        [
            [10., 20., 30.],
            [13., 24., 30.],
            [9., 27., 30.],
            [6., 23., 30.],
        ],
        [
            [10., 20., 30.],
            [10., 23., 34.],
            [10., 19., 37.],
            [10., 16., 33.],
        ],
        [
            [10., 20., 30.],
            [13., 20., 34.],
            [13., 15., 34.],
            [10., 15., 30.],
        ],
    ] {
        let result = rectangular_body(&points, 2.).unwrap();
        assert_eq!(admit(&points, 2.).unwrap().rectangle_mm, Some([5., 5.]));
        assert_eq!(result.volume_mm3(), 50.);
        // All eight vertices make one oriented closed shell, independently measured.
        let v: Vec<_> = result
            .mesh()
            .vertices()
            .iter()
            .map(|p| [p.x, p.y, p.z])
            .collect();
        let mut six_volume = 0.;
        for [a, b, c] in result.mesh().triangles() {
            let local = |i: u32| std::array::from_fn::<_, 3, _>(|k| v[i as usize][k] - v[0][k]);
            let (a, b, c) = (local(*a), local(*b), local(*c));
            six_volume += a[0] * (b[1] * c[2] - b[2] * c[1])
                + a[1] * (b[2] * c[0] - b[0] * c[2])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
        }
        assert!((six_volume / 6. - 50.).abs() < 1e-10);
    }
}

#[test]
fn scaled_planarity_is_admission_not_healing_or_rectangle_support() {
    let saddle = |s| {
        [
            [999000000., 0., 0.],
            [999393216., 0., s],
            [999393216., 524288., 0.],
            [999000000., 524288., s],
        ]
    };
    let valid = admit(&saddle(0.75), 10.).unwrap();
    assert_eq!(valid.tolerance.local_size_mm(), 655360.);
    assert_eq!(valid.tolerance.eps_geom_mm(), 0.15625);
    assert!(!valid.exactly_planar);
    assert_eq!(valid.frame, None);
    assert_eq!(valid.rectangle_mm, None);
    assert_eq!(
        rectangular_body(&saddle(0.75), 10.).unwrap_err().code,
        ErrorCode::UnsupportedGeometry
    );
    assert_eq!(
        admit(&saddle(0.875), 10.).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    let band: f64 = 0.15625 + 1e-6 * 655360.0;
    assert!(admit(&saddle(band), 10.).is_ok());
    assert_eq!(
        admit(&saddle(f64::from_bits(band.to_bits() + 1)), 10.)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
}

#[test]
fn wrong_3d_contours_are_invalid_before_support_is_decided() {
    for points in [
        vec![
            [0., 0., 30.],
            [0., 500., 30.],
            [1000., 500., 30.],
            [1000., 0., 30.],
        ], // clockwise
        vec![
            [0., 0., 0.],
            [1000., 0., 0.],
            [1000., 500., 20.],
            [0., 500., 0.],
        ], // nonplanar
        vec![
            [0., 0., 30.],
            [8., 0., 30.],
            [4., 0., 30.],
            [4., 4., 30.],
            [0., 4., 30.],
        ], // backtrack
        vec![
            [0., 0., 30.],
            [8., 0., 30.],
            [8., 8., 30.],
            [4., 0., 30.],
            [0., 8., 30.],
        ], // touch
        vec![[0., 0., 30.], [8., 8., 30.], [0., 8., 30.], [8., 0., 30.]], // zero N bowtie
    ] {
        assert_eq!(
            admit(&points, 1.).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
    assert_eq!(
        admit(&vec![[0.; 3]; 1025], 1.).unwrap_err().code,
        ErrorCode::LimitExceeded
    );
    assert_eq!(
        admit(&[[0.; 3], [1., 0., 0.], [0., 1., f64::NAN]], 1.)
            .unwrap_err()
            .code,
        ErrorCode::NonFiniteNumber
    );
}

#[test]
fn dominant_area_ties_and_tiny_coordinates_use_exact_submitted_values() {
    // N=(-2,0,+2): the tie must pick positive Z, not negative X.
    let points = [[0., 0., 0.], [1., 0., 1.], [1., 1., 1.], [0., 1., 0.]];
    assert!(admit(&points, 1.).unwrap().exactly_planar);
    let mut reversed = points;
    reversed.reverse();
    assert_eq!(
        admit(&reversed, 1.).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    // N=(+2,-2,0): the tie must pick negative Y, not positive X.
    let points = [[0., 0., 0.], [1., 1., 0.], [1., 1., 1.], [0., 0., 1.]];
    assert_eq!(
        admit(&points, 1.).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    // Subnormal offsets exercise the fixed-capacity exact fallback without dropping a coordinate.
    let tiny = f64::from_bits(1);
    let points = [
        [0., 0., tiny],
        [8., 0., tiny],
        [8., 4., tiny],
        [0., 4., tiny],
    ];
    let contour = admit(&points, 1.).unwrap();
    assert!(contour.exactly_planar);
    assert_eq!(contour.origin.z, tiny);
    assert_eq!(rectangular_body(&points, 1.).unwrap().volume_mm3(), 32.);
}
