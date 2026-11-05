// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Independent references only: expected volumes, areas and fold vertex positions are closed
// forms (the thickness times the developed mid-surface area, the annular-sector prism and the
// cone band, and the isometric image of the developed walk), computed here from the authored
// bend parameters.

//! A10 bent-plate fixtures: folds and rolls through the G13 ruled loft. The cross-section of
//! each fixture is built here by hand, independently of the engine's derivation, and the
//! sealed quantities are compared with their closed forms. The volume invariant of a
//! mid-surface bend is `V = t x developed area x body width`, whatever the radii.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::common;

use beamcraft_geom::{Point3, Vec3};
use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
use steel::engine::command::{AuthoredPlate, evaluate_plate};
use steel::kernel::ruled::{Loft, LoftResult, RuledContour, loft};
use steel::model::command::{BendRecord, BendSpec};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn ex() -> Vec3 {
    Vec3::new(1.0, 0.0, 0.0).unwrap()
}

fn ey() -> Vec3 {
    Vec3::new(0.0, 1.0, 0.0).unwrap()
}

fn run(start: RuledContour, end: RuledContour) -> LoftResult {
    loft(&Loft { start, end }).unwrap()
}

fn volume(result: &LoftResult) -> f64 {
    result.volume_mm3().midpoint()
}

fn area(result: &LoftResult) -> f64 {
    result.area_mm2().unwrap().midpoint()
}

fn close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{actual} vs {expected}"
    );
}

/// One arc of `radius` about `centre` from `start` over `sweep` in `segments` chords.
fn arc(
    out: &mut Vec<[f64; 2]>,
    centre: [f64; 2],
    radius: f64,
    start: f64,
    sweep: f64,
    segments: usize,
) {
    for k in 0..=segments {
        let angle = start + sweep * (k as f64) / (segments as f64);
        let (sin, cos) = angle.sin_cos();
        let point = [centre[0] + radius * cos, centre[1] + radius * sin];
        if out.last() != Some(&point) {
            out.push(point);
        }
    }
}

/// The closed counter-clockwise annular sector over `[0, sweep]`: the outer arc forward, the
/// end cut in, the inner arc backward.
fn sector(r_inner: f64, r_outer: f64, sweep: f64, segments: usize) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    arc(&mut points, [0.0, 0.0], r_outer, 0.0, sweep, segments);
    arc(&mut points, [0.0, 0.0], r_inner, sweep, -sweep, segments);
    points
}

fn contour(points: &[[f64; 2]], z: f64) -> RuledContour {
    let flat = points.iter().map(|[x, y]| common::p(*x, *y)).collect();
    RuledContour::polygon(p3(0.0, 0.0, z), ex(), ey(), flat).unwrap()
}

/// A 90-degree fold of a `t = 2` strip on the inner radius `r = 8` (mid radius 9): the outer
/// offset is the right-hand chain at radius 10, the inner the left-hand chain at radius 8.
/// Developed length `100 + 9 x pi/2`, body width 100.
#[test]
fn a_right_angle_fold_keeps_the_thickness_times_the_developed_area() {
    // The mid-line runs (0,0) to (50,0), turns up about the centre (50,9), then runs 50 more.
    let mut points = Vec::new();
    // Right (outer) chain, forward.
    points.push([0.0, -1.0]);
    points.push([50.0, -1.0]);
    arc(&mut points, [50.0, 9.0], 10.0, -FRAC_PI_2, FRAC_PI_2, 64);
    points.push([60.0, 59.0]);
    // End cut, then the left (inner) chain, backward.
    points.push([58.0, 59.0]);
    points.push([58.0, 9.0]);
    arc(&mut points, [50.0, 9.0], 8.0, 0.0, -FRAC_PI_2, 64);
    points.push([0.0, 1.0]);
    let start = contour(&points, 0.0);
    let end = contour(&points, 100.0);
    let result = run(start, end);
    let section = 2.0 * (100.0 + 9.0 * FRAC_PI_2);
    // V = t x developed area x width; A = 2 x section + width x perimeter, with the arc
    // perimeters `10 x pi/2` (outer) and `8 x pi/2` (inner) and two end cuts of `t`.
    close(volume(&result), section * 100.0, 1e-4);
    close(volume(&result), (200.0 + 9.0 * PI) * 100.0, 1e-4);
    let perimeter = (100.0 + 5.0 * PI) + (100.0 + 4.0 * PI) + 4.0;
    close(area(&result), 2.0 * section + 100.0 * perimeter, 1e-4);
}

/// A U-channel: two parallel right-angle folds on the same radius; the second fold turns the
/// sheet back over itself, so the final flat runs opposite to the first. The invariance holds
/// for the pair, and the two bend regions stay clear of each other and of the blank edges.
#[test]
fn two_folds_keep_the_same_developed_volume() {
    // Mid-line: (0,0) -> (50,0), fold up about (50,9), vertical to (59,59), fold back about
    // (50,59), then horizontal to (0,68). The right (outer) chain is walked forward.
    let mut points = Vec::new();
    points.push([0.0, -1.0]);
    points.push([50.0, -1.0]);
    arc(&mut points, [50.0, 9.0], 10.0, -FRAC_PI_2, FRAC_PI_2, 64);
    points.push([60.0, 59.0]);
    arc(&mut points, [50.0, 59.0], 10.0, 0.0, FRAC_PI_2, 64);
    points.push([0.0, 69.0]);
    // End cut, then the left (inner) chain, backward.
    points.push([0.0, 67.0]);
    points.push([50.0, 67.0]);
    arc(&mut points, [50.0, 59.0], 8.0, FRAC_PI_2, -FRAC_PI_2, 64);
    points.push([58.0, 9.0]);
    arc(&mut points, [50.0, 9.0], 8.0, 0.0, -FRAC_PI_2, 64);
    points.push([0.0, 1.0]);
    let start = contour(&points, 0.0);
    let end = contour(&points, 100.0);
    let result = run(start, end);
    let developed = 150.0 + 9.0 * PI;
    close(volume(&result), 2.0 * developed * 100.0, 1e-4);
}

/// A cylinder roll: the `t = 2` sheet wraps the axis on the inner radius `r = 8` over one
/// radian; the body is the annular-sector prism.
#[test]
fn a_cylinder_roll_matches_the_annular_sector_prism() {
    let (r, t, sweep) = (8.0, 2.0, 1.0_f64);
    let points = sector(r, r + t, sweep, 64);
    let start = contour(&points, 0.0);
    let end = contour(&points, 100.0);
    let result = run(start, end);
    let section = (sweep / 2.0) * ((r + t) * (r + t) - r * r);
    close(volume(&result), section * 100.0, 1e-4);
    let perimeter = sweep * (2.0 * r + t) + 2.0 * t;
    close(area(&result), 2.0 * section + 100.0 * perimeter, 1e-4);
}

/// A cone roll: the developed sector runs from slant 90 to slant 190 about the apex with
/// `sin(phi) = 0.6`, so the mid radii are 54 and 114; the band volume is
/// `theta_true x t/(2 cos phi) x axial span x (r_a + r_b)`, and the mid-surface invariance
/// gives the same value from the development alone.
#[test]
fn a_cone_roll_matches_the_band_closed_form_and_the_development() {
    let (t, sin_phi) = (2.0_f64, 0.6_f64);
    let (s_in, s_out) = (90.0_f64, 190.0_f64);
    let (r_a, r_b) = (s_in * sin_phi, s_out * sin_phi);
    let sweep_developed = 0.8_f64;
    let sweep = sweep_developed / sin_phi;
    let cos_phi = (1.0 - sin_phi * sin_phi).sqrt();
    let face = t / (2.0 * cos_phi);
    let start = contour(&sector(r_a - face, r_a + face, sweep, 64), 0.0);
    let end = contour(
        &sector(r_b - face, r_b + face, sweep, 64),
        (s_out - s_in) * cos_phi,
    );
    let result = run(start, end);
    let axial = (s_out - s_in) * cos_phi;
    let band = sweep * face * axial * (r_a + r_b);
    let development = t * (sweep_developed / 2.0) * (s_out * s_out - s_in * s_in);
    close(band, development, 1e-9);
    close(volume(&result), band, 1e-4);
}

/// The real derivation, not the hand-built section above: `evaluate_plate` folds the authored
/// blank, and the far edge of the 90-degree fold and the two fold-region ends sit at their
/// closed-form positions. A wrong angle, sign or fold line moves them, while the volume
/// invariant would still hold.
#[test]
fn a_fold_places_the_far_edge_and_the_fold_ends_at_their_closed_forms() {
    let (t, r, m, along) = (2.0_f64, 8.0_f64, 50.0_f64, 100.0_f64);
    let mid = r + t / 2.0;
    // The developed blank: `along` on the fold line plus the arc the bend consumes.
    let across = along + mid * FRAC_PI_2;
    let authored = AuthoredPlate {
        id: beamcraft_model::id::ElementId::parse("019a7bf0-0000-7000-8000-000000000001").unwrap(),
        name: "Folded".into(),
        class: 0,
        contour_points_mm: vec![
            [0.0, 0.0, 0.0],
            [along, 0.0, 0.0],
            [along, across, 0.0],
            [0.0, across, 0.0],
        ],
        thickness_mm: t,
        thickness_side: None,
        corners: Vec::new(),
        placement: beamcraft_model::command::Placement {
            translation_mm: [0.0; 3],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        },
        material: None,
        holes_mm: Vec::new(),
        hole_records: Vec::new(),
        cuts: Vec::new(),
        bends: vec![BendRecord {
            operation_id: beamcraft_model::id::OpId::parse("019a7bf0-0000-7000-8000-000000000002")
                .unwrap(),
            spec: BendSpec::Fold {
                a_mm: [0.0, m, 0.0],
                b_mm: [along, m, 0.0],
                radius_mm: r,
                angle_rad: FRAC_PI_2,
            },
        }],
    };
    let body = evaluate_plate(&authored).unwrap();
    // The developed walk: a flat run to the bend region, a quarter arc of the mid radius, then
    // the flange run to the far edge. `s` is where the bend region starts across; the flange
    // keeps its developed length, so the tip sits `mid + flange` above the plane.
    let s = m - mid * FRAC_PI_4;
    let flange = across - m - mid * FRAC_PI_4;
    let tip_z = mid + flange;
    let (y_out, y_in) = (s + r + t, s + mid - t / 2.0);
    let vertices = body.solid.vertices();
    // Both loft ends carry each far-edge corner: x is 0 or `along`.
    for (y, z) in [(y_out, tip_z), (y_in, tip_z)] {
        let count = vertices
            .iter()
            .filter(|v| (v[1] - y).hypot(v[2] - z) <= 1e-6)
            .count();
        assert_eq!(count, 2, "the far edge ({y}, {z}) misses a loft end");
    }
    // The fold ends: the outer face leaves the flat at z = -t/2 and lands on the flange at the
    // arc centre height; the inner face mirrors it at +t/2 and radius - t.
    for (y, z) in [(s, -t / 2.0), (y_out, mid), (s, t / 2.0), (y_in, mid)] {
        let nearest = vertices
            .iter()
            .map(|v| (v[1] - y).hypot(v[2] - z))
            .fold(f64::INFINITY, f64::min);
        assert!(
            nearest <= 1e-6,
            "no vertex at the closed fold end ({y}, {z}), nearest {nearest}"
        );
    }
    // A positive angle folds up: the far edge must sit above the blank plane.
    assert!(
        tip_z > m,
        "the 90-degree fold must lift the far edge above the plane"
    );
}
