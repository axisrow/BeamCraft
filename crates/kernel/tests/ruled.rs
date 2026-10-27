// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Independent references only: expected volumes and areas are closed forms (the oblique
// prism of a translated contour, the annular-sector and cone formulas) computed here, and
// the blob is reread with the G03 test reader.

//! G13 ruled-loft fixtures: a ruled surface between two contours with thickness.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod common;

use beamcraft_geom::{Point3, Vec3};
use beamcraft_kernel::ruled::{Loft, LoftCode, LoftFailure, LoftResult, RuledContour, loft};
use core::f64::consts::{FRAC_PI_2, PI, TAU};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn ex() -> Vec3 {
    Vec3::new(1.0, 0.0, 0.0).unwrap()
}

fn ey() -> Vec3 {
    Vec3::new(0.0, 1.0, 0.0).unwrap()
}

/// A rectangle of `w` x `h` centred at the origin in the XY plane at height `z`.
fn rect(w: f64, h: f64, z: f64) -> RuledContour {
    RuledContour::polygon(
        p3(0.0, 0.0, z),
        ex(),
        ey(),
        vec![
            common::p(-w / 2.0, -h / 2.0),
            common::p(w / 2.0, -h / 2.0),
            common::p(w / 2.0, h / 2.0),
            common::p(-w / 2.0, h / 2.0),
        ],
    )
    .unwrap()
}

fn run(start: RuledContour, end: RuledContour) -> LoftResult {
    loft(&Loft { start, end }).unwrap()
}

fn volume(result: &LoftResult) -> f64 {
    result.volume_mm3().midpoint()
}

fn close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{actual} vs {expected}"
    );
}

#[test]
fn a_translated_rectangle_lofts_to_an_oblique_prism_with_exact_quantities() {
    let r = run(rect(200.0, 100.0, 0.0), rect(200.0, 100.0, 50.0));
    // The oblique prism closed form: base area times the height along the normal.
    close(volume(&r), 200.0 * 100.0 * 50.0, 1e-9);
    assert!(r.volume_mm3().contains(200.0 * 100.0 * 50.0));
    // Surface area: two caps (2 * 20000) plus the rectangular walls (600 * 50).
    let area = r.area_mm2().unwrap();
    close(area.midpoint(), 2.0 * 20000.0 + 600.0 * 50.0, 1e-9);
    // An authored polygon reports no approximation and four fold lines (the corners).
    assert!(r.sagitta_bound_mm().is_none());
    assert_eq!(r.fold_vertices(), [0, 1, 2, 3]);
    // Independent read-back: the blob parses as one closed shell of the exact volume.
    let blob = common::parse_blob(r.blob());
    assert_eq!(blob.vertices.len(), 8);
    assert_eq!(blob.triangles.len(), 4 * 2 + 2 * 2);
    let topology = common::topology(&blob.triangles, blob.vertices.len());
    assert!(topology.closed && topology.components == 1);
    close(
        common::mesh_volume(&blob.vertices, &blob.triangles),
        1_000_000.0,
        1e-9,
    );
}

#[test]
fn a_cylinder_patch_matches_the_annular_sector_closed_form_within_the_faceting() {
    // A quarter-cylinder sheet: outer radius 100, inner radius 90 (thickness 10),
    // height 50. Each contour is the sector polygon (centre, then the faceted arc), the
    // outer skin is translated along the axis, and the sector volume is the difference
    // of the two skin volumes. The same 72-segment facet choice on both radii comes
    // from scaling the sagitta with the radius (the G05 segment choice is scale-linear).
    let sector = |centre: Point3, radius: f64, sagitta: f64| {
        let arc = RuledContour::arc(centre, ex(), ey(), radius, 0.0, FRAC_PI_2, sagitta).unwrap();
        let mut points = vec![common::p(0.0, 0.0)];
        points.extend(arc.points().iter().copied());
        RuledContour::polygon(centre, ex(), ey(), points).unwrap()
    };
    let start = sector(p3(0.0, 0.0, 0.0), 100.0, 0.1);
    let end = sector(p3(0.0, 0.0, 50.0), 100.0, 0.1);
    let inner = sector(p3(0.0, 0.0, 0.0), 90.0, 0.09);
    let inner_end = sector(p3(0.0, 0.0, 50.0), 90.0, 0.09);
    let contour_points = start.points().len();
    assert_eq!(inner.points().len(), contour_points);
    assert_eq!(inner_end.points().len(), end.points().len());
    let r = run(start, end);
    let ri = run(inner, inner_end);
    let (ro, ri_r, theta, height) = (100.0_f64, 90.0_f64, FRAC_PI_2, 50.0_f64);
    let exact = height * (theta / 2.0) * (ro * ro - ri_r * ri_r);
    // Each skin is translated, so its volume is its faceted cap area times the height;
    // the sector volume they enclose is the outer minus the inner skin volume, below
    // the true sector value (every chord lies inside the arc), and the deficit is
    // bounded by the sagitta bands over the two arc lengths. The sector contours are
    // authored polygons, so their sagitta bounds are computed here from the facet angle
    // (the 90-degree sweep over 18 chords of the 72-segment full turn).
    let facet = FRAC_PI_2 / 18.0;
    let d_out = 100.0 * facet * facet / 8.0;
    let d_in = 90.0 * facet * facet / 8.0;
    let v = volume(&r) - volume(&ri);
    assert!(v <= exact, "{v} vs {exact}");
    let deficit = exact - v;
    assert!(
        deficit <= theta * ro * d_out * height + theta * ri_r * d_in * height,
        "{deficit}"
    );
    // Area of one skin: two faceted sector caps (below the exact sector areas), the
    // cylindrical wall (below its true arc-by-height area), and the two rectangular
    // radial end faces.
    let area = r.area_mm2().unwrap().midpoint();
    let exact_area = theta * ro * ro + theta * ro * height + 2.0 * ro * height;
    assert!(area <= exact_area, "{area} vs {exact_area}");
    assert!(area >= exact_area * 0.98, "{area} vs {exact_area}");
    // The sector is an authored polygon, so every vertex is a fold of this faceted
    // sheet (the centre vertex and every authored chord joint alike); a contour built
    // by `arc` reports none of its turns — see the cone test below.
    assert_eq!(r.fold_vertices().len(), contour_points);
    let blob = common::parse_blob(r.blob());
    let topology = common::topology(&blob.triangles, blob.vertices.len());
    assert!(topology.closed && topology.components == 1);
    close(
        common::mesh_volume(&blob.vertices, &blob.triangles),
        volume(&r),
        1e-12,
    );
}

#[test]
fn a_cone_patch_matches_the_cone_closed_form_within_the_faceting() {
    // A full conical patch: radius 100 tapering to radius 50 over the height 60. The
    // two circles are faceted at the same 64 segments by scaling the sagitta with the
    // radius, so the index correspondence holds.
    let outer = RuledContour::arc(p3(0.0, 0.0, 0.0), ex(), ey(), 100.0, 0.0, TAU, 0.13).unwrap();
    let inner = RuledContour::arc(p3(0.0, 0.0, 60.0), ex(), ey(), 50.0, 0.0, TAU, 0.065).unwrap();
    assert_eq!(outer.points().len(), 64);
    assert_eq!(inner.points().len(), 64);
    let r = run(outer, inner);
    let (r1, r2, height) = (100.0_f64, 50.0_f64, 60.0_f64);
    let exact = PI * height / 3.0 * (r1 * r1 + r1 * r2 + r2 * r2);
    // Every chord lies inside its circle, so the mesh volume sits below the true cone;
    // the deficit is bounded by the sagitta bands over the two perimeters.
    let v = volume(&r);
    let d = r.sagitta_bound_mm().unwrap();
    assert!(d > 0.0 && d <= 0.13, "{d}");
    assert!(v <= exact, "{v} vs {exact}");
    assert!(
        exact - v <= TAU * r1 * d * height + TAU * r2 * d * height,
        "{} vs {exact}",
        exact - v
    );
    // A circular contour turns nowhere: no fold lines.
    assert!(r.fold_vertices().is_empty());
    let blob = common::parse_blob(r.blob());
    let topology = common::topology(&blob.triangles, blob.vertices.len());
    assert!(topology.closed && topology.components == 1);
}

#[test]
fn a_tilted_loft_between_nonparallel_planes_stays_certified() {
    // The end contour is the start contour rotated about X by 30 degrees and lifted:
    // the two contour planes are not parallel, the body is not a prism, the analytic
    // self-check does not apply, and the seal still certifies the shell. The end frame
    // is the rotated start frame, so the index correspondence stays the continuous one.
    let start = rect(200.0, 100.0, 0.0);
    let c = 30.0_f64.to_radians().cos();
    let s = 30.0_f64.to_radians().sin();
    let end = RuledContour::polygon(
        p3(0.0, 0.0, 40.0),
        ex(),
        Vec3::new(0.0, c, s).unwrap(),
        vec![
            common::p(-100.0, -50.0),
            common::p(100.0, -50.0),
            common::p(100.0, 50.0),
            common::p(-100.0, 50.0),
        ],
    )
    .unwrap();
    let r = run(start, end);
    assert!(volume(&r) > 0.0);
    assert_eq!(r.fold_vertices(), [0, 1, 2, 3]);
    let blob = common::parse_blob(r.blob());
    let topology = common::topology(&blob.triangles, blob.vertices.len());
    assert!(topology.closed && topology.components == 1);
}

#[test]
fn a_skewed_loft_certifies_through_the_folded_wall_split() {
    // A skewed five-point loft (from the seeded random scan over random contours and
    // end frames): the correspondence stays continuous and the body seals, but the
    // wall quads fold non-convex, so this fixture dies if the wall split ignores the
    // interior-diagonal choice (verified by mutation: with the default split forced,
    // the certification refuses this exact body). No translation is exact, so the
    // analytic self-check does not apply.
    let start = RuledContour::polygon(
        p3(0.0, 0.0, 0.0),
        ex(),
        ey(),
        vec![
            common::p(66.69452022220796, 84.5902026130943),
            common::p(-37.51708006062485, 25.60071804341901),
            common::p(-88.47971438302095, 46.13612283203746),
            common::p(-68.83937549157699, -50.48686668848098),
            common::p(6.422995332023466, -38.8474963915148),
        ],
    )
    .unwrap();
    let end = RuledContour::polygon(
        p3(0.0, 0.0, 133.34023772656656),
        Vec3::new(0.389994497272547, 0.9208171871208385, 0.0).unwrap(),
        Vec3::new(-0.8766308044392784, 0.3712802005139592, 0.3060546444909106).unwrap(),
        vec![
            common::p(98.0924390735619, 60.473163936761864),
            common::p(13.844804786238598, 99.61760493394429),
            common::p(-61.265442437330236, 27.39545257802057),
            common::p(-6.3208615808396065, -80.6020821902689),
            common::p(86.97231015761841, -36.89828644894497),
        ],
    )
    .unwrap();
    // Independent bounds: the volume stays under its bounding box, and the surface
    // area covers at least the two cap areas (the shoelace areas of the contours).
    let cap_area = common::shoelace(start.points()) + common::shoelace(end.points());
    let r = run(start, end);
    assert!(volume(&r) > 0.0);
    let bounds = r.bounds();
    let box_volume = (bounds.max[0] - bounds.min[0])
        * (bounds.max[1] - bounds.min[1])
        * (bounds.max[2] - bounds.min[2]);
    assert!(volume(&r) < box_volume, "{} vs {box_volume}", volume(&r));
    assert!(r.area_mm2().unwrap().midpoint() >= cap_area);
    let blob = common::parse_blob(r.blob());
    let topology = common::topology(&blob.triangles, blob.vertices.len());
    assert!(topology.closed && topology.components == 1);
    close(
        common::mesh_volume(&blob.vertices, &blob.triangles),
        volume(&r),
        1e-12,
    );
}

#[test]
fn fold_lines_skip_straight_continuations() {
    // A five-point contour with one straight continuation (index 1): folds everywhere
    // except there; both contours fold at the same indices, so the union is the same.
    let flat = vec![
        common::p(0.0, 0.0),
        common::p(60.0, 0.0),
        common::p(120.0, 0.0),
        common::p(120.0, 40.0),
        common::p(0.0, 40.0),
    ];
    let start = RuledContour::polygon(p3(0.0, 0.0, 0.0), ex(), ey(), flat.clone()).unwrap();
    let end = RuledContour::polygon(p3(0.0, 0.0, 30.0), ex(), ey(), flat).unwrap();
    let r = run(start, end);
    assert_eq!(r.fold_vertices(), [0, 2, 3, 4]);
    // The straight continuation keeps the rectangle's area: the prism closed form.
    close(volume(&r), 120.0 * 40.0 * 30.0, 1e-9);
}

#[test]
fn repeat_builds_are_byte_identical() {
    let a = run(rect(200.0, 100.0, 0.0), rect(200.0, 100.0, 50.0));
    let b = run(rect(200.0, 100.0, 0.0), rect(200.0, 100.0, 50.0));
    assert_eq!(a.blob(), b.blob());
    assert_eq!(a.mesh_digest(), b.mesh_digest());
}

#[test]
fn bad_contours_are_refused_with_typed_codes() {
    // Different vertex counts: no ruling correspondence.
    let mismatch = loft(&Loft {
        start: rect(200.0, 100.0, 0.0),
        end: RuledContour::polygon(
            p3(0.0, 0.0, 50.0),
            ex(),
            ey(),
            vec![
                common::p(-100.0, -50.0),
                common::p(100.0, -50.0),
                common::p(100.0, 50.0),
                common::p(0.0, 0.0),
                common::p(-100.0, 50.0),
            ],
        )
        .unwrap(),
    })
    .unwrap_err();
    assert_eq!(mismatch, LoftFailure::Contour(LoftCode::ContourMismatch));
    // Fewer than three points.
    let tiny = RuledContour::polygon(
        p3(0.0, 0.0, 0.0),
        ex(),
        ey(),
        vec![common::p(0.0, 0.0), common::p(1.0, 0.0)],
    )
    .unwrap_err();
    assert_eq!(tiny, LoftFailure::Contour(LoftCode::TooFewPoints));
    // A zero axis cannot be normalized into a certified frame.
    let zero_axis = RuledContour::polygon(
        p3(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0).unwrap(),
        ey(),
        vec![common::p(0.0, 0.0); 3],
    )
    .unwrap_err();
    assert_eq!(zero_axis, LoftFailure::Contour(LoftCode::NumericUncertain));
    let far = RuledContour::polygon(
        p3(1.1e9, 0.0, 0.0),
        ex(),
        ey(),
        vec![common::p(0.0, 0.0); 3],
    )
    .unwrap_err();
    assert_eq!(far, LoftFailure::Contour(LoftCode::AboveLimit));
    // Degenerate arc parameters.
    let zero_radius =
        RuledContour::arc(p3(0.0, 0.0, 0.0), ex(), ey(), 0.0, 0.0, FRAC_PI_2, 0.1).unwrap_err();
    assert_eq!(zero_radius, LoftFailure::Contour(LoftCode::BadDeviation));
    let negative_sweep =
        RuledContour::arc(p3(0.0, 0.0, 0.0), ex(), ey(), 100.0, 0.0, -1.0, 0.1).unwrap_err();
    assert_eq!(negative_sweep, LoftFailure::Contour(LoftCode::BadDeviation));
    let bad_sagitta =
        RuledContour::arc(p3(0.0, 0.0, 0.0), ex(), ey(), 100.0, 0.0, FRAC_PI_2, 0.0).unwrap_err();
    assert_eq!(bad_sagitta, LoftFailure::Contour(LoftCode::BadDeviation));
    // A kilometre radius at a nanometre sagitta is unreachable inside the G05 cap.
    let unreachable =
        RuledContour::arc(p3(0.0, 0.0, 0.0), ex(), ey(), 1.0e6, 0.0, FRAC_PI_2, 1.0e-9)
            .unwrap_err();
    assert_eq!(
        unreachable,
        LoftFailure::Contour(LoftCode::DeviationUnreachable)
    );
}

#[test]
fn self_intersecting_correspondence_is_refused_by_the_seal() {
    // Both contours are simple and counter-clockwise, but the 180-degree renumbering of
    // the end contour twists every wall through the axis: the G06 certification refuses
    // the self-intersecting candidate.
    let start = rect(200.0, 100.0, 0.0);
    let end = RuledContour::polygon(
        p3(0.0, 0.0, 100.0),
        ex(),
        ey(),
        vec![
            common::p(100.0, 50.0),
            common::p(-100.0, 50.0),
            common::p(-100.0, -50.0),
            common::p(100.0, -50.0),
        ],
    )
    .unwrap();
    let failure = loft(&Loft { start, end }).unwrap_err();
    match failure {
        LoftFailure::Body(_) => {}
        other => panic!("expected a body refusal, got {other:?}"),
    }
}

#[test]
fn a_flat_translation_is_refused_as_degenerate() {
    // Moving the end contour sideways within its own plane puts both rings in one
    // plane: the body has no volume and never seals.
    let start = rect(200.0, 100.0, 0.0);
    let end = RuledContour::polygon(
        p3(500.0, 0.0, 0.0),
        ex(),
        ey(),
        vec![
            common::p(-100.0, -50.0),
            common::p(100.0, -50.0),
            common::p(100.0, 50.0),
            common::p(-100.0, 50.0),
        ],
    )
    .unwrap();
    let failure = loft(&Loft { start, end }).unwrap_err();
    match failure {
        LoftFailure::Body(_) => {}
        other => panic!("expected a body refusal, got {other:?}"),
    }
}
