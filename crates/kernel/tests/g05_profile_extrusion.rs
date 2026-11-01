// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G05 positive, mapping, metamorphic and byte-identity tests of the profile extrusion.
//!
//! Expected values come from closed-form formulas and independent scalar readers in
//! `common`, never from production helpers.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::common;

use beamcraft_kernel::profile::{
    Cavity, CurveKind, CurveSpec, FaceRole, Point2, Profile, VolumeStatus, extrude,
};
use beamcraft_kernel::{RectangularPlate, rectangular_plate};
use common::*;
use std::collections::BTreeSet;
use std::f64::consts::PI;

fn circle(cx: f64, cy: f64, r: f64) -> Cavity {
    Cavity::circle(p(cx, cy), r, CurveSpec::DEFAULT).unwrap()
}

#[test]
fn plain_rectangle_equals_the_g02_plate() {
    let outer = rect(0.0, 0.0, 23.0, 47.0);
    let ex = extrude(&outer, &[], 3.0).unwrap();
    assert_valid_extrusion(&ex, &outer, &[], 3.0);
    let plate = rectangular_plate(RectangularPlate {
        width_mm: 23.0,
        length_mm: 47.0,
        thickness_mm: 3.0,
    })
    .unwrap();
    assert_eq!(ex.volume_mm3(), plate.volume_mm3());
    assert_eq!(ex.volume_mm3(), 3243.0);
    assert_eq!(ex.volume_status(), VolumeStatus::Exact);
    assert_eq!(ex.volume_error_bound_mm3(), 0.0);
    let key = |v: &[f64; 3]| (v[0].to_bits(), v[1].to_bits(), v[2].to_bits());
    let ours: BTreeSet<_> = parse_blob(ex.mesh().blob())
        .vertices
        .iter()
        .map(key)
        .collect();
    let theirs: BTreeSet<_> = plate
        .mesh()
        .vertices()
        .iter()
        .map(|v| key(&[v.x, v.y, v.z]))
        .collect();
    assert_eq!(ours, theirs);
    assert_eq!(ex.mesh().triangles().len(), 12);
    assert_eq!(
        ex.mesh().face_table(),
        [
            "extrusion:f.z0",
            "extrusion:f.z1",
            "extrusion:f.o0",
            "extrusion:f.o1",
            "extrusion:f.o2",
            "extrusion:f.o3"
        ]
    );
}

#[test]
fn one_round_hole_has_the_documented_polygon_volume_and_bound() {
    let (r, t) = (5.0, 4.0);
    let outer = rect(0.0, 0.0, 40.0, 30.0);
    let hole = circle(20.0, 15.0, r);
    let approx = *hole.approximation().unwrap();
    assert_eq!(approx.kind, CurveKind::Circle);
    let n = approx.segments;
    assert_eq!(hole.points().len() as u32, n);
    assert!(n.is_multiple_of(4) && (16..=512).contains(&n));
    // The bound meets the request and one step fewer edges would not (independent formula).
    let bound = |n: u32| r * (2.0 * PI / f64::from(n)).powi(2) / 8.0;
    assert!(approx.sagitta_bound_mm <= approx.max_sagitta_mm);
    assert!((approx.sagitta_bound_mm - bound(n)).abs() < 1e-15);
    assert!(bound(n - 4) > approx.max_sagitta_mm);
    // Vertices lie on the circle; edge midpoints are within the true sagitta of it.
    let pts = hole.points();
    for (i, q) in pts.iter().enumerate() {
        let (dx, dy) = (q.x - 20.0, q.y - 15.0);
        assert!(((dx * dx + dy * dy).sqrt() - r).abs() < 1e-12);
        let nx = pts[(i + 1) % pts.len()];
        let (mx, my) = ((q.x + nx.x) / 2.0 - 20.0, (q.y + nx.y) / 2.0 - 15.0);
        let sagitta = r - (mx * mx + my * my).sqrt();
        assert!(sagitta >= -1e-12 && sagitta <= approx.sagitta_bound_mm + 1e-12);
    }
    assert!(shoelace(pts) < 0.0, "cavities are clockwise");
    let cavities = [hole];
    let ex = extrude(&outer, &cavities, t).unwrap();
    assert_valid_extrusion(&ex, &outer, &cavities, t);
    let polygon = regular_polygon_area(n, r);
    assert!(close(ex.volume_mm3(), (1200.0 - polygon) * t, 1e-12));
    assert!(polygon < PI * r * r);
    assert_eq!(ex.volume_status(), VolumeStatus::Approximated);
    // The reported bound covers the true curved solid.
    assert!(ex.volume_error_bound_mm3() >= (PI * r * r - polygon) * t);
    assert!(ex.volume_error_bound_mm3() <= 2.0 * PI * r * 0.01 * t * 1.01);
    // One face for the whole round wall, found through the cavity lookup.
    let mesh = ex.mesh();
    let walls = mesh.cavity_walls(0).unwrap();
    assert_eq!(walls.len(), 1);
    assert_eq!(mesh.face_table()[walls.start], "extrusion:f.c0");
    assert_eq!(
        mesh.face_roles()[walls.start],
        FaceRole::CavityWall {
            cavity: 0,
            edge: None
        }
    );
    assert!(mesh.face_normals()[walls.start].is_none());
    assert!(mesh.cavity_walls(1).is_none());
    // Every wall-face triangle lies on the cavity contour.
    let blob = parse_blob(mesh.blob());
    let wall_triangles = blob
        .faces
        .iter()
        .filter(|f| **f as usize == walls.start)
        .count();
    assert_eq!(wall_triangles, 2 * n as usize);
}

#[test]
fn many_round_holes_and_independent_volume() {
    let outer = rect(0.0, 0.0, 220.0, 170.0);
    let mut cavities = Vec::new();
    for row in 0..4 {
        for col in 0..5 {
            let r = 4.0 + f64::from(row * 5 + col) * 0.25;
            cavities.push(circle(
                30.0 + f64::from(col) * 40.0,
                30.0 + f64::from(row) * 35.0,
                r,
            ));
        }
    }
    let ex = extrude(&outer, &cavities, 6.0).unwrap();
    assert_valid_extrusion(&ex, &outer, &cavities, 6.0);
    let mut area = 220.0 * 170.0;
    for c in &cavities {
        let a = c.approximation().unwrap();
        // The closed-form n-gon area uses the radius read back from the first vertex.
        let first = c.points()[0];
        let centre_x = first.x - (first.x - c.points()[c.points().len() / 2].x) / 2.0;
        let r = (first.x - centre_x).abs();
        area -= regular_polygon_area(a.segments, r);
    }
    assert!(close(ex.volume_mm3(), area * 6.0, 1e-9));
    assert_eq!(ex.mesh().cavity_walls(19).unwrap().len(), 1);
    assert_eq!(ex.mesh().face_table().len(), 2 + 4 + 20);
}

#[test]
fn slot_and_ellipse_cavities_match_closed_forms() {
    let outer = rect(0.0, 0.0, 120.0, 80.0);
    let spec = CurveSpec::DEFAULT;
    for axis in [p(1.0, 0.0), p(0.0, 2.0), p(3.0, 4.0), p(-1.0, 1.0)] {
        let slot = Cavity::slot(p(40.0, 40.0), 30.0, 10.0, axis, spec).unwrap();
        let ellipse = Cavity::ellipse(p(85.0, 40.0), 14.0, 7.0, axis, spec).unwrap();
        let (ns, ne) = (
            slot.approximation().unwrap().segments,
            ellipse.approximation().unwrap().segments,
        );
        assert_eq!(slot.approximation().unwrap().kind, CurveKind::Slot);
        assert_eq!(slot.points().len(), ns as usize + 2);
        assert_eq!(ellipse.points().len(), ne as usize);
        // Obround: a 20 x 10 rectangle plus the two semicircle polygons, one full n-gon.
        let slot_area = 20.0 * 10.0 + regular_polygon_area(ns, 5.0);
        let ellipse_area = 0.5 * f64::from(ne) * 14.0 * 7.0 * (2.0 * PI / f64::from(ne)).sin();
        assert!(close(-shoelace(slot.points()), slot_area, 1e-12));
        assert!(close(-shoelace(ellipse.points()), ellipse_area, 1e-12));
        let cavities = [slot, ellipse];
        let ex = extrude(&outer, &cavities, 5.0).unwrap();
        assert_valid_extrusion(&ex, &outer, &cavities, 5.0);
        assert!(close(
            ex.volume_mm3(),
            (9600.0 - slot_area - ellipse_area) * 5.0,
            1e-12
        ));
        assert_eq!(ex.mesh().cavity_walls(0).unwrap().len(), 1);
    }
}

#[test]
fn ellipse_and_slot_stay_within_the_documented_sagitta() {
    let spec = CurveSpec {
        max_sagitta_mm: 0.02,
        min_segments: 8,
        max_segments: 1020,
    };
    let a = 30.0;
    let ellipse = Cavity::ellipse(p(0.0, 0.0), a, 12.0, p(1.0, 0.0), spec).unwrap();
    let info = ellipse.approximation().unwrap();
    let pts = ellipse.points();
    for (i, q) in pts.iter().enumerate() {
        // Vertices are on x^2/a^2 + y^2/b^2 = 1.
        assert!(((q.x / a).powi(2) + (q.y / 12.0).powi(2) - 1.0).abs() < 1e-12);
        // Distance from the curve to each chord: sample the arc between consecutive vertices.
        let nx = pts[(i + 1) % pts.len()];
        let (a0, a1) = (q.y.atan2(q.x * 12.0 / a), nx.y.atan2(nx.x * 12.0 / a));
        let (lo, hi) = (a0.min(a1), a0.max(a1));
        let (lo, hi) = if hi - lo > PI {
            (hi, lo + 2.0 * PI)
        } else {
            (lo, hi)
        };
        for k in 1..8 {
            let t = lo + (hi - lo) * f64::from(k) / 8.0;
            let c = (a * t.cos(), 12.0 * t.sin());
            let (ex, ey) = (nx.x - q.x, nx.y - q.y);
            let len = (ex * ex + ey * ey).sqrt();
            let dist = ((c.0 - q.x) * ey - (c.1 - q.y) * ex).abs() / len;
            assert!(dist <= info.sagitta_bound_mm + 1e-9, "sagitta {dist}");
        }
    }
}

#[test]
fn concave_l_shape_and_comb_with_holes() {
    let l_shape = vec![
        p(0.0, 0.0),
        p(60.0, 0.0),
        p(60.0, 20.0),
        p(20.0, 20.0),
        p(20.0, 50.0),
        p(0.0, 50.0),
    ];
    let cavities = [circle(40.0, 10.0, 4.0), hole_rect(5.0, 25.0, 15.0, 45.0)];
    let ex = extrude(&l_shape, &cavities, 8.0).unwrap();
    assert_valid_extrusion(&ex, &l_shape, &cavities, 8.0);
    // Comb: many reflex vertices, holes in the spine.
    let comb = vec![
        p(0.0, 0.0),
        p(100.0, 0.0),
        p(100.0, 40.0),
        p(90.0, 40.0),
        p(90.0, 15.0),
        p(70.0, 15.0),
        p(70.0, 40.0),
        p(60.0, 40.0),
        p(60.0, 15.0),
        p(40.0, 15.0),
        p(40.0, 40.0),
        p(30.0, 40.0),
        p(30.0, 15.0),
        p(10.0, 15.0),
        p(10.0, 40.0),
        p(0.0, 40.0),
    ];
    assert!(shoelace(&comb) > 0.0);
    let spine_hole = [circle(50.0, 7.5, 3.0), hole_rect(75.0, 3.0, 95.0, 12.0)];
    let ex = extrude(&comb, &spine_hole, 2.0).unwrap();
    assert_valid_extrusion(&ex, &comb, &spine_hole, 2.0);
}

#[test]
fn polygonal_cavity_walls_are_one_face_per_edge_and_findable() {
    let outer = rect(0.0, 0.0, 50.0, 50.0);
    let cavities = [
        hole_rect(5.0, 5.0, 15.0, 15.0),
        hole_rect(25.0, 25.0, 45.0, 35.0),
    ];
    let ex = extrude(&outer, &cavities, 3.0).unwrap();
    assert_valid_extrusion(&ex, &outer, &cavities, 3.0);
    let mesh = ex.mesh();
    assert_eq!(mesh.face_table().len(), 2 + 4 + 4 + 4);
    for (j, expected_first) in [(0usize, "extrusion:f.c0.s0"), (1, "extrusion:f.c1.s0")] {
        let walls = mesh.cavity_walls(j).unwrap();
        assert_eq!(walls.len(), 4);
        assert_eq!(mesh.face_table()[walls.start], expected_first);
        let blob = parse_blob(mesh.blob());
        for (k, face) in walls.clone().enumerate() {
            assert_eq!(
                mesh.face_roles()[face],
                FaceRole::CavityWall {
                    cavity: j as u32,
                    edge: Some(k as u32)
                }
            );
            // Wall triangles use exactly this cavity's edge k: bottom/top of points k, k+1.
            let n_total = 4 + 4 + 4;
            let offset = 4 + 4 * j;
            let (i0, i1) = (offset + k, offset + (k + 1) % 4);
            let allowed: BTreeSet<u32> = [i0, i1, n_total + i0, n_total + i1]
                .iter()
                .map(|v| *v as u32)
                .collect();
            let own: Vec<_> = blob
                .triangles
                .iter()
                .zip(&blob.faces)
                .filter(|(_, f)| **f as usize == face)
                .collect();
            assert_eq!(own.len(), 2);
            for (tri, _) in own {
                assert!(tri.iter().all(|v| allowed.contains(v)));
            }
            // The wall normal points into the hole: toward the cavity centre.
            let nm = mesh.face_normals()[face].unwrap();
            let (a, b) = (cavities[j].points()[k], cavities[j].points()[(k + 1) % 4]);
            let mid = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
            let pts = cavities[j].points();
            let centre = (
                pts.iter().map(|q| q.x).sum::<f64>() / 4.0,
                pts.iter().map(|q| q.y).sum::<f64>() / 4.0,
            );
            assert!(nm.x() * (centre.0 - mid.0) + nm.y() * (centre.1 - mid.1) > 0.0);
        }
    }
    assert_eq!(mesh.face_roles()[2], FaceRole::OuterWall { edge: 0 });
    assert_eq!(mesh.face_roles()[0], FaceRole::BottomCap);
    assert_eq!(mesh.face_roles()[1], FaceRole::TopCap);
}

#[test]
fn cavities_at_the_exact_minimum_distance_and_dimension_are_accepted() {
    // D = 2^20 exactly, so eps_geom = 2^-22 * D = 0.25 exactly (a dyadic boundary).
    // Every edge stays within the 1e6 mm edge cap; the far vertex (2^20, 0) sets D.
    let (half, big) = (524_288.0, 1_048_576.0);
    let outer = vec![
        p(0.0, 0.0),
        p(half, 0.0),
        p(big, 0.0),
        p(half, half),
        p(0.0, half),
    ];
    let eps = 0.25;
    let at_min = [hole_rect(eps, 1000.0, 2000.0, 3000.0)];
    let ex = extrude(&outer, &at_min, eps).unwrap();
    assert_valid_extrusion(&ex, &outer, &at_min, eps);
    // Bottom leg at the exact minimum distance as well, with an all-minimum 0.25 x 0.25 void.
    let tiny = hole_rect(5000.0, 1000.0, 5000.0 + eps, 1000.0 + eps);
    let cavities = [
        at_min[0].clone(),
        hole_rect(1000.0, eps, 1500.0, 900.0),
        tiny,
    ];
    let ex = extrude(&outer, &cavities, 1.0).unwrap();
    assert_valid_extrusion(&ex, &outer, &cavities, 1.0);
    // Just below the minimum: distance, edge length and thickness are all rejected.
    let below = f64::from_bits(eps.to_bits() - 1);
    let narrow_hi = f64::from_bits((5000.0 + eps).to_bits() - 1);
    for bad in [
        vec![hole_rect(below, 1000.0, 2000.0, 3000.0)],
        vec![hole_rect(5000.0, 1000.0, narrow_hi, 1000.0 + eps)],
    ] {
        assert!(extrude(&outer, &bad, 1.0).is_err());
    }
    assert!(extrude(&outer, &at_min, below).is_err());
    // The wall may be thicker than the minimum by one representable step.
    let above = f64::from_bits(eps.to_bits() + 1);
    let near = [hole_rect(above, 1000.0, 2000.0, 3000.0)];
    assert_valid_extrusion(&extrude(&outer, &near, 1.0).unwrap(), &outer, &near, 1.0);
}

fn g03_row(name: &str) -> Vec<String> {
    let path = format!(
        "{}/../../docs/fixtures/g03/parameters/{name}.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).unwrap();
    text.lines()
        .find(|l| !l.starts_with('#') && !l.is_empty())
        .unwrap()
        .split('\t')
        .map(str::to_owned)
        .collect()
}

fn g03_supports(id: &str) -> String {
    let path = format!(
        "{}/../../docs/fixtures/g03/manifest.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).unwrap();
    text.lines()
        .find(|l| l.starts_with(id))
        .map(|l| l.split('\t').nth(4).unwrap().to_owned())
        .unwrap()
}

#[test]
fn g03_reserved_hole_cavity_thin_wall_and_nonrectangular_fixtures_get_real_g05_solids() {
    // These four G03 ids are placeholders for 23 x 47 x 3 plates; G03 records them as
    // unsupported. G05 supplies the missing solids; the G03 expectations stay untouched.
    for (id, name) in [
        ("g03.plate.through-hole", "through-hole"),
        ("g03.plate.cavity", "cavity"),
        ("g03.plate.thin-wall", "thin-wall"),
        ("g03.plate.nonrectangular", "nonrectangular"),
    ] {
        let row = g03_row(name);
        assert_eq!(row[0], id);
        assert_eq!(row[1], "placeholder");
        assert_eq!(g03_supports(id), "unsupported");
        let dims: Vec<f64> = row[2..5].iter().map(|s| s.parse().unwrap()).collect();
        assert_eq!(dims, [23.0, 47.0, 3.0]);
    }
    let (w, l, t) = (23.0, 47.0, 3.0);
    let plate = rect(0.0, 0.0, w, l);
    // through-hole: a round hole of diameter 8 at the plate centre.
    let hole = [circle(w / 2.0, l / 2.0, 4.0)];
    let ex = extrude(&plate, &hole, t).unwrap();
    assert_valid_extrusion(&ex, &plate, &hole, t);
    // cavity: a rectangular void inside the plate outline.
    let void = [hole_rect(6.0, 12.0, 17.0, 35.0)];
    let ex = extrude(&plate, &void, t).unwrap();
    assert_valid_extrusion(&ex, &plate, &void, t);
    assert!(close(ex.volume_mm3(), (w * l - 11.0 * 23.0) * t, 1e-12));
    // nonrectangular: an L-shaped outline with the same 23 x 47 bounding box.
    let l_shape = vec![
        p(0.0, 0.0),
        p(w, 0.0),
        p(w, 20.0),
        p(10.0, 20.0),
        p(10.0, l),
        p(0.0, l),
    ];
    let ex = extrude(&l_shape, &[], t).unwrap();
    assert_valid_extrusion(&ex, &l_shape, &[], t);
    assert_eq!(ex.volume_mm3(), (23.0 * 20.0 + 10.0 * 27.0) * 3.0);
    // thin-wall: a void leaving a wall of the minimum thickness (0.001 mm) on every side.
    let e = 0.001;
    // Largest inner coordinate whose distance to the outer edge is still at least e.
    let inset = |hi: f64| {
        let mut y = hi - e;
        while hi - y < e {
            y = f64::from_bits(y.to_bits() - 1);
        }
        y
    };
    let (xi, yi) = (inset(w), inset(l));
    let wall = [hole_rect(e, e, xi, yi)];
    let ex = extrude(&plate, &wall, t).unwrap();
    assert_valid_extrusion(&ex, &plate, &wall, t);
    let tighter = [hole_rect(f64::from_bits(e.to_bits() - 1), e, xi, yi)];
    assert!(extrude(&plate, &tighter, t).is_err());
    let one_step_more = [hole_rect(e, e, f64::from_bits(xi.to_bits() + 1), yi)];
    assert!(extrude(&plate, &one_step_more, t).is_err());
}

#[test]
fn g03_large_translation_fixtures_run_on_xy_and_the_minimum_cube_is_rejected() {
    // The z origin is not part of the G05 profile frame; only the x/y translation is exercised.
    for name in ["large-positive", "large-negative", "large-mixed"] {
        let row = g03_row(name);
        assert_eq!(g03_supports(&row[0]), "unverified");
        let num = |i: usize| -> f64 { row[i].parse().unwrap() };
        let (w, l, t, ox, oy) = (num(2), num(3), num(4), num(5), num(6));
        // As given, a positive origin plus a positive extent leaves the 1e9 coordinate cap.
        let given = rect(ox, oy, ox + w, oy + l);
        let beyond = given.iter().any(|q| q.x.abs() > 1e9 || q.y.abs() > 1e9);
        let attempt = extrude(&given, &[], t);
        if beyond {
            let failure = attempt.unwrap_err();
            assert_eq!(failure.diagnostics()[0].code.as_str(), "GEO_ABOVE_LIMIT");
            assert_eq!(failure.status(), "limit_exceeded");
        } else {
            assert_valid_extrusion(&attempt.unwrap(), &given, &[], t);
        }
        // The same plate placed inward from the cap corner.
        let (x0, y0) = (
            if ox > 0.0 { ox - w } else { ox },
            if oy > 0.0 { oy - l } else { oy },
        );
        let outer = rect(x0, y0, x0 + w, y0 + l);
        let ex = extrude(&outer, &[], t).unwrap();
        assert_valid_extrusion(&ex, &outer, &[], t);
        assert_eq!(ex.volume_mm3(), (w * l) * t);
        // A round hole at the large translation: its coordinates are rounded to the grid.
        let hole = [Cavity::circle(p(x0 + 1.5, y0 + 2.0), 0.5, CurveSpec::DEFAULT).unwrap()];
        let ex = extrude(&outer, &hole, t).unwrap();
        assert_valid_extrusion(&ex, &outer, &hole, t);
    }
    // 0.001 mm edges at 1e9: the submitted binary64 difference decides, never the decimal.
    let row = g03_row("large-minimum-cube");
    let (ox, oy) = (
        row[5].parse::<f64>().unwrap(),
        row[6].parse::<f64>().unwrap(),
    );
    let outer = rect(ox - 0.001, oy - 0.001, ox, oy);
    let edge = outer[1].x - outer[0].x;
    let result = extrude(&outer, &[], 0.001);
    if edge < 0.001 {
        let failure = result.unwrap_err();
        assert_eq!(
            failure.diagnostics()[0].code.as_str(),
            "GEO_CONTOUR_INVALID"
        );
    } else {
        assert!(result.is_ok());
    }
    // One grid step (1.19e-7) either way: the smaller edge fails, the larger passes.
    let step = f64::from_bits(1.0e9f64.to_bits() + 1) - 1.0e9;
    let (lo, hi) = (0.001 - step, 0.001 + step);
    let make = |edge: f64| rect(-1.0e9, -1.0e9, -1.0e9 + edge, -1.0e9 + edge);
    assert!(make(lo).iter().all(|q| q.x.is_finite()));
    assert!(extrude(&make(lo), &[], 0.001).is_err());
    assert!(extrude(&make(hi), &[], 0.001).is_ok());
}

fn dyadic_profile() -> (Vec<Point2>, Vec<Cavity>) {
    let outer = vec![
        p(0.0, 0.0),
        p(64.0, 0.0),
        p(64.0, 24.0),
        p(32.0, 24.0),
        p(32.0, 48.0),
        p(0.0, 48.0),
    ];
    let cavities = vec![
        Cavity::polygon(vec![p(4.0, 4.0), p(4.0, 12.0), p(12.0, 12.0), p(12.0, 4.0)]),
        Cavity::polygon(vec![
            p(40.0, 4.0),
            p(44.0, 16.0),
            p(52.0, 12.0),
            p(48.0, 4.0),
        ]),
        Cavity::polygon(vec![
            p(4.0, 20.0),
            p(4.0, 44.0),
            p(20.0, 44.0),
            p(20.0, 36.0),
            p(12.0, 20.0),
        ]),
    ];
    (outer, cavities)
}

fn map_profile(
    outer: &[Point2],
    cavities: &[Cavity],
    f: impl Fn(Point2) -> Point2,
) -> (Vec<Point2>, Vec<Cavity>) {
    (
        outer.iter().map(|q| f(*q)).collect(),
        cavities
            .iter()
            .map(|c| Cavity::polygon(c.points().iter().map(|q| f(*q)).collect()))
            .collect(),
    )
}

#[test]
fn exact_translation_keeps_the_combinatorial_result() {
    let (outer, cavities) = dyadic_profile();
    let base = extrude(&outer, &cavities, 5.0).unwrap();
    assert_valid_extrusion(&base, &outer, &cavities, 5.0);
    for (dx, dy) in [(4096.0, -2048.0), (-512.5, 77.25), (1.0e6, 1.0e6)] {
        let (o2, c2) = map_profile(&outer, &cavities, |q| p(q.x + dx, q.y + dy));
        let moved = extrude(&o2, &c2, 5.0).unwrap();
        assert_valid_extrusion(&moved, &o2, &c2, 5.0);
        assert_eq!(moved.mesh().triangles(), base.mesh().triangles());
        assert_eq!(moved.mesh().triangle_faces(), base.mesh().triangle_faces());
        assert_eq!(moved.mesh().face_table(), base.mesh().face_table());
        assert_eq!(moved.mesh().edge_table(), base.mesh().edge_table());
        assert_eq!(moved.volume_mm3(), base.volume_mm3());
        assert_ne!(moved.mesh().mesh_digest(), base.mesh().mesh_digest());
    }
}

#[test]
fn exact_quarter_turns_keep_identity_and_quantities() {
    let (outer, cavities) = dyadic_profile();
    let base = extrude(&outer, &cavities, 5.0).unwrap();
    let mut rotated = (outer.clone(), cavities.clone());
    for _ in 0..3 {
        rotated = map_profile(&rotated.0, &rotated.1, |q| p(-q.y, q.x));
        let ex = extrude(&rotated.0, &rotated.1, 5.0).unwrap();
        assert_valid_extrusion(&ex, &rotated.0, &rotated.1, 5.0);
        assert_eq!(ex.mesh().face_table(), base.mesh().face_table());
        assert_eq!(ex.mesh().face_roles(), base.mesh().face_roles());
        assert_eq!(ex.mesh().triangles().len(), base.mesh().triangles().len());
        assert!(close(ex.volume_mm3(), base.volume_mm3(), 1e-14));
        assert!(close(ex.net_area_mm2(), base.net_area_mm2(), 1e-14));
    }
}

#[test]
fn cyclic_start_change_moves_roles_but_not_geometry_or_quantities() {
    let (outer, cavities) = dyadic_profile();
    let base = extrude(&outer, &cavities, 5.0).unwrap();
    let rotate = |v: &[Point2], by: usize| -> Vec<Point2> {
        v.iter().cycle().skip(by).take(v.len()).copied().collect()
    };
    let o2 = rotate(&outer, 2);
    let c2: Vec<Cavity> = cavities
        .iter()
        .enumerate()
        .map(|(i, c)| Cavity::polygon(rotate(c.points(), i + 1)))
        .collect();
    let ex = extrude(&o2, &c2, 5.0).unwrap();
    assert_valid_extrusion(&ex, &o2, &c2, 5.0);
    assert!(close(ex.volume_mm3(), base.volume_mm3(), 1e-14));
    let ids = |e: &beamcraft_kernel::profile::Extrusion| -> BTreeSet<String> {
        e.mesh().face_table().iter().cloned().collect()
    };
    assert_eq!(ids(&ex), ids(&base));
    // Wall normals form the same multiset: the same physical walls carry different roles.
    let normals = |e: &beamcraft_kernel::profile::Extrusion| -> Vec<(u64, u64)> {
        let mut v: Vec<_> = e
            .mesh()
            .face_normals()
            .iter()
            .flatten()
            .map(|n| (n.x().to_bits(), n.y().to_bits()))
            .collect();
        v.sort_unstable();
        v
    };
    assert_eq!(normals(&ex), normals(&base));
}

#[test]
fn repeat_builds_are_byte_identical_and_negative_zero_is_normalised() {
    let outer = vec![p(-0.0, -0.0), p(30.0, -0.0), p(30.0, 20.0), p(-0.0, 20.0)];
    let cavities = [
        circle(15.0, 10.0, 4.0),
        Cavity::polygon(vec![p(2.0, 2.0), p(2.0, 6.0), p(6.0, 6.0), p(6.0, 2.0)]),
    ];
    let first = extrude(&outer, &cavities, 2.5).unwrap();
    for _ in 0..3 {
        let again = extrude(&outer, &cavities, 2.5).unwrap();
        assert_eq!(again.mesh().blob(), first.mesh().blob());
        assert_eq!(again.mesh().mesh_digest(), first.mesh().mesh_digest());
        assert_eq!(again, first);
    }
    // parse_blob rejects any negative zero, so a clean parse proves normalisation.
    let blob = parse_blob(first.mesh().blob());
    assert_eq!(blob.vertices[0], [0.0, 0.0, 0.0]);
    let digest = beamcraft_model::digest::Sha256::digest(first.mesh().blob()).unwrap();
    assert_eq!(digest, first.mesh().mesh_digest());
    assert_eq!(first.mesh().mesh_digest().to_string().len(), 64);
}

#[test]
fn admitted_profile_triangulation_is_an_independent_exact_tiling() {
    let (outer, cavities) = dyadic_profile();
    let profile = Profile::admit(&outer, &cavities).unwrap();
    let tris = profile.triangulate().unwrap();
    let mut points = outer.clone();
    let mut boundary: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut start = 0usize;
    let mut spans = vec![(0usize, outer.len())];
    for c in &cavities {
        spans.push((points.len(), c.points().len()));
        points.extend_from_slice(c.points());
    }
    for (s, len) in spans {
        for k in 0..len {
            boundary.insert(((s + k) as u32, (s + (k + 1) % len) as u32));
        }
        start += len;
    }
    let mut area = 0.0;
    let mut directed: BTreeSet<(u32, u32)> = BTreeSet::new();
    for [a, b, c] in &tris {
        let [pa, pb, pc] = [a, b, c].map(|i| points[*i as usize]);
        let cross = (pb.x - pa.x) * (pc.y - pa.y) - (pb.y - pa.y) * (pc.x - pa.x);
        assert!(cross > 0.0, "every cap triangle is counter-clockwise");
        area += cross / 2.0;
        for e in [(*a, *b), (*b, *c), (*c, *a)] {
            assert!(directed.insert(e), "directed edge used twice");
        }
    }
    // Interior edges cancel; what remains is exactly the contour boundary, once.
    let leftover: BTreeSet<(u32, u32)> = directed
        .iter()
        .filter(|(a, b)| !directed.contains(&(*b, *a)))
        .copied()
        .collect();
    assert_eq!(leftover, boundary);
    assert!(close(area, net_area(&outer, &cavities), 1e-12));
    assert_eq!(tris.len(), points.len() + 2 * cavities.len() - 2);
    assert_eq!(start, points.len());
}

#[test]
fn one_above_the_face_cap_is_a_topology_limit() {
    // Clockwise rings of `n` points on a circle of `radius` around (cx, cy).
    let ring = |n: usize, radius: f64, cx: f64, cy: f64| -> Vec<Point2> {
        (0..n)
            .map(|k| {
                let a = 2.0 * PI * k as f64 / n as f64;
                p(cx + radius * a.cos(), cy - radius * a.sin())
            })
            .collect()
    };
    // 63 polygonal cavities of 1024 points and one of 1018 on an 8x8 grid: 2 caps + 4 outer
    // walls + 65,530 cavity walls is exactly one face above the F06 cap of 65,535, with every
    // other total inside its cap. Admission accepts the contours; the face cap refuses before
    // any triangulation. Acceptance at the cap is covered blob-level by the F06-POS-27 cases
    // in the shell and engine validators: a full 65,535-face build here would take minutes.
    let cavities: Vec<Cavity> = (0..64)
        .map(|k| {
            let (col, row) = (f64::from(k % 8), f64::from(k / 8));
            let n = if k == 63 { 1018 } else { 1024 };
            Cavity::polygon(ring(n, 10.0, 15.0 + 25.0 * col, 15.0 + 25.0 * row))
        })
        .collect();
    let outer = rect(0.0, 0.0, 220.0, 220.0);
    assert!(Profile::admit(&outer, &cavities).is_ok());
    let failure = extrude(&outer, &cavities, 10.0).unwrap_err();
    assert_eq!(failure.diagnostics()[0].code.as_str(), "GEO_TOPOLOGY_LIMIT");
    assert_eq!(failure.status(), "limit_exceeded");
}

#[test]
fn triangle_cap_is_enforced_before_any_triangulation() {
    let spec = CurveSpec {
        max_sagitta_mm: 1.0,
        min_segments: 8,
        max_segments: 8,
    };
    let holes = |count: usize| -> (Vec<Point2>, Vec<Cavity>) {
        let outer = rect(0.0, 0.0, 20.0 * 20.0 + 20.0, 20.0 * 12.0 + 20.0);
        let cavities = (0..count)
            .map(|k| {
                let (col, row) = ((k % 20) as f64, (k / 20) as f64);
                Cavity::circle(p(20.0 + col * 20.0, 20.0 + row * 20.0), 5.0, spec).unwrap()
            })
            .collect();
        (outer, cavities)
    };
    // T = 4N + 4h - 4 = 36h + 12 with N = 4 + 8h: 227 holes give 8184, far below the F06 cap.
    let (outer, cavities) = holes(227);
    let ex = extrude(&outer, &cavities, 3.0).unwrap();
    assert_eq!(ex.mesh().triangles().len(), 8184);
    assert_valid_extrusion(&ex, &outer, &cavities, 3.0);
    // At the cap: 128 curve holes of 1020 edges and one of 508 put N exactly at the total-point
    // cap (131,072) and 4N + 4h - 4 at 524,800, one bucket above the F06 triangle cap of
    // 524,288. Admission accepts; the triangle cap refuses before any triangulation.
    let cap_spec = CurveSpec {
        max_sagitta_mm: 1.0e9,
        min_segments: 1020,
        max_segments: 1020,
    };
    let cavities: Vec<Cavity> = (0..129)
        .map(|k| {
            let (col, row) = (f64::from(k % 16), f64::from(k / 16));
            let segments = if k == 128 { 508 } else { 1020 };
            Cavity::circle(
                p(15.0 + 25.0 * col, 15.0 + 25.0 * row),
                10.0,
                CurveSpec {
                    min_segments: segments,
                    max_segments: segments,
                    ..cap_spec
                },
            )
            .unwrap()
        })
        .collect();
    let outer = rect(0.0, 0.0, 420.0, 240.0);
    assert!(Profile::admit(&outer, &cavities).is_ok());
    let failure = extrude(&outer, &cavities, 3.0).unwrap_err();
    assert_eq!(failure.diagnostics()[0].code.as_str(), "GEO_BUFFER_LIMIT");
}

#[test]
fn flat_vertices_and_aligned_holes_triangulate_without_degenerate_triangles() {
    // 10 points per side: every side carries eight flat (collinear) vertices.
    let mut outer = Vec::new();
    for k in 0..10 {
        outer.push(p(f64::from(k) * 10.0, 0.0));
    }
    for k in 0..10 {
        outer.push(p(100.0, f64::from(k) * 10.0));
    }
    for k in 0..10 {
        outer.push(p(100.0 - f64::from(k) * 10.0, 100.0));
    }
    for k in 0..10 {
        outer.push(p(0.0, 100.0 - f64::from(k) * 10.0));
    }
    // Nine square holes on a grid whose lines coincide with the outer subdivision points.
    let mut cavities = Vec::new();
    for gy in 0..3 {
        for gx in 0..3 {
            let (x, y) = (20.0 + f64::from(gx) * 30.0, 20.0 + f64::from(gy) * 30.0);
            cavities.push(hole_rect(x, y, x + 10.0, y + 10.0));
        }
    }
    let ex = extrude(&outer, &cavities, 4.0).unwrap();
    assert_valid_extrusion(&ex, &outer, &cavities, 4.0);
    assert_eq!(ex.volume_mm3(), (10000.0 - 900.0) * 4.0);
    // The same profile with a flat vertex on a cavity edge, and rotated by exact quarter turns.
    let mut with_flat = cavities.clone();
    with_flat[4] = Cavity::polygon(vec![
        p(50.0, 50.0),
        p(50.0, 55.0),
        p(50.0, 60.0),
        p(60.0, 60.0),
        p(60.0, 50.0),
    ]);
    let ex = extrude(&outer, &with_flat, 4.0).unwrap();
    assert_valid_extrusion(&ex, &outer, &with_flat, 4.0);
    let (o, c) = map_profile(&outer, &with_flat, |q| p(-q.y, q.x));
    assert_valid_extrusion(&extrude(&o, &c, 4.0).unwrap(), &o, &c, 4.0);
}

#[test]
fn public_vertices_and_blob_both_carry_positive_zero() {
    let outer = vec![p(-0.0, -0.0), p(30.0, -0.0), p(30.0, 20.0), p(-0.0, 20.0)];
    let hole = [Cavity::polygon(vec![
        p(-0.0 + 5.0, 5.0),
        p(5.0, 10.0),
        p(10.0, 10.0),
        p(10.0, 5.0),
    ])];
    let ex = extrude(&outer, &hole, 2.0).unwrap();
    let blob = parse_blob(ex.mesh().blob());
    for (public, encoded) in ex.mesh().vertices().iter().zip(&blob.vertices) {
        for (a, b) in [public.x, public.y, public.z].into_iter().zip(*encoded) {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "public vertex and blob differ bitwise"
            );
            assert!(a != 0.0 || a.is_sign_positive());
        }
    }
    assert_eq!(ex.mesh().vertices()[0].x.to_bits(), 0.0f64.to_bits());
}
