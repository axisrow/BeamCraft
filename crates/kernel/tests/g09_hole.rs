// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G09 through-hole tests: closed-form areas and volumes, genus and Euler change, voxel
//! counting, equality with the G05 extrusion of the same profile, metamorphic moves,
//! canonical bytes, the G03 fixtures and every refusal of the pilot scope.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use crate::common;
use crate::common_mesh;
use crate::common_ops;

use beamcraft_kernel::diagnostic::Stage;
use beamcraft_kernel::ops::{
    Body, CutPlane, HoleCutter, Keep, OpCode, OpDefect, OpField, OpKind, OpResult, plane_cut,
    through_hole,
};
use beamcraft_kernel::profile::{Cavity, CurveSpec, Point2, extrude};
use common::{close, hole_rect, p, parse_blob, rect, regular_polygon_area};
use common_mesh::Rng;
use common_ops::*;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

fn circle(cx: f64, cy: f64, r: f64) -> Cavity {
    Cavity::circle(p(cx, cy), r, CurveSpec::DEFAULT).unwrap()
}

fn drill(body: &Body, cavity: Cavity, thickness: f64, ordinal: u16) -> OpResult {
    through_hole(
        body,
        &HoleCutter::new(cavity, -1.0, thickness + 1.0, ordinal),
    )
    .unwrap()
}

fn refuse(body: &Body, cavity: Cavity, thickness: f64) -> beamcraft_kernel::ops::OpFailure {
    through_hole(body, &HoleCutter::new(cavity, -1.0, thickness + 1.0, 9)).unwrap_err()
}

fn face_areas(res: &OpResult) -> BTreeMap<String, f64> {
    let blob = parse_blob(res.blob());
    let mut map: BTreeMap<String, f64> = BTreeMap::new();
    for (tri, f) in blob.triangles.iter().zip(&blob.faces) {
        let [a, b, c] = tri.map(|i| blob.vertices[i as usize]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let x = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        let area = 0.5 * (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
        *map.entry(res.face_table()[usize::from(*f)].clone())
            .or_default() += area;
    }
    map
}

fn vertex_set(res: &OpResult) -> BTreeSet<[u64; 3]> {
    res.body()
        .mesh()
        .vertices()
        .iter()
        .map(|v| v.map(f64::to_bits))
        .collect()
}

#[test]
fn round_hole_in_a_g02_plate_matches_closed_forms_and_the_g05_extrusion() {
    let (w, l, t) = (40.0, 30.0, 8.0);
    let body = plate(w, l, t);
    let cavity = circle(20.0, 15.0, 5.0);
    let n = cavity.approximation().unwrap().segments;
    let res = drill(&body, cavity.clone(), t, 0);
    let facts = assert_sealed(&res);
    assert_eq!(res.kind(), OpKind::ThroughHole);
    // The inscribed n-gon has the closed-form area (n/2) r^2 sin(2 pi / n) and lies below pi r^2.
    let area = regular_polygon_area(n, 5.0);
    assert!(area < PI * 25.0 && PI * 25.0 - area < 0.02 * PI * 25.0);
    assert!(close(facts.volume, (w * l - area) * t, 1e-12));
    // The analytic volume is exact, enclosed outward, and agrees with the certified mesh.
    let analytic = res.analytic_volume_mm3().unwrap();
    assert!(analytic.overlaps(res.volume_mm3()));
    assert!(analytic.contains(facts.volume) || (facts.volume - analytic.lo).abs() < 1e-9);
    // Genus grows by exactly one: Euler 2 -> 0.
    assert_eq!(body.analysis().euler_characteristic(), 2);
    assert_eq!((res.euler_characteristic(), res.genus()), (0, Some(1)));
    assert_eq!(facts.euler, 0);
    // 12 + 4 n + 4 triangles: two caps of n + 4, eight outer wall triangles, 2 n hole walls.
    assert_eq!(facts.triangles, 4 * n as usize + 16);
    assert_eq!(res.stats().triangles_in, 12);
    assert_eq!(res.stats().triangles_out, facts.triangles);
    assert_eq!(res.stats().new_vertices, 2 * n as usize);
    assert_eq!(res.stats().loops, 2);
    // Face table: the six plate ids keep their order, then the one curved wall, no normal.
    assert_eq!(&res.face_table()[..6], body.face_table());
    assert_eq!(res.face_table()[6], "cut:f.h0");
    assert!(res.face_normals()[6].is_none());
    assert!(res.removed_faces().is_empty());
    assert_eq!(res.face_normals()[..6], *body.face_normals());
    // The very same solid as the G05 extrusion of that profile: identical vertices and areas.
    let outer = rect(0.0, 0.0, w, l);
    let ex = extrude(&outer, &[cavity], t).unwrap();
    let g05 = Body::from_extrusion(&ex).unwrap();
    let verts: BTreeSet<[u64; 3]> = g05
        .mesh()
        .vertices()
        .iter()
        .map(|v| v.map(f64::to_bits))
        .collect();
    assert_eq!(vertex_set(&res), verts);
    assert!(close(facts.volume, ex.volume_mm3(), 1e-12));
    let total = |b: &Body| -> f64 {
        let v = b.mesh().vertices();
        b.mesh()
            .triangles()
            .iter()
            .map(|t| {
                let [a, b2, c] = t.map(|i| v[i as usize]);
                let u = [b2[0] - a[0], b2[1] - a[1], b2[2] - a[2]];
                let w2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let x = [
                    u[1] * w2[2] - u[2] * w2[1],
                    u[2] * w2[0] - u[0] * w2[2],
                    u[0] * w2[1] - u[1] * w2[0],
                ];
                0.5 * (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt()
            })
            .sum()
    };
    assert!(close(total(res.body()), total(&g05), 1e-12));
    // The cap area equals the net profile area.
    let areas = face_areas(&res);
    assert!(close(areas["plate:f.z0"], w * l - area, 1e-12));
    assert!(close(areas["plate:f.z1"], w * l - area, 1e-12));
    assert!(close(
        areas["cut:f.h0"],
        2.0 * 5.0 * (PI / f64::from(n)).sin() * f64::from(n) * t,
        1e-12
    ));
}

#[test]
fn oval_slot_and_polygon_holes_match_their_closed_forms() {
    let (w, l, t) = (120.0, 80.0, 5.0);
    let body = plate(w, l, t);
    let spec = CurveSpec::DEFAULT;
    for axis in [p(1.0, 0.0), p(0.0, 2.0), p(3.0, 4.0), p(-1.0, 1.0)] {
        let slot = Cavity::slot(p(40.0, 40.0), 30.0, 10.0, axis, spec).unwrap();
        let ns = slot.approximation().unwrap().segments;
        let res = drill(&body, slot, t, 0);
        let slot_area = 20.0 * 10.0 + regular_polygon_area(ns, 5.0);
        assert!(close(
            assert_sealed(&res).volume,
            (w * l - slot_area) * t,
            1e-12
        ));
        assert_eq!(res.genus(), Some(1));
        assert!(res.face_normals().last().unwrap().is_none());

        let ellipse = Cavity::ellipse(p(85.0, 40.0), 14.0, 7.0, axis, spec).unwrap();
        let ne = ellipse.approximation().unwrap().segments;
        let res2 = drill(res.body(), ellipse, t, 1);
        let ellipse_area = 0.5 * f64::from(ne) * 14.0 * 7.0 * (2.0 * PI / f64::from(ne)).sin();
        assert!(close(
            assert_sealed(&res2).volume,
            (w * l - slot_area - ellipse_area) * t,
            1e-12
        ));
        assert_eq!((res2.genus(), res2.euler_characteristic()), (Some(2), -2));
        assert_eq!(res2.stats().loops, 3);
        // Second hole: its wall is new, the first hole's wall id is kept.
        assert!(res2.face_table().contains(&"cut:f.h0".to_owned()));
        assert_eq!(res2.face_table().last().unwrap(), "cut:f.h1");
    }
    // A polygonal hole has one planar wall per edge, with exact inward normals.
    let res = drill(&body, hole_rect(10.0, 10.0, 16.0, 14.0), t, 3);
    let facts = assert_sealed(&res);
    assert_eq!(facts.volume, (w * l - 24.0) * t);
    assert_eq!(res.analytic_volume_mm3().unwrap().lo, facts.volume);
    assert_eq!(res.analytic_volume_mm3().unwrap().hi, facts.volume);
    let ids: Vec<&String> = res.face_table().iter().skip(6).collect();
    assert_eq!(
        ids,
        ["cut:f.h3.s0", "cut:f.h3.s1", "cut:f.h3.s2", "cut:f.h3.s3"]
    );
    // Edge 0 runs (10,10) -> (10,14) on the clockwise cavity; its wall faces +X into the hole.
    assert_eq!(res.face_normals()[6], Some([1.0, 0.0, 0.0]));
    assert_eq!(res.face_normals()[7], Some([0.0, -1.0, 0.0]));
    assert_eq!(res.face_normals()[8], Some([-1.0, 0.0, 0.0]));
    assert_eq!(res.face_normals()[9], Some([0.0, 1.0, 0.0]));
}

#[test]
fn several_holes_in_any_order_give_the_same_solid() {
    let (w, l, t) = (60.0, 40.0, 6.0);
    let holes = [
        (circle(12.0, 12.0, 4.0), 0u16),
        (hole_rect(25.0, 8.0, 33.0, 20.0), 1),
        (circle(48.0, 28.0, 6.0), 2),
    ];
    let orders: [[usize; 3]; 3] = [[0, 1, 2], [2, 0, 1], [1, 2, 0]];
    type Signature = (BTreeMap<String, f64>, BTreeSet<[u64; 3]>, f64);
    let mut reference: Option<Signature> = None;
    for order in orders {
        let mut body = plate(w, l, t);
        let mut last = None;
        for i in order {
            let (cavity, ordinal) = holes[i].clone();
            let res = drill(&body, cavity, t, ordinal);
            assert_sealed(&res);
            body = res.body().clone();
            last = Some(res);
        }
        let res = last.unwrap();
        assert_eq!((res.genus(), res.euler_characteristic()), (Some(3), -4));
        let key = (
            face_areas(&res),
            vertex_set(&res),
            assert_sealed(&res).volume,
        );
        match &reference {
            None => reference = Some(key),
            Some((areas, verts, volume)) => {
                assert_eq!(&key.1, verts);
                assert!(close(key.2, *volume, 1e-12));
                assert_eq!(
                    key.0.keys().collect::<Vec<_>>(),
                    areas.keys().collect::<Vec<_>>()
                );
                for (id, a) in &key.0 {
                    assert!(close(*a, areas[id], 1e-12), "{id}");
                }
            }
        }
    }
    // The same solid as the G05 extrusion of the three-cavity profile.
    let cavities: Vec<Cavity> = holes.iter().map(|(c, _)| c.clone()).collect();
    let ex = extrude(&rect(0.0, 0.0, w, l), &cavities, t).unwrap();
    let g05 = Body::from_extrusion(&ex).unwrap();
    let expected: BTreeSet<[u64; 3]> = g05
        .mesh()
        .vertices()
        .iter()
        .map(|v| v.map(f64::to_bits))
        .collect();
    assert_eq!(reference.as_ref().unwrap().1, expected);
    assert!(close(reference.unwrap().2, ex.volume_mm3(), 1e-12));
}

#[test]
fn nonrectangular_and_offset_prisms_accept_holes() {
    // An L-shaped extrusion that already has a hole, placed far from the origin in x, y and z
    // by an exact dyadic translation; the z range of the cutter follows the body.
    let outline = [
        p(0.0, 0.0),
        p(23.0, 0.0),
        p(23.0, 20.0),
        p(10.0, 20.0),
        p(10.0, 47.0),
        p(0.0, 47.0),
    ];
    let first = [circle(15.0, 8.0, 3.0)];
    let ex = extrude(&outline, &first, 3.0).unwrap();
    let body = Body::from_extrusion(&ex).unwrap();
    let res = drill(&body, hole_rect(3.0, 25.0, 7.0, 40.0), 3.0, 0);
    let facts = assert_sealed(&res);
    let n = first[0].approximation().unwrap().segments;
    let area = (23.0 * 20.0 + 10.0 * 27.0) - regular_polygon_area(n, 3.0) - 4.0 * 15.0;
    assert!(close(facts.volume, area * 3.0, 1e-12));
    assert_eq!((res.genus(), res.euler_characteristic()), (Some(2), -2));
    // The existing hole's wall id and the L's outer walls survive.
    assert!(res.face_table().contains(&"extrusion:f.c0".to_owned()));
    assert!(res.face_table().contains(&"extrusion:f.o3".to_owned()));
    // Translate the whole body and the cutter by dyadic offsets.
    let (dx, dy, dz) = (4096.0, -2048.5, 1000.25);
    let moved = mapped(&body, |q| [q[0] + dx, q[1] + dy, q[2] + dz], |n| n, false);
    let cavity = hole_rect(3.0 + dx, 25.0 + dy, 7.0 + dx, 40.0 + dy);
    let moved_res = through_hole(&moved, &HoleCutter::new(cavity, dz - 1.0, dz + 4.0, 0)).unwrap();
    assert!(close(assert_sealed(&moved_res).volume, facts.volume, 1e-12));
    assert_eq!(moved_res.euler_characteristic(), res.euler_characteristic());
}

#[test]
fn quarter_turns_and_mirrors_commute_with_the_hole() {
    let (w, l, t) = (40.0, 30.0, 8.0);
    let body = plate(w, l, t);
    let cavity = hole_rect(5.0, 6.0, 12.0, 21.0);
    let base = assert_sealed(&drill(&body, cavity.clone(), t, 0)).volume;
    assert_eq!(base, (w * l - 7.0 * 15.0) * t);
    // (x, y) -> (-y, x): the clockwise cavity stays clockwise.
    let turned = mapped(
        &body,
        |q| [-q[1], q[0], q[2]],
        |n| [-n[1], n[0], n[2]],
        false,
    );
    let pts: Vec<Point2> = cavity.points().iter().map(|q| p(-q.y, q.x)).collect();
    let r = drill(&turned, Cavity::polygon(pts), t, 0);
    assert_eq!(assert_sealed(&r).volume, base);
    // x -> -x reverses the winding of the cavity, so it is listed backwards to stay clockwise.
    let mirrored = mapped(
        &body,
        |q| [-q[0], q[1], q[2]],
        |n| [-n[0], n[1], n[2]],
        true,
    );
    let pts: Vec<Point2> = cavity.points().iter().rev().map(|q| p(-q.x, q.y)).collect();
    let r = drill(&mirrored, Cavity::polygon(pts), t, 0);
    assert_eq!(assert_sealed(&r).volume, base);
    // Rotating a round hole about the plate centre leaves the volume equal to rounding.
    let a = assert_sealed(&drill(&body, circle(10.0, 10.0, 4.0), t, 0)).volume;
    let b = assert_sealed(&drill(&body, circle(30.0, 20.0, 4.0), t, 0)).volume;
    assert!(close(a, b, 1e-12));
}

#[test]
fn renumbering_the_body_never_changes_the_bytes_and_repeats_are_identical() {
    let t = 6.0;
    let first = Body::from_extrusion(
        &extrude(&rect(0.0, 0.0, 50.0, 40.0), &[circle(12.0, 12.0, 4.0)], t).unwrap(),
    )
    .unwrap();
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for body in [plate(50.0, 40.0, t), first] {
        let reference = drill(&body, hole_rect(30.0, 20.0, 40.0, 30.0), t, 5);
        let again = drill(&body, hole_rect(30.0, 20.0, 40.0, 30.0), t, 5);
        assert_eq!(reference.blob(), again.blob());
        assert_eq!(reference.mesh_digest(), again.mesh_digest());
        for _ in 0..16 {
            let shuffled = renumbered(&body, &mut rng);
            let r = drill(&shuffled, hole_rect(30.0, 20.0, 40.0, 30.0), t, 5);
            assert_eq!(r.blob(), reference.blob());
            assert_eq!(r.mesh_digest(), reference.mesh_digest());
            assert_eq!(r.face_table(), reference.face_table());
            assert_eq!(r.face_normals(), reference.face_normals());
        }
    }
}

#[test]
fn voxel_counting_agrees_with_the_hole_volume() {
    let (w, l, t) = (24.0, 16.0, 4.0);
    let cavity = circle(10.0, 8.0, 5.0);
    let poly: Vec<[f64; 2]> = cavity.points().iter().map(|q| [q.x, q.y]).collect();
    let res = drill(&plate(w, l, t), cavity, t, 0);
    let counted = voxel_volume([0.0; 3], [w, l, t], 96, |q| {
        !in_polygon(&poly, [q[0], q[1]])
    });
    let f = assert_sealed(&res);
    assert!(
        (f.volume - counted).abs() / f.volume < 0.005,
        "{} {counted}",
        f.volume
    );
    // Both oracles agree that the hole removed volume pi r^2 t within the sagitta bound.
    let ideal = (w * l - PI * 25.0) * t;
    assert!((f.volume - ideal).abs() / ideal < 0.01);
}

#[test]
fn a_hole_after_a_plane_cut_composes_and_a_tilted_cut_is_refused() {
    let t = 8.0;
    let body = plate(40.0, 30.0, t);
    // A vertical cut keeps a prism: the hole still works.
    let cut = plane_cut(
        &body,
        &CutPlane::new([30.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        Keep::Negative,
        0,
    )
    .unwrap();
    let res = through_hole(
        cut.body(),
        &HoleCutter::new(circle(15.0, 15.0, 5.0), -1.0, 9.0, 0),
    );
    let res = res.unwrap();
    assert_eq!(res.genus(), Some(1));
    assert!(res.face_table().contains(&"cut:f.p0".to_owned()));
    assert_sealed(&res);
    // A tilted cut leaves vertices at many heights: not a prism.
    let tilted = plane_cut(
        &body,
        &CutPlane::new([0.0, 0.0, 4.0], [0.5, 0.0, 1.0]),
        Keep::Negative,
        0,
    )
    .unwrap();
    let e = through_hole(
        tilted.body(),
        &HoleCutter::new(circle(20.0, 15.0, 2.0), -1.0, 9.0, 1),
    )
    .unwrap_err();
    assert_eq!(
        (e.code, e.field, e.defect),
        (OpCode::OpUnsupported, OpField::Body, OpDefect::BodyNotPrism)
    );
    // A horizontal cut keeps a thinner prism: a hole through its new caps is fine.
    let low = plane_cut(
        &body,
        &CutPlane::new([0.0, 0.0, 3.0], [0.0, 0.0, 1.0]),
        Keep::Negative,
        0,
    )
    .unwrap();
    let res = through_hole(
        low.body(),
        &HoleCutter::new(circle(20.0, 15.0, 5.0), -1.0, 4.0, 1),
    )
    .unwrap();
    assert_sealed(&res);
    // The cutter must now clear the new top at z = 3: reaching only z = 3 is tangent.
    let e = through_hole(
        low.body(),
        &HoleCutter::new(circle(20.0, 15.0, 5.0), -1.0, 3.0, 2),
    )
    .unwrap_err();
    assert_eq!(e.defect, OpDefect::CutterTangent);
}

fn g03_row(name: &str) -> Vec<String> {
    let path = format!(
        "{}/../../docs/fixtures/g03/parameters/{name}.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .find(|l| !l.starts_with('#') && !l.is_empty())
        .unwrap()
        .split('\t')
        .map(str::to_owned)
        .collect()
}

#[test]
fn g03_through_hole_cavity_and_thin_wall_fixtures_run_on_the_real_operation() {
    // The four reserved G03 ids describe 23 x 47 x 3 plates; their expectation rows stay
    // `not_run` in G03 (this task changes no fixture). The first three are exercised here
    // by cutting the G02 plate; `nonrectangular` is an outline, not a cut, and is not run.
    for name in ["through-hole", "cavity", "thin-wall"] {
        let row = g03_row(name);
        assert_eq!(row[1], "placeholder");
        let dims: Vec<f64> = row[2..5].iter().map(|s| s.parse().unwrap()).collect();
        assert_eq!(dims, [23.0, 47.0, 3.0]);
    }
    let (w, l, t) = (23.0, 47.0, 3.0);
    let body = plate(w, l, t);
    // through-hole: a round hole of diameter 8 at the centre.
    let res = drill(&body, circle(w / 2.0, l / 2.0, 4.0), t, 0);
    let n = res.face_table().len();
    assert_eq!(n, 7);
    let f = assert_sealed(&res);
    assert_eq!((res.genus(), f.euler), (Some(1), 0));
    let seg = Cavity::circle(p(0.0, 0.0), 4.0, CurveSpec::DEFAULT)
        .unwrap()
        .approximation()
        .unwrap()
        .segments;
    assert!(close(
        f.volume,
        (w * l - regular_polygon_area(seg, 4.0)) * t,
        1e-12
    ));
    // cavity: a rectangular void of 11 x 23 inside the outline.
    let res = drill(&body, hole_rect(6.0, 12.0, 17.0, 35.0), t, 0);
    assert_eq!(assert_sealed(&res).volume, (w * l - 11.0 * 23.0) * t);
    // thin-wall: a void leaving the minimum wall (0.001 mm = eps_geom) on every side.
    let e = 0.001;
    let inset = |hi: f64| {
        let mut y = hi - e;
        while hi - y < e {
            y = f64::from_bits(y.to_bits() - 1);
        }
        y
    };
    let (xi, yi) = (inset(w), inset(l));
    let res = drill(&body, hole_rect(e, e, xi, yi), t, 0);
    let f = assert_sealed(&res);
    assert_eq!(res.genus(), Some(1));
    assert!(close(f.volume, (w * l - (xi - e) * (yi - e)) * t, 1e-9));
    // One representable step tighter on any side is refused and nothing is returned.
    let tighter = hole_rect(f64::from_bits(e.to_bits() - 1), e, xi, yi);
    assert!(through_hole(&body, &HoleCutter::new(tighter, -1.0, 4.0, 0)).is_err());
    let one_more = hole_rect(e, e, f64::from_bits(xi.to_bits() + 1), yi);
    assert!(through_hole(&body, &HoleCutter::new(one_more, -1.0, 4.0, 0)).is_err());
}

#[test]
fn holes_that_touch_cross_or_leave_the_outline_are_refused() {
    let t = 8.0;
    let body = plate(40.0, 30.0, t);
    let cases: [(&str, Cavity); 6] = [
        ("touching the left edge", hole_rect(0.0, 5.0, 6.0, 9.0)),
        ("crossing the right edge", hole_rect(35.0, 5.0, 45.0, 9.0)),
        ("wholly outside", hole_rect(50.0, 5.0, 56.0, 9.0)),
        ("touching one corner", hole_rect(34.0, 24.0, 40.0, 30.0)),
        ("closer than the floor", hole_rect(0.0005, 5.0, 6.0, 9.0)),
        ("enclosing the plate", hole_rect(-1.0, -1.0, 41.0, 31.0)),
    ];
    for (why, cavity) in cases {
        let e = refuse(&body, cavity, t);
        assert_eq!(e.code, OpCode::ContourInvalid, "{why}");
        assert_eq!(e.field, OpField::Cutter, "{why}");
        assert!(e.profile.is_some(), "{why}");
        assert_eq!(e.status(), "invalid_input");
        assert_eq!(e.code.as_str(), "GEO_CONTOUR_INVALID");
    }
}

#[test]
fn holes_touching_overlapping_nesting_or_enclosing_other_holes_are_refused() {
    let t = 8.0;
    let first = drill(
        &plate(40.0, 30.0, t),
        hole_rect(10.0, 10.0, 20.0, 20.0),
        t,
        0,
    );
    let body = first.body();
    // Overlapping, crossing, touching along an edge and at a corner, and closer than the floor.
    for cavity in [
        hole_rect(15.0, 12.0, 25.0, 18.0),
        hole_rect(20.0, 12.0, 25.0, 18.0),
        hole_rect(20.0, 20.0, 25.0, 25.0),
        hole_rect(20.0005, 12.0, 25.0, 18.0),
    ] {
        let e = refuse(body, cavity, t);
        assert_eq!(e.code, OpCode::ContourInvalid);
        assert_eq!(e.field, OpField::Cutter);
        // The diagnostic names the new cutter: its cavity index is the existing hole count.
        assert_eq!(e.profile.unwrap().cavity, Some(1));
    }
    // A hole inside the existing hole is a valid island, and one enclosing it a nested cavity:
    // both are outside the pilot.
    for cavity in [
        hole_rect(12.0, 12.0, 18.0, 18.0),
        hole_rect(5.0, 5.0, 25.0, 25.0),
    ] {
        let e = refuse(body, cavity, t);
        assert_eq!(
            (e.code, e.status()),
            (OpCode::UnsupportedContour, "unsupported")
        );
    }
    // A clear second hole is accepted.
    assert!(
        through_hole(
            body,
            &HoleCutter::new(hole_rect(25.0, 10.0, 35.0, 20.0), -1.0, 9.0, 1)
        )
        .is_ok()
    );
}

#[test]
fn cutters_that_do_not_traverse_the_plate_are_refused_by_range() {
    let t = 8.0;
    let body = plate(40.0, 30.0, t);
    let cavity = || hole_rect(10.0, 10.0, 20.0, 20.0);
    let eps: f64 = 0.001;
    let cases: [(f64, f64, OpDefect); 9] = [
        (0.5, 9.0, OpDefect::CutterNotThrough),
        (-1.0, 7.5, OpDefect::CutterNotThrough),
        (1.0, 7.0, OpDefect::CutterNotThrough),
        (0.0, 9.0, OpDefect::CutterTangent),
        (-1.0, 8.0, OpDefect::CutterTangent),
        (0.0, 8.0, OpDefect::CutterTangent),
        (-0.0005, 9.0, OpDefect::CutterMargin),
        (-1.0, 8.0005, OpDefect::CutterMargin),
        (
            f64::from_bits((-eps).to_bits() - 1),
            9.0,
            OpDefect::CutterMargin,
        ),
    ];
    for (from, to, defect) in cases {
        let r = through_hole(&body, &HoleCutter::new(cavity(), from, to, 0));
        if defect == OpDefect::None {
            // Exactly at the floor is accepted (inclusive minimum) and one step below as well.
            assert!(r.is_ok());
            continue;
        }
        let e = r.unwrap_err();
        assert_eq!(
            (e.code, e.field, e.defect),
            (OpCode::OpUnsupported, OpField::Cutter, defect),
            "{from} {to}"
        );
    }
    assert!(through_hole(&body, &HoleCutter::new(cavity(), -eps, t + 2.0 * eps, 0)).is_ok());
    // Reversed, empty and non-finite ranges.
    let e = through_hole(&body, &HoleCutter::new(cavity(), 9.0, -1.0, 0)).unwrap_err();
    assert_eq!(
        (e.code, e.defect),
        (OpCode::NotPositive, OpDefect::CutterRange)
    );
    let e = through_hole(&body, &HoleCutter::new(cavity(), 2.0, 2.0, 0)).unwrap_err();
    assert_eq!(e.defect, OpDefect::CutterRange);
    for z in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for (a, b) in [(z, 9.0), (-1.0, z)] {
            let e = through_hole(&body, &HoleCutter::new(cavity(), a, b, 0)).unwrap_err();
            assert_eq!((e.code, e.stage), (OpCode::NonFinite, Stage::Finiteness));
        }
    }
}

#[test]
fn invalid_cavities_ordinals_and_ids_are_typed() {
    let t = 8.0;
    let body = plate(40.0, 30.0, t);
    let cutter = |c: Cavity| HoleCutter::new(c, -1.0, 9.0, 0);
    // Counter-clockwise (wrong winding), bow-tie, too few points, repeated point.
    let ccw = Cavity::polygon(vec![
        p(10.0, 10.0),
        p(20.0, 10.0),
        p(20.0, 20.0),
        p(10.0, 20.0),
    ]);
    let bow = Cavity::polygon(vec![
        p(10.0, 10.0),
        p(20.0, 20.0),
        p(20.0, 10.0),
        p(10.0, 20.0),
    ]);
    let two = Cavity::polygon(vec![p(10.0, 10.0), p(20.0, 10.0)]);
    let repeated = Cavity::polygon(vec![
        p(10.0, 10.0),
        p(10.0, 10.0),
        p(10.0, 20.0),
        p(20.0, 20.0),
        p(20.0, 10.0),
    ]);
    for c in [ccw, bow, repeated] {
        let e = through_hole(&body, &cutter(c)).unwrap_err();
        assert_eq!((e.code, e.field), (OpCode::ContourInvalid, OpField::Cutter));
    }
    assert_eq!(
        through_hole(&body, &cutter(two)).unwrap_err().code,
        OpCode::ContourInvalid
    );
    // Non-finite and over-cap coordinates, and too many points, before any geometry.
    let nan = Cavity::polygon(vec![p(10.0, 10.0), p(f64::NAN, 20.0), p(20.0, 10.0)]);
    assert_eq!(
        through_hole(&body, &cutter(nan)).unwrap_err().code,
        OpCode::NonFinite
    );
    let far = Cavity::polygon(vec![p(10.0, 10.0), p(10.0, 5.0e9), p(20.0, 10.0)]);
    assert_eq!(
        through_hole(&body, &cutter(far)).unwrap_err().code,
        OpCode::AboveLimit
    );
    let many = Cavity::polygon(
        (0..1025)
            .map(|i| p(f64::from(i), f64::from(i % 7)))
            .collect(),
    );
    assert_eq!(
        through_hole(&body, &cutter(many)).unwrap_err().code,
        OpCode::AboveLimit
    );
    // Ordinal rules.
    let ok = HoleCutter::new(hole_rect(10.0, 10.0, 20.0, 20.0), -1.0, 9.0, 999);
    let res = through_hole(&body, &ok).unwrap();
    assert!(res.face_table().iter().any(|id| id == "cut:f.h999.s3"));
    assert!(res.face_table().iter().all(|id| id.len() <= 22));
    let bad = HoleCutter::new(hole_rect(10.0, 10.0, 20.0, 20.0), -1.0, 9.0, 1000);
    let e = through_hole(&body, &bad).unwrap_err();
    assert_eq!((e.code, e.defect), (OpCode::AboveLimit, OpDefect::Ordinal));
    let again = HoleCutter::new(hole_rect(25.0, 10.0, 30.0, 20.0), -1.0, 9.0, 999);
    let e = through_hole(res.body(), &again).unwrap_err();
    assert_eq!((e.code, e.field), (OpCode::BlobInvalid, OpField::FaceTable));
    assert_eq!(
        e.code.f04_code(),
        beamcraft_model::error::ErrorCode::IntegrityError
    );
}

#[test]
fn boxes_triangular_prisms_and_rotated_boxes_are_prisms_and_sheared_ones_are_not() {
    // A box of any origin is a prism and takes a hole.
    let boxed = cuboid([5.0, -3.0, 10.0], [45.0, 27.0, 18.0]);
    let res = through_hole(
        &boxed,
        &HoleCutter::new(hole_rect(20.0, 5.0, 30.0, 15.0), 9.0, 19.0, 0),
    )
    .unwrap();
    assert_eq!(assert_sealed(&res).volume, (40.0 * 30.0 - 100.0) * 8.0);
    // A triangular prism takes a hole through its triangular caps.
    let tri = common_mesh::prism(40.0, 40.0, 8.0);
    let ids = ["t:f.a", "t:f.b", "t:f.c", "t:f.d", "t:f.e"];
    let faces = [0u16, 1, 2, 2, 3, 3, 4, 4];
    let body = body_of(&tri, &faces, &ids, &[None; 5]);
    let res = through_hole(
        &body,
        &HoleCutter::new(hole_rect(5.0, 5.0, 15.0, 10.0), -1.0, 9.0, 0),
    )
    .unwrap();
    assert_eq!(assert_sealed(&res).volume, (800.0 - 50.0) * 8.0);
    assert_eq!(res.genus(), Some(1));
    // The same box lying along X (axes swapped, winding reversed) is a prism along Z again.
    let turned = mapped(&boxed, |q| [q[2], q[1], q[0]], |n| [n[2], n[1], n[0]], true);
    let res = through_hole(
        &turned,
        &HoleCutter::new(hole_rect(12.0, -1.0, 14.0, 2.0), 4.0, 50.0, 0),
    )
    .unwrap();
    assert_eq!(assert_sealed(&res).volume, (8.0 * 30.0 - 6.0) * 40.0);
    // Shearing the top cap sideways keeps two horizontal caps but slants the walls.
    let mut slanted = common_mesh::cuboid([5.0, -3.0, 10.0], [45.0, 27.0, 18.0]);
    for q in &mut slanted.v {
        if q[2] > 14.0 {
            q[0] += 1.0;
        }
    }
    let ids = ["s:f.a", "s:f.b", "s:f.c", "s:f.d", "s:f.e", "s:f.f"];
    let sheared = body_of(&slanted, &CUBOID_FACES, &ids, &[None; 6]);
    let e = through_hole(
        &sheared,
        &HoleCutter::new(hole_rect(20.0, 5.0, 30.0, 15.0), 9.0, 19.0, 1),
    )
    .unwrap_err();
    assert_eq!(
        (e.code, e.field, e.defect),
        (OpCode::OpUnsupported, OpField::Body, OpDefect::BodyNotPrism)
    );
    // A pyramid-like body (apex above a square base) has vertices at three heights.
    let apex = [
        [0.0, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [4.0, 4.0, 0.0],
        [0.0, 4.0, 0.0],
        [2.0, 2.0, 3.0],
    ];
    let pyramid = common_mesh::Buf {
        v: apex.to_vec(),
        t: vec![
            [0, 2, 1],
            [0, 3, 2],
            [0, 1, 4],
            [1, 2, 4],
            [2, 3, 4],
            [3, 0, 4],
        ],
    };
    let body = body_of(
        &pyramid,
        &[0, 0, 1, 2, 3, 4],
        &["p:f.a", "p:f.b", "p:f.c", "p:f.d", "p:f.e"],
        &[None; 5],
    );
    let e = through_hole(
        &body,
        &HoleCutter::new(hole_rect(1.0, 1.0, 1.5, 1.5), -1.0, 5.0, 0),
    )
    .unwrap_err();
    assert_eq!(e.defect, OpDefect::BodyNotPrism);
}

#[test]
fn negative_zero_in_a_cutter_gives_the_same_bytes_as_positive_zero() {
    let body = cuboid([-10.0, -10.0, 0.0], [10.0, 10.0, 4.0]);
    let hole =
        |a: f64, b: f64| Cavity::polygon(vec![p(a, 2.0), p(2.0, 2.0), p(2.0, -2.0), p(b, -2.0)]);
    let negative = through_hole(&body, &HoleCutter::new(hole(-0.0, 0.0), -1.0, 5.0, 0)).unwrap();
    let positive = through_hole(&body, &HoleCutter::new(hole(0.0, 0.0), -1.0, 5.0, 0)).unwrap();
    let both = through_hole(&body, &HoleCutter::new(hole(-0.0, -0.0), -1.0, 5.0, 0)).unwrap();
    assert_sealed(&negative);
    assert_eq!(negative.blob(), positive.blob());
    assert_eq!(negative.mesh_digest(), positive.mesh_digest());
    assert_eq!(both.blob(), positive.blob());
    assert!(
        negative
            .body()
            .mesh()
            .vertices()
            .iter()
            .flatten()
            .all(|v| !v.is_sign_negative() || *v != 0.0)
    );
}

#[test]
fn a_hole_after_a_cut_still_respects_the_floor_of_the_original_body() {
    // The plate's F06 floor is eps = 2^-22 * D = 0.234375 mm. Halving it in X shrinks the cap
    // outline, whose own floor would be about 0.169 mm; the body keeps 0.234375 mm.
    let body = plate(786_432.0, 589_824.0, 10.0);
    assert_eq!(body.tolerance().eps_geom_mm(), 0.234_375);
    let half = plane_cut(
        &body,
        &CutPlane::new([393_216.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        Keep::Negative,
        0,
    )
    .unwrap();
    assert_eq!(half.body().tolerance().eps_geom_mm(), 0.234_375);
    let cavity = || hole_rect(0.2, 1000.0, 1000.0, 2000.0);
    // The cap outline alone would admit 0.2 mm of clearance to the wall ...
    let outline = rect(0.0, 0.0, 393_216.0, 589_824.0);
    let alone = beamcraft_kernel::profile::Profile::admit(&outline, &[cavity()]).unwrap();
    assert!(alone.tolerance().eps_geom_mm() < 0.2);
    // ... but the body's floor does not, before and after the cut.
    for b in [&body, half.body()] {
        let e = through_hole(b, &HoleCutter::new(cavity(), -1.0, 11.0, 5)).unwrap_err();
        assert_eq!((e.code, e.field), (OpCode::ContourInvalid, OpField::Cutter));
        assert!(e.profile.is_some());
    }
    // 0.25 mm of clearance clears the floor.
    let ok = through_hole(
        half.body(),
        &HoleCutter::new(hole_rect(0.25, 1000.0, 1000.0, 2000.0), -1.0, 11.0, 5),
    );
    assert!(ok.is_ok());
}
