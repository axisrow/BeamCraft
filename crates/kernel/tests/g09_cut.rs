// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G09 plane-cut tests: closed-form and voxel oracles, topology, exact special positions,
//! metamorphic moves, canonical bytes and every refusal of the pilot scope.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use crate::common;
use crate::common_mesh;
use crate::common_ops;

use beamcraft_kernel::diagnostic::Stage;
use beamcraft_kernel::ops::{Body, CutPlane, Keep, OpCode, OpDefect, OpField, OpKind, plane_cut};
use beamcraft_kernel::profile::{Cavity, CurveSpec, Point2, extrude};
use common::{close, p, rect};
use common_mesh::Rng;
use common_ops::*;

fn cut(
    body: &Body,
    point: [f64; 3],
    normal: [f64; 3],
    keep: Keep,
) -> beamcraft_kernel::ops::OpResult {
    plane_cut(body, &CutPlane::new(point, normal), keep, 0).unwrap()
}

fn refusal(
    body: &Body,
    point: [f64; 3],
    normal: [f64; 3],
    keep: Keep,
) -> beamcraft_kernel::ops::OpFailure {
    plane_cut(body, &CutPlane::new(point, normal), keep, 0).unwrap_err()
}

fn point_on(n: [f64; 3], d: f64) -> [f64; 3] {
    // Some point with n . x = d for a normal with a non-zero x component.
    [d / n[0], 0.0, 0.0]
}

#[test]
fn corner_cut_of_a_cube_matches_the_pyramid_formula_and_its_complement() {
    let body = cuboid([0.0; 3], [4.0; 3]);
    let n = [1.0, 1.0, 1.0];
    let small = cut(&body, point_on(n, 3.0), n, Keep::Negative);
    let facts = assert_sealed(&small);
    // The tetrahedron x+y+z <= 3 has volume 27/6 and the three new vertices are exact.
    assert_eq!(facts.volume, 4.5);
    assert!(small.volume_mm3().contains(4.5));
    assert_eq!(small.stats().new_vertices, 6);
    assert_eq!(small.plane_deviation_mm(), Some(0.0));
    assert_eq!(facts.euler, 2);
    assert_eq!(small.genus(), Some(0));
    assert_eq!(small.kind(), OpKind::PlaneCut);
    // Faces x1, y1 and z1 lie wholly on the removed side; the other three keep their ids.
    assert_eq!(
        small.removed_faces(),
        ["box:f.x1", "box:f.y1", "box:f.z1"].map(str::to_owned)
    );
    assert_eq!(small.face_table().last().unwrap(), "cut:f.p0");
    let cap = small.face_normals().last().unwrap().unwrap();
    // Kept side is negative, so the cap faces the removed (positive) side.
    let s = 1.0 / 3.0f64.sqrt();
    assert!(cap.iter().all(|c| (c - s).abs() < 1e-15));

    let large = cut(&body, point_on(n, 3.0), n, Keep::Positive);
    let facts_large = assert_sealed(&large);
    assert_eq!(facts_large.volume, 64.0 - 4.5);
    let cap = large.face_normals().last().unwrap().unwrap();
    assert!(cap.iter().all(|c| (c + s).abs() < 1e-15));
    // Volume conservation: both halves share the exact cap vertices.
    assert_eq!(facts.volume + facts_large.volume, 64.0);
}

#[test]
fn hexagonal_sections_and_general_planes_match_closed_forms() {
    let l = [4.0, 6.0, 5.0];
    let body = cuboid([0.0; 3], l);
    let total = l[0] * l[1] * l[2];
    // (normal, d): the first is a hexagon with exact dyadic vertices, the others are not exact.
    for (n, d) in [
        ([1.0, 1.0, 1.0], 7.5),
        ([1.0, 2.0, 3.0], 9.3),
        ([0.3, 0.7, 0.11], 2.9),
        ([5.0, 1.0, 0.25], 11.0),
        ([1.0, 1.0, 1.0], 0.5),
        ([1.0, 1.0, 1.0], 14.5),
    ] {
        let expected = box_halfspace_volume(l, n, d);
        let low = cut(&body, point_on(n, d), n, Keep::Negative);
        let high = cut(&body, point_on(n, d), n, Keep::Positive);
        let (a, b) = (assert_sealed(&low), assert_sealed(&high));
        assert!(close(a.volume, expected, 1e-12), "{n:?} {d}");
        assert!(close(b.volume, total - expected, 1e-12), "{n:?} {d}");
        // Complement identity within a few ulps of the rounded cap vertices.
        assert!(close(a.volume + b.volume, total, 1e-12));
        // The result's own certified enclosure holds the closed form up to the f64 oracle error.
        let enc = low.volume_mm3();
        assert!(enc.lo <= expected * (1.0 + 1e-12) && expected * (1.0 - 1e-12) <= enc.hi);
        // The cap is planar up to the rounding of the new vertices, and the bound says so.
        assert!(low.plane_deviation_mm().unwrap() < 1e-12);
        assert_eq!(low.stats().new_vertices, high.stats().new_vertices);
    }
}

#[test]
fn planes_through_vertices_and_edges_are_decided_exactly() {
    let body = cuboid([0.0; 3], [4.0; 3]);
    // x + y = 4 contains the vertical edges (4,0,*) and (0,4,*), so four vertices are on the
    // plane and no edge needs a new vertex on the side faces; the diagonals decide the rest.
    let n = [1.0, 1.0, 0.0];
    for keep in [Keep::Negative, Keep::Positive] {
        let r = cut(&body, [4.0, 0.0, 0.0], n, keep);
        let f = assert_sealed(&r);
        assert_eq!(f.volume, 32.0);
        assert_eq!(f.euler, 2);
        assert!(
            r.stats().exact_predicates >= 4,
            "zero signs need exact evaluation"
        );
    }
    // x + y + z = 4 passes through three vertices of the cube.
    let n = [1.0, 1.0, 1.0];
    let r = cut(&body, [4.0, 0.0, 0.0], n, Keep::Negative);
    assert_eq!(assert_sealed(&r).volume, 64.0 / 6.0);
    // A plane through exactly one vertex that does cross the body: x + y + z = 12 touches the
    // far corner only (tangent, refused below); x + y + z = 8 crosses through no vertex.
    let r = cut(&body, [8.0, 0.0, 0.0], n, Keep::Negative);
    assert!(assert_sealed(&r).volume > 32.0);
}

#[test]
fn tangent_coplanar_outside_and_empty_planes_are_refused_without_a_result() {
    let body = cuboid([0.0; 3], [4.0; 3]);
    let z = [0.0, 0.0, 1.0];
    // Plane z = 4 is the top face: coplanar and tangent. Both keeps are refused.
    for keep in [Keep::Negative, Keep::Positive] {
        let e = refusal(&body, [0.0, 0.0, 4.0], z, keep);
        assert_eq!(
            (e.code, e.defect),
            (OpCode::OpUnsupported, OpDefect::PlaneTangent)
        );
        let e = refusal(&body, [0.0, 0.0, 0.0], z, keep);
        assert_eq!(
            (e.code, e.defect),
            (OpCode::OpUnsupported, OpDefect::PlaneTangent)
        );
    }
    // Touching at one vertex and at one edge.
    let e = refusal(&body, [12.0, 0.0, 0.0], [1.0, 1.0, 1.0], Keep::Negative);
    assert_eq!(e.defect, OpDefect::PlaneTangent);
    let e = refusal(&body, [8.0, 0.0, 0.0], [1.0, 1.0, 0.0], Keep::Negative);
    assert_eq!(e.defect, OpDefect::PlaneTangent);
    // Strictly outside: nothing removed (keep the body side) or nothing kept.
    let e = refusal(&body, [0.0, 0.0, 5.0], z, Keep::Negative);
    assert_eq!(
        (e.code, e.defect),
        (OpCode::OpNoEffect, OpDefect::PlaneOutside)
    );
    assert_eq!(e.code.as_str(), "GEO_OP_NO_EFFECT");
    assert_eq!(e.status(), "invalid_input");
    let e = refusal(&body, [0.0, 0.0, 5.0], z, Keep::Positive);
    assert_eq!(
        (e.code, e.defect),
        (OpCode::OpNoEffect, OpDefect::PlaneEmpty)
    );
    let e = refusal(&body, [0.0, 0.0, -1.0], z, Keep::Negative);
    assert_eq!(e.defect, OpDefect::PlaneEmpty);
    // A plane inside the cube that holds a whole face of a non-convex body: the step face.
    let l_shape = extrude(
        &[
            p(0.0, 0.0),
            p(4.0, 0.0),
            p(4.0, 2.0),
            p(2.0, 2.0),
            p(2.0, 4.0),
            p(0.0, 4.0),
        ],
        &[],
        1.0,
    )
    .unwrap();
    let l_body = Body::from_extrusion(&l_shape).unwrap();
    // y = 2 contains the step edge face (x in [2,4]) from inside the body: coplanar face.
    let e = refusal(&l_body, [0.0, 2.0, 0.0], [0.0, 1.0, 0.0], Keep::Negative);
    assert_eq!(
        (e.code, e.defect),
        (OpCode::OpUnsupported, OpDefect::PlaneCoplanarFace)
    );
}

#[test]
fn invalid_planes_and_ordinals_are_typed_and_leave_no_result() {
    let body = cuboid([0.0; 3], [4.0; 3]);
    let ok = [1.0, 0.0, 0.0];
    let bad = |point: [f64; 3], normal: [f64; 3]| {
        plane_cut(&body, &CutPlane::new(point, normal), Keep::Negative, 0).unwrap_err()
    };
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for k in 0..3 {
            let mut point = [1.0; 3];
            point[k] = v;
            let e = bad(point, ok);
            assert_eq!(
                (e.code, e.field, e.stage),
                (OpCode::NonFinite, OpField::Plane, Stage::Finiteness)
            );
            let mut normal = ok;
            normal[k] = v;
            assert_eq!(bad([1.0; 3], normal).code, OpCode::NonFinite);
        }
    }
    let e = bad([1.0; 3], [0.0; 3]);
    assert_eq!((e.code, e.defect), (OpCode::NotPositive, OpDefect::None));
    assert_eq!(bad([1.0; 3], [-0.0, 0.0, -0.0]).code, OpCode::NotPositive);
    assert_eq!(
        bad([4.000_000_001e9, 0.0, 0.0], ok).code,
        OpCode::AboveLimit
    );
    assert_eq!(
        bad([1.0; 3], [0.0, 4.000_000_001e9, 0.0]).code,
        OpCode::AboveLimit
    );
    // The cap and ordinal rules.
    let plane = CutPlane::new([1.0, 0.0, 0.0], ok);
    assert!(plane_cut(&body, &plane, Keep::Negative, 999).is_ok());
    let e = plane_cut(&body, &plane, Keep::Negative, 1000).unwrap_err();
    assert_eq!((e.code, e.defect), (OpCode::AboveLimit, OpDefect::Ordinal));
    // Cutting the same body twice with one ordinal collides on the cap id.
    let first = plane_cut(&body, &plane, Keep::Negative, 7).unwrap();
    let second = CutPlane::new([0.5, 0.0, 0.0], ok);
    let e = plane_cut(first.body(), &second, Keep::Negative, 7).unwrap_err();
    assert_eq!((e.code, e.field), (OpCode::BlobInvalid, OpField::FaceTable));
    assert!(plane_cut(first.body(), &second, Keep::Negative, 8).is_ok());
}

#[test]
fn a_scaled_normal_describes_the_same_plane() {
    let body = cuboid([0.0; 3], [4.0; 3]);
    let a = cut(&body, [1.0, 1.0, 1.0], [1.0, 2.0, 3.0], Keep::Negative);
    let b = cut(&body, [1.0, 1.0, 1.0], [4.0, 8.0, 12.0], Keep::Negative);
    assert_eq!(a.blob(), b.blob());
    assert_eq!(a.mesh_digest(), b.mesh_digest());
    // Reversing keep and normal is the same body.
    let c = cut(&body, [1.0, 1.0, 1.0], [-1.0, -2.0, -3.0], Keep::Positive);
    assert_eq!(a.blob(), c.blob());
    // Only the cap normal's sign convention follows the kept side, which is the same here.
    assert_eq!(a.face_normals().last(), c.face_normals().last());
}

#[test]
fn horizontal_cut_of_a_plate_with_a_hole_caps_a_section_with_a_cavity() {
    let (w, l, t) = (40.0, 30.0, 8.0);
    let hole = [Cavity::circle(p(20.0, 15.0), 5.0, CurveSpec::DEFAULT).unwrap()];
    let outer = rect(0.0, 0.0, w, l);
    let ex = extrude(&outer, &hole, t).unwrap();
    let body = Body::from_extrusion(&ex).unwrap();
    let area = common::net_area(&outer, &hole);
    for keep in [Keep::Negative, Keep::Positive] {
        let r = plane_cut(
            &body,
            &CutPlane::new([0.0, 0.0, 3.0], [0.0, 0.0, 1.0]),
            keep,
            2,
        )
        .unwrap();
        let f = assert_sealed(&r);
        let height = if keep == Keep::Negative { 3.0 } else { 5.0 };
        assert!(close(f.volume, area * height, 1e-12));
        // A through hole keeps genus one, and the cap has an outer loop and one hole.
        assert_eq!(r.genus(), Some(1));
        assert_eq!(f.euler, 0);
        assert_eq!(r.stats().loops, 2);
        assert!(r.face_table().contains(&"cut:f.p2".to_owned()));
        assert!(r.face_table().contains(&"extrusion:f.c0".to_owned()));
        // The hole's curved wall has no single normal, and the cap has an exact +-Z one.
        let at = r
            .face_table()
            .iter()
            .position(|id| id == "extrusion:f.c0")
            .unwrap();
        assert!(r.face_normals()[at].is_none());
        let cap = r.face_normals().last().unwrap().unwrap();
        assert_eq!(
            cap,
            if keep == Keep::Negative {
                [0.0, 0.0, 1.0]
            } else {
                [0.0, 0.0, -1.0]
            }
        );
    }
}

#[test]
fn a_vertical_cut_through_the_hole_has_two_section_loops_and_is_refused() {
    let hole = [Cavity::circle(p(20.0, 15.0), 5.0, CurveSpec::DEFAULT).unwrap()];
    let body =
        Body::from_extrusion(&extrude(&rect(0.0, 0.0, 40.0, 30.0), &hole, 8.0).unwrap()).unwrap();
    let plane = CutPlane::new([20.3, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let e = plane_cut(&body, &plane, Keep::Negative, 0).unwrap_err();
    assert_eq!(
        (e.code, e.defect),
        (OpCode::OpUnsupported, OpDefect::SectionMultiple)
    );
    assert_eq!(e.status(), "unsupported");
    // A vertical plane clear of the hole is one rectangle.
    let clear = CutPlane::new([10.3, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let r = plane_cut(&body, &clear, Keep::Negative, 0).unwrap();
    assert!(close(assert_sealed(&r).volume, 10.3 * 30.0 * 8.0, 1e-12));
}

fn clip_area(poly: &[[f64; 2]], n: [f64; 2], d: f64) -> f64 {
    // Sutherland-Hodgman against n . p <= d, then the shoelace area (independent oracle).
    let mut out: Vec<[f64; 2]> = Vec::new();
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (n[0] * a[0] + n[1] * a[1] - d, n[0] * b[0] + n[1] * b[1] - d);
        if da <= 0.0 {
            out.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            let t = da / (da - db);
            out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
        }
    }
    let mut area = 0.0;
    for i in 0..out.len() {
        let (a, b) = (out[i], out[(i + 1) % out.len()]);
        area += a[0] * b[1] - b[0] * a[1];
    }
    0.5 * area
}

#[test]
fn a_nonconvex_prism_is_cut_through_its_reflex_vertex_and_elsewhere() {
    let outline = [
        [0.0, 0.0],
        [23.0, 0.0],
        [23.0, 20.0],
        [10.0, 20.0],
        [10.0, 47.0],
        [0.0, 47.0],
    ];
    let pts: Vec<Point2> = outline.iter().map(|q| p(q[0], q[1])).collect();
    let t = 3.0;
    let body = Body::from_extrusion(&extrude(&pts, &[], t).unwrap()).unwrap();
    // x + y = 30 passes exactly through the reflex vertex (10, 20): a vertex-on-plane case.
    // The other lines cross the outline once without touching a vertex; 5x + 2y = 100 would\n    // leave and re-enter the body (two section loops) and is refused in its own test.
    for (n, d) in [
        ([1.0, 1.0], 30.0),
        ([2.0, 1.0], 39.7),
        ([3.0, 1.0], 33.7),
        ([1.0, 3.0], 58.0),
    ] {
        for keep in [Keep::Negative, Keep::Positive] {
            let normal = [n[0], n[1], 0.0];
            let plane = CutPlane::new([d / n[0], 0.0, 0.0], normal);
            if d == 30.0 && keep == Keep::Positive {
                // The kept side is two prisms that touch only along the reflex edge: a body
                // pinched at an edge is never sealed, and the failure is typed.
                let e = plane_cut(&body, &plane, keep, 0).unwrap_err();
                assert_eq!(e.code, OpCode::OpUnsupported);
                assert!(matches!(
                    e.defect,
                    OpDefect::ResultOverlapping | OpDefect::SectionPinched
                ));
                continue;
            }
            let r = plane_cut(&body, &plane, keep, 0)
                .unwrap_or_else(|e| panic!("{n:?} {d} {keep:?} {e:?}"));
            let f = assert_sealed(&r);
            let below = clip_area(&outline, n, d) * t;
            let expected = if keep == Keep::Negative {
                below
            } else {
                (23.0 * 20.0 + 10.0 * 27.0) * t - below
            };
            assert!(close(f.volume, expected, 1e-12), "{n:?} {d} {keep:?}");
            assert_eq!(f.euler, 2);
            // Vertical walls: every new vertex stays on the plane within rounding.
            assert!(r.plane_deviation_mm().unwrap() < 1e-12);
        }
    }
}

#[test]
fn voxel_counting_agrees_with_cut_volumes() {
    let l = [10.0, 8.0, 6.0];
    let body = cuboid([0.0; 3], l);
    let (n, d) = ([1.0, 0.5, 2.0], 9.0);
    let r = cut(&body, [d / n[0], 0.0, 0.0], n, Keep::Negative);
    let counted = voxel_volume([0.0; 3], l, 60, |q| {
        n[0] * q[0] + n[1] * q[1] + n[2] * q[2] <= d
    });
    let f = assert_sealed(&r);
    assert!(
        (f.volume - counted).abs() / f.volume < 0.01,
        "{} vs {counted}",
        f.volume
    );
}

#[test]
fn exact_motions_and_mirrors_commute_with_the_cut() {
    let body = cuboid([0.0; 3], [4.0, 6.0, 5.0]);
    let n = [1.0, 2.0, 3.0];
    let point = [1.5, 0.75, 2.25];
    let base = cut(&body, point, n, Keep::Negative);
    let v0 = assert_sealed(&base).volume;
    // Translation by a dyadic vector is exact: identical volume bits and vertex count.
    let t = [64.0, -32.5, 8.25];
    let moved = mapped(
        &body,
        |q| [q[0] + t[0], q[1] + t[1], q[2] + t[2]],
        |n| n,
        false,
    );
    let moved_point = [point[0] + t[0], point[1] + t[1], point[2] + t[2]];
    let r = cut(&moved, moved_point, n, Keep::Negative);
    assert!(close(assert_sealed(&r).volume, v0, 1e-12));
    // A quarter turn about Z: (x, y) -> (-y, x); the normal turns with it.
    let turned = mapped(
        &body,
        |q| [-q[1], q[0], q[2]],
        |n| [-n[1], n[0], n[2]],
        false,
    );
    let r = cut(
        &turned,
        [-point[1], point[0], point[2]],
        [-n[1], n[0], n[2]],
        Keep::Negative,
    );
    let f = assert_sealed(&r);
    assert!(close(f.volume, v0, 1e-12));
    assert_eq!(f.vertices, assert_sealed(&base).vertices);
    // Mirror x -> -x with the winding reversed and the normal mirrored.
    let mirrored = mapped(
        &body,
        |q| [-q[0], q[1], q[2]],
        |n| [-n[0], n[1], n[2]],
        true,
    );
    let r = cut(
        &mirrored,
        [-point[0], point[1], point[2]],
        [-n[0], n[1], n[2]],
        Keep::Negative,
    );
    assert!(close(assert_sealed(&r).volume, v0, 1e-12));
}

#[test]
fn renumbering_the_input_never_changes_the_bytes_and_repeats_are_identical() {
    let hole = [Cavity::circle(p(20.0, 15.0), 5.0, CurveSpec::DEFAULT).unwrap()];
    let bodies = [
        cuboid([0.0; 3], [4.0, 6.0, 5.0]),
        Body::from_extrusion(&extrude(&rect(0.0, 0.0, 40.0, 30.0), &hole, 8.0).unwrap()).unwrap(),
    ];
    let planes = [
        (
            CutPlane::new([1.0, 0.7, 2.0], [1.0, 2.0, 3.0]),
            Keep::Negative,
        ),
        (
            CutPlane::new([0.3, 4.1, 3.3], [0.0, 0.0, 1.0]),
            Keep::Positive,
        ),
        (
            CutPlane::new([2.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            Keep::Positive,
        ),
    ];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for body in &bodies {
        for (plane, keep) in &planes {
            let Ok(reference) = plane_cut(body, plane, *keep, 1) else {
                continue;
            };
            let again = plane_cut(body, plane, *keep, 1).unwrap();
            assert_eq!(reference.blob(), again.blob());
            assert_eq!(reference.mesh_digest(), again.mesh_digest());
            for _ in 0..12 {
                let shuffled = renumbered(body, &mut rng);
                let r = plane_cut(&shuffled, plane, *keep, 1).unwrap();
                assert_eq!(r.blob(), reference.blob());
                assert_eq!(r.mesh_digest(), reference.mesh_digest());
                assert_eq!(r.face_table(), reference.face_table());
                assert_eq!(r.face_normals(), reference.face_normals());
            }
        }
    }
}

#[test]
fn a_result_is_a_body_and_can_be_cut_again() {
    let body = cuboid([0.0; 3], [4.0; 3]);
    let first = cut(&body, [2.0, 0.0, 0.0], [1.0, 0.0, 0.0], Keep::Negative);
    let second = plane_cut(
        first.body(),
        &CutPlane::new([0.0, 2.0, 0.0], [0.0, 1.0, 0.0]),
        Keep::Negative,
        1,
    )
    .unwrap();
    assert_eq!(assert_sealed(&second).volume, 2.0 * 2.0 * 4.0);
    assert_eq!(second.face_table().last().unwrap(), "cut:f.p1");
    assert!(second.face_table().contains(&"cut:f.p0".to_owned()));
    // A third cut keeps both earlier ids.
    let third = plane_cut(
        second.body(),
        &CutPlane::new([0.0, 0.0, 2.0], [0.0, 0.0, 1.0]),
        Keep::Negative,
        2,
    )
    .unwrap();
    assert_eq!(assert_sealed(&third).volume, 2.0 * 2.0 * 2.0);
    assert_eq!(third.face_table().len(), 6);
}

#[test]
fn nonconvex_cut_bodies_and_input_gates() {
    // A hollow-looking body is refused at admission: a mirrored (inward) cube.
    let cube = common_mesh::inverted(&common_mesh::cuboid([0.0; 3], [4.0; 3]));
    let blob = common_mesh::blob(&cube.v, &cube.t, &CUBOID_FACES, 6, 0);
    let mesh = beamcraft_kernel::trimesh::TriMesh::from_blob(&blob).unwrap();
    let ids = ["b:f.a", "b:f.b", "b:f.c", "b:f.d", "b:f.e", "b:f.f"]
        .map(str::to_owned)
        .to_vec();
    let e = Body::from_mesh(mesh, ids, vec![None; 6], None).unwrap_err();
    assert_eq!(e.code, OpCode::MeshInward);
    // Two disjoint cubes are not a single shell.
    let two = common_mesh::merge(
        &common_mesh::cuboid([0.0; 3], [1.0; 3]),
        &common_mesh::cuboid([5.0; 3], [6.0; 3]),
    );
    let faces: Vec<u16> = CUBOID_FACES
        .iter()
        .copied()
        .chain(CUBOID_FACES.iter().map(|f| f + 6))
        .collect();
    let blob = common_mesh::blob(&two.v, &two.t, &faces, 12, 0);
    let mesh = beamcraft_kernel::trimesh::TriMesh::from_blob(&blob).unwrap();
    let ids: Vec<String> = (0..12)
        .map(|i| format!("b:f.{}", (b'a' + i as u8) as char))
        .collect();
    let e = Body::from_mesh(mesh, ids, vec![None; 12], None).unwrap_err();
    assert_eq!(
        (e.code, e.defect),
        (OpCode::OpUnsupported, OpDefect::BodyShells)
    );
    // An open mesh (one face missing).
    let open = common_mesh::cuboid([0.0; 3], [4.0; 3]);
    let t: Vec<[u32; 3]> = open.t[2..].to_vec();
    let blob = common_mesh::blob(&open.v, &t, &CUBOID_FACES[2..], 6, 0);
    let mesh = beamcraft_kernel::trimesh::TriMesh::from_blob(&blob).unwrap();
    let ids = ["b:f.a", "b:f.b", "b:f.c", "b:f.d", "b:f.e", "b:f.f"]
        .map(str::to_owned)
        .to_vec();
    let e = Body::from_mesh(mesh, ids, vec![None; 6], None).unwrap_err();
    assert!(matches!(
        e.code,
        OpCode::MeshNotClosed | OpCode::BlobInvalid
    ));
}

#[test]
fn a_line_that_leaves_and_reenters_a_nonconvex_prism_has_two_sections_and_is_refused() {
    let pts = [
        p(0.0, 0.0),
        p(23.0, 0.0),
        p(23.0, 20.0),
        p(10.0, 20.0),
        p(10.0, 47.0),
        p(0.0, 47.0),
    ];
    let body = Body::from_extrusion(&extrude(&pts, &[], 3.0).unwrap()).unwrap();
    // 5x + 2y = 100 crosses the upper arm, leaves the L past the reflex corner and re-enters
    // the lower arm: two section loops. Both halves are refused with the same typed reason.
    for keep in [Keep::Negative, Keep::Positive] {
        let e = plane_cut(
            &body,
            &CutPlane::new([20.0, 0.0, 0.0], [5.0, 2.0, 0.0]),
            keep,
            0,
        )
        .unwrap_err();
        assert_eq!(
            (e.code, e.field, e.defect),
            (
                OpCode::OpUnsupported,
                OpField::Section,
                OpDefect::SectionMultiple
            )
        );
    }
}

#[test]
fn normals_too_small_to_bound_are_refused_and_huge_ones_are_exact() {
    let body = cuboid([0.0; 3], [4.0; 3]);
    let tiny = CutPlane::new([1.0, 0.0, 0.0], [1e-101, 5e-324, 0.0]);
    let e = plane_cut(&body, &tiny, Keep::Negative, 0).unwrap_err();
    assert_eq!(
        (e.code, e.field, e.defect),
        (
            OpCode::NumericUncertain,
            OpField::Plane,
            OpDefect::NumericRange
        )
    );
    // 1e-100 is the smallest accepted largest component; scaling it up is the same plane.
    let small = CutPlane::new([1.0, 0.0, 0.0], [1e-100, 0.0, 0.0]);
    let big = CutPlane::new([1.0, 0.0, 0.0], [4.0e9, 0.0, 0.0]);
    let a = plane_cut(&body, &small, Keep::Negative, 0).unwrap();
    let b = plane_cut(&body, &big, Keep::Negative, 0).unwrap();
    assert_eq!(a.blob(), b.blob());
    assert_eq!(assert_sealed(&a).volume, 16.0);
}

#[test]
fn the_face_table_gate_rejects_every_malformed_table() {
    let cube = common_mesh::cuboid([0.0; 3], [4.0; 3]);
    let blob = common_mesh::blob(&cube.v, &cube.t, &CUBOID_FACES, 6, 0);
    let make = |ids: Vec<&str>, normals: Vec<Option<[f64; 3]>>| {
        let mesh = beamcraft_kernel::trimesh::TriMesh::from_blob(&blob).unwrap();
        Body::from_mesh(
            mesh,
            ids.into_iter().map(str::to_owned).collect(),
            normals,
            None,
        )
    };
    let ids = vec!["b:f.a", "b:f.b", "b:f.c", "b:f.d", "b:f.e", "b:f.f"];
    let normals: Vec<Option<[f64; 3]>> = CUBOID_NORMALS.iter().map(|n| Some(*n)).collect();
    assert!(make(ids.clone(), normals.clone()).is_ok());
    let invalid = |r: Result<Body, beamcraft_kernel::ops::OpFailure>, defect| {
        let e = r.unwrap_err();
        assert_eq!(
            (e.code, e.field, e.defect),
            (OpCode::BlobInvalid, OpField::FaceTable, defect)
        );
    };
    invalid(
        make(ids[..5].to_vec(), normals.clone()),
        OpDefect::FaceTableSize,
    );
    invalid(
        make(ids.clone(), normals[..5].to_vec()),
        OpDefect::FaceTableSize,
    );
    let mut dup = ids.clone();
    dup[5] = "b:f.a";
    invalid(make(dup, normals.clone()), OpDefect::FaceTableSize);
    for bad in [
        "B:f.a",
        "nokind",
        "b:",
        ":f.a",
        "b:f.a:x",
        "b:f.aaaaaaaaaaaaaaaaaaaa",
        "b:f_a",
        "b:f.é",
    ] {
        let mut table = ids.clone();
        table[0] = bad;
        invalid(make(table, normals.clone()), OpDefect::FaceTableEntry);
    }
    let mut skewed = normals.clone();
    skewed[0] = Some([2.0, 0.0, 0.0]);
    invalid(make(ids.clone(), skewed), OpDefect::FaceTableEntry);
    let mut nan = normals.clone();
    nan[0] = Some([f64::NAN, 0.0, 0.0]);
    invalid(make(ids.clone(), nan), OpDefect::FaceTableEntry);
    // A face that no triangle uses.
    let faces = [0u16, 0, 1, 1, 2, 2, 3, 3, 4, 4, 4, 4];
    let blob = common_mesh::blob(&cube.v, &cube.t, &faces, 6, 0);
    let mesh = beamcraft_kernel::trimesh::TriMesh::from_blob(&blob).unwrap();
    let e = Body::from_mesh(
        mesh,
        ids.iter().map(|s| (*s).to_owned()).collect(),
        normals,
        None,
    )
    .unwrap_err();
    assert_eq!(e.defect, OpDefect::FaceTableSize);
}

#[test]
fn a_cap_edge_beyond_the_g05_per_edge_cap_is_refused_with_the_profile_diagnostic() {
    // 1,900,000 mm long: the body is admissible, but its section at z = 1 has an edge above
    // the 1,000,000 mm cap that G05 enforces on every contour edge.
    let body = cuboid([0.0; 3], [1.9e6, 10.0, 10.0]);
    let e = plane_cut(
        &body,
        &CutPlane::new([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
        Keep::Negative,
        0,
    )
    .unwrap_err();
    assert_eq!((e.code, e.field), (OpCode::AboveLimit, OpField::Section));
    assert!(e.profile.is_some());
    // A section across the short direction is fine.
    let ok = plane_cut(
        &body,
        &CutPlane::new([7.0e5, 0.0, 0.0], [1.0, 0.0, 0.0]),
        Keep::Negative,
        0,
    );
    assert!(ok.is_ok());
}

#[test]
fn world_space_meshes_are_not_admitted_as_element_local_bodies() {
    let cube = common_mesh::cuboid([0.0; 3], [4.0; 3]);
    let ids = ["b:f.a", "b:f.b", "b:f.c", "b:f.d", "b:f.e", "b:f.f"].map(str::to_owned);
    for (space, expected) in [(0u32, true), (1, false)] {
        let blob = common_mesh::blob(&cube.v, &cube.t, &CUBOID_FACES, 6, space);
        let mesh = beamcraft_kernel::trimesh::TriMesh::from_blob(&blob).unwrap();
        let r = Body::from_mesh(mesh, ids.to_vec(), vec![None; 6], None);
        if expected {
            assert!(r.is_ok());
        } else {
            let e = r.unwrap_err();
            assert_eq!(
                (e.code, e.field, e.defect),
                (OpCode::BlobInvalid, OpField::Body, OpDefect::BodySpace)
            );
        }
    }
}

#[test]
fn supplied_normals_must_agree_with_the_oriented_faces() {
    let cube = common_mesh::cuboid([0.0; 3], [4.0; 3]);
    let blob = common_mesh::blob(&cube.v, &cube.t, &CUBOID_FACES, 6, 0);
    let ids = ["b:f.a", "b:f.b", "b:f.c", "b:f.d", "b:f.e", "b:f.f"].map(str::to_owned);
    let make = |normals: Vec<Option<[f64; 3]>>| {
        let mesh = beamcraft_kernel::trimesh::TriMesh::from_blob(&blob).unwrap();
        Body::from_mesh(mesh, ids.to_vec(), normals, None)
    };
    let good: Vec<Option<[f64; 3]>> = CUBOID_NORMALS.iter().map(|n| Some(*n)).collect();
    assert!(make(good.clone()).is_ok());
    // The x-min normal flipped inward, perpendicular, and slightly tilted (cos < 0.999999).
    let tilt: [f64; 3] = [-0.999_99, 0.004_472_091, 0.0];
    let len = (tilt[0] * tilt[0] + tilt[1] * tilt[1]).sqrt();
    for bad in [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, -1.0],
        [tilt[0] / len, tilt[1] / len, 0.0],
    ] {
        let mut normals = good.clone();
        normals[0] = Some(bad);
        let e = make(normals).unwrap_err();
        assert_eq!(
            (e.code, e.field, e.defect),
            (
                OpCode::BlobInvalid,
                OpField::FaceTable,
                OpDefect::FaceTableEntry
            ),
            "{bad:?}"
        );
    }
    // A curved face has no normal and is not checked.
    let mut curved = good;
    curved[0] = None;
    assert!(make(curved).is_ok());
}

#[test]
fn new_vertices_are_the_correctly_rounded_exact_intersections() {
    // The reviewer's case: the edge (-1,0,0)-(1,0,0) cut by x = 0.1 must give exactly 0.1.
    let body = cuboid([-1.0; 3], [1.0; 3]);
    let r = cut(&body, [0.1, 0.0, 0.0], [1.0, 0.0, 0.0], Keep::Negative);
    let new: Vec<[f64; 3]> = r
        .body()
        .mesh()
        .vertices()
        .iter()
        .copied()
        .filter(|v| v[0] == 0.1)
        .collect();
    assert_eq!(
        new.len(),
        8,
        "every crossing vertex lies exactly at x = 0.1"
    );
    assert!(r.body().mesh().vertices().iter().all(|v| v[0] <= 0.1));
    assert_eq!(r.plane_deviation_mm(), Some(0.0));
    // Oblique planes with d / n0 exact, through an integer box: the exact intersection of a segment with n.x = d
    // is (A (n.B - d) - B (n.A - d)) / (n.(B - A)) with small integers, so a correctly rounded
    // IEEE division of exact integers is an independent oracle for every coordinate.
    let corners: Vec<[f64; 3]> = (0..8)
        .map(|i| {
            [
                f64::from(i & 1) * 7.0,
                f64::from(i >> 1 & 1) * 5.0,
                f64::from(i >> 2 & 1) * 3.0,
            ]
        })
        .collect();
    let body = cuboid([0.0; 3], [7.0, 5.0, 3.0]);
    let mut checked = 0;
    for (n, d) in [
        ([3.0, 5.0, 7.0], 42.0),
        ([1.0, 1.0, 1.0], 6.0),
        ([2.0, -3.0, 5.0], 4.0),
        ([7.0, 11.0, 13.0], 56.0),
        ([-1.0, 4.0, 9.0], 23.0),
    ] {
        let mut oracle: Vec<[f64; 3]> = Vec::new();
        for a in &corners {
            for b in &corners {
                let (fa, fb) = (
                    n[0] * a[0] + n[1] * a[1] + n[2] * a[2] - d,
                    n[0] * b[0] + n[1] * b[1] + n[2] * b[2] - d,
                );
                if fa < 0.0 && fb > 0.0 {
                    let den = fb - fa;
                    let mut p = [0.0; 3];
                    for k in 0..3 {
                        p[k] = (a[k] * fb - b[k] * fa) / den;
                    }
                    oracle.push(p);
                }
            }
        }
        let r = cut(&body, [d / n[0], 0.0, 0.0], n, Keep::Negative);
        for v in r.body().mesh().vertices() {
            if corners.contains(v) {
                continue;
            }
            assert!(
                oracle.contains(v),
                "{v:?} is not a correctly rounded intersection for {n:?}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 20);
}
