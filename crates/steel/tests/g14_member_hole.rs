// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G14.1 member-hole tests: exact `A * t` wall volumes, genus per crossed wall, feature
//! ids of the amendment and every refusal of the first slice (blind, tangent, margin,
//! edge-touching footprint, parallel-wall breach, sloped and curved walls, ordinal clash).
//! The G14.2 slice-1 tests cover the directed oblique cutters: `A * t / |n . d|` volumes on
//! flat walls at any angle, the byte-exact `new` special case and the refusals that stay.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use crate::common;
use crate::common_mesh;
use crate::common_ops;

use beamcraft_kernel::diagnostic::Stage;
use beamcraft_kernel::ops::{Body, OpCode, OpDefect, OpField, OpKind, OpResult};
use beamcraft_kernel::profile::{Cavity, CurveSpec, Point2, extrude};
use beamcraft_kernel::trimesh::TriMesh;
use common::regular_polygon_area;
use common_ops::assert_sealed;
use std::f64::consts::PI;
use steel::kernel::member_hole::{MemberHoleCutter, member_through_hole};

const L: f64 = 400.0;
const B: f64 = 150.0;
const H: f64 = 150.0;
const TW: f64 = 10.0;
const TF: f64 = 12.0;
const R: f64 = 10.0;
const N: u32 = 48;

fn circle(cx: f64, cy: f64, r: f64) -> Cavity {
    Cavity::circle(Point2::new(cx, cy), r, CurveSpec::DEFAULT).unwrap()
}

/// A CCW I-shape in (u, v); `swapped` mirrors it into the plane so that the web walls face
/// the cutter axis instead of the flange walls.
fn i_outline(swapped: bool) -> Vec<Point2> {
    let pts = [
        (-B / 2.0, -H / 2.0),
        (B / 2.0, -H / 2.0),
        (B / 2.0, -H / 2.0 + TF),
        (TW / 2.0, -H / 2.0 + TF),
        (TW / 2.0, H / 2.0 - TF),
        (B / 2.0, H / 2.0 - TF),
        (B / 2.0, H / 2.0),
        (-B / 2.0, H / 2.0),
        (-B / 2.0, H / 2.0 - TF),
        (-TW / 2.0, H / 2.0 - TF),
        (-TW / 2.0, -H / 2.0 + TF),
        (-B / 2.0, -H / 2.0 + TF),
    ];
    let mapped: Vec<Point2> = if swapped {
        pts.iter().rev().map(|(u, v)| Point2::new(*v, *u)).collect()
    } else {
        pts.iter().map(|(u, v)| Point2::new(*u, *v)).collect()
    };
    mapped
}

/// Extrudes a CCW outline along Z and places it as an X-extrusion (member axis on X) by
/// the exact cyclic coordinate map `[z, x, y]`.
fn x_extruded(outer: &[Point2], cavities: &[Cavity]) -> Body {
    let ex = extrude(outer, cavities, L).unwrap();
    let mesh = ex.mesh();
    let v: Vec<[f64; 3]> = mesh.vertices().iter().map(|p| [p.z, p.x, p.y]).collect();
    let t: Vec<[u32; 3]> = mesh.triangles().to_vec();
    let faces: Vec<u16> = mesh.triangle_faces().to_vec();
    let normals = mesh
        .face_normals()
        .iter()
        .map(|n| n.as_ref().map(|w| [w.z(), w.x(), w.y()]))
        .collect();
    let ids: Vec<String> = mesh
        .face_table()
        .iter()
        .map(|s| s.replacen("extrusion:", "member:", 1))
        .collect();
    let buf = common_mesh::blob(&v, &t, &faces, ids.len() as u32, 0);
    Body::from_mesh(TriMesh::from_blob(&buf).unwrap(), ids, normals, None).unwrap()
}

fn i_body(web_to_cutter: bool) -> Body {
    x_extruded(&i_outline(web_to_cutter), &[])
}

fn hollow_body() -> Body {
    let (t, half) = (10.0, B / 2.0);
    let outer = vec![
        Point2::new(-half, -half),
        Point2::new(half, -half),
        Point2::new(half, half),
        Point2::new(-half, half),
    ];
    let inner = half - t;
    let cavity = vec![
        Point2::new(inner, -inner),
        Point2::new(-inner, -inner),
        Point2::new(-inner, inner),
        Point2::new(inner, inner),
    ];
    x_extruded(&outer, &[Cavity::polygon(cavity)])
}

fn tube_body() -> Body {
    // A solid 48-sided section: every band is its own face, so a hole crosses bands of
    // one strip each (the curve-approximated cavity wall is the merge-walk slice).
    let outer: Vec<Point2> = (0..N)
        .map(|i| {
            let a = 2.0 * PI * f64::from(i) / f64::from(N);
            Point2::new(B / 2.0 * a.cos(), B / 2.0 * a.sin())
        })
        .collect();
    x_extruded(&outer, &[])
}

/// The same 48-band tube with every odd/even pair of consecutive bands grouped into one
/// face: the curved wall then carries two strips per face around the apex (the
/// merge-walk carve case, a curve-approximated cavity wall's coarse face grouping).
fn coarse_tube_body() -> Body {
    let outer: Vec<Point2> = (0..N)
        .map(|i| {
            let a = 2.0 * PI * f64::from(i) / f64::from(N);
            Point2::new(B / 2.0 * a.cos(), B / 2.0 * a.sin())
        })
        .collect();
    let ex = extrude(&outer, &[], L).unwrap();
    let mesh = ex.mesh();
    let v: Vec<[f64; 3]> = mesh.vertices().iter().map(|p| [p.z, p.x, p.y]).collect();
    let t: Vec<[u32; 3]> = mesh.triangles().to_vec();
    let old_faces: Vec<u16> = mesh.triangle_faces().to_vec();
    let old_ids: Vec<String> = mesh.face_table().to_vec();
    let old_normals: Vec<Option<[f64; 3]>> = mesh
        .face_normals()
        .iter()
        .map(|n| n.as_ref().map(|w| [w.z(), w.x(), w.y()]))
        .collect();
    // The band index of every old wall face from its first triangle's section angle;
    // the end caps carry no band.
    let mut band_of_face: std::collections::BTreeMap<u16, Option<u32>> =
        std::collections::BTreeMap::new();
    let step = 2.0 * PI / f64::from(N);
    for (tri, &face) in t.iter().zip(&old_faces) {
        let p = v.get(tri[0] as usize).copied().unwrap_or([f64::NAN; 3]);
        let is_cap = p[0] == 0.0 || p[0] == L;
        let band = if is_cap {
            None
        } else {
            let mut angle = p[2].atan2(p[1]);
            if angle < 0.0 {
                angle += 2.0 * PI;
            }
            Some((angle / step).floor() as u32)
        };
        band_of_face.entry(face).or_insert(band);
    }
    // New faces: the caps one for one, the bands as their odd/even pairs.
    let mut new_ids: Vec<String> = Vec::new();
    let mut new_normals: Vec<Option<[f64; 3]>> = Vec::new();
    let mut remap: std::collections::BTreeMap<u16, u16> = std::collections::BTreeMap::new();
    let mut pair_slot: std::collections::BTreeMap<u32, u16> = std::collections::BTreeMap::new();
    for (old, band) in &band_of_face {
        let old = *old;
        match band {
            None => {
                remap.insert(old, u16::try_from(new_ids.len()).unwrap());
                new_ids.push(old_ids[old as usize].clone());
                new_normals.push(old_normals[old as usize]);
            }
            Some(band) => {
                let slot = *pair_slot.entry(band.div_ceil(2)).or_insert_with(|| {
                    let at = u16::try_from(new_ids.len()).unwrap();
                    new_ids.push(format!("member:band-pair{}", band.div_ceil(2)));
                    new_normals.push(old_normals[old as usize]);
                    at
                });
                remap.insert(old, slot);
            }
        }
    }
    let faces: Vec<u16> = old_faces
        .iter()
        .map(|f| remap.get(f).copied().unwrap_or_default())
        .collect();
    let buf = common_mesh::blob(&v, &t, &faces, new_ids.len() as u32, 0);
    Body::from_mesh(
        TriMesh::from_blob(&buf).unwrap(),
        new_ids,
        new_normals,
        None,
    )
    .unwrap()
}

fn tapered_body() -> Body {
    let outer = vec![
        Point2::new(-B / 2.0, -H / 2.0),
        Point2::new(B / 2.0, -H / 2.0),
        Point2::new(B / 4.0, H / 2.0),
        Point2::new(-B / 4.0, H / 2.0),
    ];
    x_extruded(&outer, &[])
}

/// A square section rotated by `deg`, so every wall is a sloped plane: each wall is a
/// single-strip chain of its own (the turns between walls are hard edges), and a central
/// `+Z` hole crosses the top and bottom walls through the merge-walk path.
fn rotated_square_body(deg: f64) -> Body {
    let half = 50.0;
    let th = deg.to_radians();
    let outer: Vec<Point2> = [(-half, -half), (half, -half), (half, half), (-half, half)]
        .iter()
        .map(|&(u, v)| {
            let (bu, bv) = (u * th.cos() - v * th.sin(), u * th.sin() + v * th.cos());
            Point2::new(bu, bv)
        })
        .collect();
    x_extruded(&outer, &[])
}

fn drill(body: &Body, cavity: Cavity, from: f64, to: f64, ordinal: u16) -> OpResult {
    member_through_hole(body, &MemberHoleCutter::new(cavity, from, to, ordinal)).unwrap()
}

fn refused(body: &Body, cavity: Cavity, from: f64, to: f64) -> beamcraft_kernel::ops::OpFailure {
    member_through_hole(body, &MemberHoleCutter::new(cavity, from, to, 9)).unwrap_err()
}

fn volume(body: &Body) -> f64 {
    let enc = body.volume_mm3();
    (enc.lo + enc.hi) / 2.0
}

fn genus(res: &OpResult) -> u32 {
    res.body()
        .analysis()
        .shells()
        .first()
        .unwrap()
        .genus
        .unwrap()
}

fn polygon_area() -> f64 {
    let segments = circle(0.0, 0.0, R).approximation().unwrap().segments;
    regular_polygon_area(segments, R)
}

fn polygon_segments() -> u32 {
    circle(0.0, 0.0, R).approximation().unwrap().segments
}

#[test]
fn web_hole_removes_area_times_web_thickness() {
    let body = i_body(true);
    let before = volume(&body);
    let res = drill(
        &body,
        circle(L / 2.0, 0.0, R),
        -TW / 2.0 - 1.0,
        TW / 2.0 + 1.0,
        1,
    );
    assert_sealed(&res);
    assert_eq!(res.kind(), OpKind::MemberThroughHole);
    assert_eq!(genus(&res), 1);
    let removed = before - volume(res.body());
    let expected = polygon_area() * TW;
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
    let analytic = res.analytic_volume_mm3().unwrap();
    let expect_result = before - expected;
    assert!(analytic.lo <= expect_result && expect_result <= analytic.hi);
    let walls: Vec<&String> = res
        .face_table()
        .iter()
        .filter(|id| id.starts_with("cut:f.h1.w0.s"))
        .collect();
    assert_eq!(walls.len(), usize::try_from(polygon_segments()).unwrap());
    assert!(res.face_table().iter().any(|id| id == "cut:f.h1.w0.s0"));
    assert!(
        res.face_table()
            .iter()
            .any(|id| *id == format!("cut:f.h1.w0.s{}", polygon_segments() - 1))
    );
}

#[test]
fn flange_range_hole_removes_area_times_flange_thickness() {
    let body = i_body(false);
    let before = volume(&body);
    let res = drill(
        &body,
        circle(L / 2.0, B / 4.0, R),
        H / 2.0 - TF - 1.0,
        H / 2.0 + 1.0,
        2,
    );
    assert_sealed(&res);
    assert_eq!(genus(&res), 1);
    let removed = before - volume(res.body());
    let expected = polygon_area() * TF;
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
}

#[test]
fn flange_through_hole_removes_both_walls() {
    let body = i_body(false);
    let before = volume(&body);
    let res = drill(
        &body,
        circle(L / 2.0, B / 4.0, R),
        -H / 2.0 - 1.0,
        H / 2.0 + 1.0,
        3,
    );
    assert_sealed(&res);
    assert_eq!(genus(&res), 2);
    let removed = before - volume(res.body());
    let expected = 2.0 * polygon_area() * TF;
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
}

#[test]
fn hollow_section_hole_crosses_two_walls() {
    let body = hollow_body();
    let before = volume(&body);
    let res = drill(
        &body,
        circle(L / 2.0, 0.0, R),
        -H / 2.0 - 1.0,
        H / 2.0 + 1.0,
        4,
    );
    assert_sealed(&res);
    // The hollow section itself is genus 1; two crossed walls add two more.
    assert_eq!(genus(&res), 3);
    let removed = before - volume(res.body());
    let expected = 2.0 * polygon_area() * 10.0;
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
}

#[test]
fn blind_range_is_refused() {
    let body = i_body(true);
    let err = refused(
        &body,
        circle(L / 2.0, 0.0, R),
        -TW / 2.0 - 1.0,
        TW / 2.0 - 2.0,
    );
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::CutterNotThrough);
}

#[test]
fn tangent_range_is_refused() {
    let body = i_body(true);
    let err = refused(&body, circle(L / 2.0, 0.0, R), -TW / 2.0 - 1.0, TW / 2.0);
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::CutterTangent);
}

#[test]
fn sub_floor_overshoot_is_refused() {
    let body = i_body(true);
    let err = refused(
        &body,
        circle(L / 2.0, 0.0, R),
        -TW / 2.0 - 1.0,
        TW / 2.0 - 5e-4,
    );
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::CutterMargin);
}

#[test]
fn footprint_touching_a_free_edge_is_refused() {
    let body = i_body(false);
    let err = refused(
        &body,
        circle(L / 2.0, B / 2.0 - R, R),
        H / 2.0 - TF - 1.0,
        H / 2.0 + 1.0,
    );
    assert_eq!(err.code, OpCode::ContourInvalid);
    assert_eq!(err.field, OpField::Cutter);
}

#[test]
fn footprint_over_a_parallel_wall_is_refused() {
    let body = i_body(false);
    let err = refused(
        &body,
        circle(L / 2.0, TW / 2.0 + 2.0, R),
        H / 2.0 - TF - 1.0,
        H / 2.0 + 1.0,
    );
    assert_eq!(err.code, OpCode::ContourInvalid);
}

// G14.2 slice 2: holes through faceted walls (a tube) with the strip registry.

#[test]
fn a_small_hole_cuts_a_tube_through_four_chains() {
    let body = tube_body();
    let before = volume(&body);
    // A round hole of radius 4 spans two bands on each side of the section; the opening
    // touches every band's lower boundary edge only (a 48-sided band is about 9.8 mm
    // wide, the hole is 8 mm).
    let res = drill(
        &body,
        circle(L / 2.0, 0.0, 4.0),
        -H / 2.0 - 1.0,
        H / 2.0 + 1.0,
        7,
    );
    assert_sealed(&res);
    assert_eq!(res.kind(), OpKind::MemberThroughHole);
    // The solid section is genus 0; the through hole adds one more.
    assert_eq!(genus(&res), 1);
    let removed = before - volume(res.body());
    let expected = tube_removed_volume(4.0, H);
    assert!(
        (removed - expected).abs() < 1e-4 * expected,
        "{removed} vs {expected}"
    );
    let analytic = res.analytic_volume_mm3().unwrap();
    let expect_result = before - expected;
    assert!(analytic.lo <= expect_result && expect_result <= analytic.hi);
    assert!(res.face_table().iter().any(|id| id == "cut:f.h7.w0.s0"));
}

/// The removed volume of a through hole across a solid `N`-band section: the flat prism
/// over the cavity's own approximation polygon minus the inclined-member term
/// `2·tan(γ)·∬|b|` (each band chord tilts by `γ = π/N` against the cutter axis). The
/// polygon is taken at the cavity's own radius: the CurveSpec segment count depends on
/// the radius (48 sides at r=4, 72 at R=10), so a far polygon's area does not scale as
/// `r²` — the old `polygon_area_of` base was that 72-side area.
fn tube_removed_volume(r: f64, height: f64) -> f64 {
    let pts: Vec<(f64, f64)> = circle(0.0, 0.0, r)
        .points()
        .iter()
        .map(|p| (p.x, p.y))
        .collect();
    let mut area2 = 0.0;
    let mut moment = 0.0;
    for (p, q) in pts.iter().zip(pts.iter().cycle().skip(1)) {
        let cross = p.0 * q.1 - q.0 * p.1;
        area2 += cross;
        moment += cross.abs() * (p.1 + q.1).abs() / 6.0;
    }
    let area = area2.abs() / 2.0;
    let gamma = PI / f64::from(N);
    area * height - 2.0 * gamma.tan() * moment
}

#[test]
fn a_tube_hole_repeats_byte_for_byte() {
    let body = tube_body();
    let cutter = MemberHoleCutter::new(circle(L / 2.0, 0.0, 4.0), -H / 2.0 - 1.0, H / 2.0 + 1.0, 7);
    let first = member_through_hole(&body, &cutter).unwrap();
    let second = member_through_hole(&body, &cutter).unwrap();
    assert_eq!(first.blob(), second.blob());
    assert_eq!(first.mesh_digest(), second.mesh_digest());
}

#[test]
fn a_hole_wider_than_a_facet_band_cuts_each_band_in_two() {
    let body = tube_body();
    let before = volume(&body);
    // A round hole of radius 10 is wider than a band: the opening touches both boundary
    // edges of the flanking bands and cuts each in two, the band events include the kink
    // crossings between neighbouring strips, and the merge walk emits one coherent band
    // whose sides carry the `.p<run>` suffixes past the mid-edge kink points.
    let cutter = MemberHoleCutter::new(circle(L / 2.0, 0.0, R), -H / 2.0 - 1.0, H / 2.0 + 1.0, 9);
    let res = member_through_hole(&body, &cutter).unwrap();
    assert_sealed(&res);
    assert_eq!(genus(&res), 1);
    let removed = before - volume(res.body());
    let expected = tube_removed_volume(R, H);
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
    // The moment certification brackets the removed volume to dust width; the closed
    // form above carries the apex-band approximation and may sit a hair outside it.
    let analytic = res.analytic_volume_mm3().unwrap();
    assert!(analytic.hi - analytic.lo < 1.0);
    assert!(analytic.lo <= before - removed && before - removed <= analytic.hi);
    // The band walk crosses the kink lines mid-edge, so the run suffixes appear.
    assert!(
        res.face_table()
            .iter()
            .any(|id| id.starts_with("cut:f.h9.w0.") && id.ends_with(".p1"))
    );
    // The merge walk is deterministic.
    let second = member_through_hole(&body, &cutter).unwrap();
    assert_eq!(res.blob(), second.blob());
    assert_eq!(res.mesh_digest(), second.mesh_digest());
}

#[test]
fn a_sloped_wall_hole_passes_the_merge_walk() {
    // A square section rotated by 20 degrees: every wall is one sloped strip, and the
    // central hole crosses the top and bottom walls as two single-strip chains.
    let body = rotated_square_body(20.0);
    let before = volume(&body);
    let res = drill(
        &body,
        circle(L / 2.0, 0.0, 8.0),
        -H / 2.0 - 1.0,
        H / 2.0 + 1.0,
        5,
    );
    assert_sealed(&res);
    assert_eq!(genus(&res), 1);
    // The two crossed walls are parallel (opposite sides of the square), so the removed
    // column is constant: the footprint area times the wall-to-wall height along `Z`,
    // the square's side over the cosine of the walls' tilt (the square rotated by `deg`
    // tilts its sides by `deg` against the section plane).
    let segments = circle(0.0, 0.0, 8.0).approximation().unwrap().segments;
    let footprint = regular_polygon_area(segments, 8.0);
    let expected = footprint * (100.0 / 20.0f64.to_radians().cos());
    let removed = before - volume(res.body());
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
    let analytic = res.analytic_volume_mm3().unwrap();
    let expect_result = before - expected;
    assert!(analytic.lo <= expect_result && expect_result <= analytic.hi);
    assert!(res.face_table().iter().any(|id| id == "cut:f.h5.w0.s0"));
}

#[test]
fn a_sloped_wall_hole_repeats_byte_for_byte() {
    let body = rotated_square_body(20.0);
    let cutter = MemberHoleCutter::new(circle(L / 2.0, 0.0, 8.0), -H / 2.0 - 1.0, H / 2.0 + 1.0, 6);
    let first = member_through_hole(&body, &cutter).unwrap();
    let second = member_through_hole(&body, &cutter).unwrap();
    assert_eq!(first.blob(), second.blob());
    assert_eq!(first.mesh_digest(), second.mesh_digest());
}

#[test]
fn a_hole_crosses_several_strips_of_one_coarse_face() {
    // The coarse tube groups the apex bands of each side into one face, so the wide
    // hole crosses both strips of one face per wall; the face is carved strip by strip
    // under its own id while the untouched partner bands keep their triangles.
    let body = coarse_tube_body();
    let before = volume(&body);
    let res = drill(
        &body,
        circle(L / 2.0, 0.0, R),
        -H / 2.0 - 1.0,
        H / 2.0 + 1.0,
        9,
    );
    assert_sealed(&res);
    assert_eq!(genus(&res), 1);
    let removed = before - volume(res.body());
    let expected = tube_removed_volume(R, H);
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
}

#[test]
fn a_footprint_vertex_a_hair_above_a_kink_line_refuses_honestly() {
    // A diamond whose top and bottom vertices sit one rounding step off the tube's
    // apex-band kink lines: the contour crossings of those lines land within the merge
    // tolerance of the vertices, their events fold together, and the dust slivers the
    // opening leaves on the flanking bands are entirely within the floor of the lines.
    // Every clip vertex of such a sliver is a boundary contact, which the carve
    // assembly refuses honestly (`BodyWallUnsupported`), never with an internal
    // self-check; the band event merge keeps the vertex anchor and the `Footprint`
    // resolution falls back to position, so no swallowed event surfaces as an
    // unreachable vertex.
    let diamond = vec![
        Point2::new(L / 2.0, 9.789464416503875),
        Point2::new(L / 2.0 + 8.0, 0.0),
        Point2::new(L / 2.0, -9.789464416503875),
        Point2::new(L / 2.0 - 8.0, 0.0),
    ];
    let body = tube_body();
    let err = member_through_hole(
        &body,
        &MemberHoleCutter::new(Cavity::polygon(diamond), -H / 2.0 - 1.0, H / 2.0 + 1.0, 7),
    )
    .unwrap_err();
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::BodyWallUnsupported);
}

#[test]
fn a_counter_clockwise_cavity_is_refused() {
    // The cavity contract is a clockwise footprint polygon; the chains' moment sum
    // signs against it, so a counter-clockwise cavity is refused up front, exactly.
    let ccw = vec![
        Point2::new(L / 2.0 - 6.0, -6.0),
        Point2::new(L / 2.0 + 6.0, -6.0),
        Point2::new(L / 2.0 + 6.0, 6.0),
        Point2::new(L / 2.0 - 6.0, 6.0),
    ];
    let body = i_body(true);
    let err = member_through_hole(
        &body,
        &MemberHoleCutter::new(Cavity::polygon(ccw), -TW / 2.0 - 1.0, TW / 2.0 + 1.0, 8),
    )
    .unwrap_err();
    assert_eq!(err.code, OpCode::ContourInvalid);
    assert_eq!(err.stage, Stage::Caps);
}

#[test]
fn a_non_convex_hole_in_a_tube_is_refused_or_cut_without_crashing() {
    let body = tube_body();
    // A non-convex footprint on a multi-strip chain is refused unchanged (the per-strip
    // clip relies on convex footprints), never a crash.
    let l_shape = vec![
        Point2::new(-6.0, -6.0),
        Point2::new(6.0, -6.0),
        Point2::new(6.0, -2.0),
        Point2::new(2.0, -2.0),
        Point2::new(2.0, 6.0),
        Point2::new(-6.0, 6.0),
    ];
    let cavity = Cavity::polygon(l_shape);
    let cut = member_through_hole(
        &body,
        &MemberHoleCutter::new(cavity, -H / 2.0 - 1.0, H / 2.0 + 1.0, 8),
    );
    if let Ok(res) = cut {
        assert_sealed(&res);
    }
}

#[test]
fn degenerate_faceted_inputs_never_crash() {
    let body = tube_body();
    // A tiny range, an inverted one, a sub-floor tangent on the tube wall.
    let _ = refused(&body, circle(L / 2.0, 0.0, 4.0), -H / 2.0, H / 2.0 - 0.0);
    let _ = refused(&body, circle(L / 2.0, 0.0, 4.0), -H / 2.0, H / 2.0 - 5e-4);
    let _ = refused(
        &body,
        circle(L / 2.0, 0.0, 4.0),
        H / 2.0 + 2.0,
        H / 2.0 + 3.0,
    );
}

#[test]
fn hole_meeting_a_sloped_wall_is_refused() {
    let body = tapered_body();
    let err = refused(
        &body,
        circle(L / 2.0, 0.0, R),
        -H / 2.0 - 1.0,
        H / 2.0 + 1.0,
    );
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::BodyWallUnsupported);
}

#[test]
fn a_reused_ordinal_is_refused() {
    let body = i_body(true);
    let once = drill(
        &body,
        circle(L / 2.0, 0.0, R),
        -TW / 2.0 - 1.0,
        TW / 2.0 + 1.0,
        1,
    );
    let err = member_through_hole(
        once.body(),
        &MemberHoleCutter::new(circle(L / 2.0, 20.0, R), -TW / 2.0 - 1.0, TW / 2.0 + 1.0, 1),
    )
    .unwrap_err();
    assert_eq!(err.code, OpCode::BlobInvalid);
    assert_eq!(err.defect, OpDefect::Ordinal);
}

#[test]
fn footprint_too_close_to_an_end_face_is_refused() {
    let body = i_body(true);
    let eps = body.tolerance().eps_geom_mm();
    let err = refused(
        &body,
        circle(R + eps / 2.0, 0.0, R),
        -TW / 2.0 - 1.0,
        TW / 2.0 + 1.0,
    );
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::CutterMargin);
    let err = refused(&body, circle(5.0, 0.0, R), -TW / 2.0 - 1.0, TW / 2.0 + 1.0);
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::CutterNotThrough);
}

#[test]
fn a_second_hole_keeps_the_first_intact() {
    let body = i_body(true);
    let first = drill(
        &body,
        circle(L / 2.0, 0.0, R),
        -TW / 2.0 - 1.0,
        TW / 2.0 + 1.0,
        1,
    );
    let second = drill(
        first.body(),
        circle(L / 4.0, 30.0, 5.0),
        -TW / 2.0 - 1.0,
        TW / 2.0 + 1.0,
        2,
    );
    assert_sealed(&second);
    assert_eq!(genus(&second), 2);
    assert!(second.face_table().iter().any(|id| id == "cut:f.h1.w0.s0"));
    assert!(second.face_table().iter().any(|id| id == "cut:f.h2.w0.s0"));
}

// G14.2 slice 4: directed cutters through chain walls.

#[test]
fn an_oblique_hole_cuts_a_tube_with_a_certified_enclosure() {
    // The oblique cutter through the faceted bands must seal with the moment-based
    // enclosure of the clip pieces and repeat byte for byte, like the +Z tube tests.
    // The carve's opening arcs carry this chain's ring vertices for the foreign kink
    // crossings (issue #420): the band walks them, so the carved walls meet the band
    // on identical vertices.
    let body = tube_body();
    let before = volume(&body);
    let cutter = MemberHoleCutter::directed(circle(L / 2.0, 0.0, 4.0), 3.0, 4.0, -76.0, 76.0, 11);
    let res = member_through_hole(&body, &cutter).unwrap();
    assert_sealed(&res);
    assert_eq!(res.kind(), OpKind::MemberThroughHole);
    // The solid section is genus 0; the through hole adds one more.
    assert_eq!(genus(&res), 1);
    let removed = before - volume(res.body());
    assert!(removed > 0.0);
    let analytic = res.analytic_volume_mm3().unwrap();
    assert!(analytic.hi - analytic.lo < 1.0);
    assert!(analytic.lo <= before - removed && before - removed <= analytic.hi);
    assert!(
        res.face_table()
            .iter()
            .any(|id| id.starts_with("cut:f.h11.w0."))
    );
    // The merge walk is deterministic.
    let second = member_through_hole(&body, &cutter).unwrap();
    assert_eq!(res.blob(), second.blob());
    assert_eq!(res.mesh_digest(), second.mesh_digest());
}

#[test]
fn an_oblique_hole_crosses_sloped_walls_with_the_closed_cosine_volume() {
    // A square section rotated by 20 degrees, the cutter axis 10 degrees off +Z: the two
    // crossed walls stay parallel, so the removed column is the wall-to-wall distance
    // 100 over |n . d| = cos(20 deg + 10 deg).
    let body = rotated_square_body(20.0);
    let before = volume(&body);
    let axis = ((10.0f64).to_radians().sin(), (10.0f64).to_radians().cos());
    let res = drill_directed(&body, circle(L / 2.0, 0.0, 8.0), axis, -76.0, 76.0, 12);
    assert_sealed(&res);
    assert_eq!(genus(&res), 1);
    let segments = circle(0.0, 0.0, 8.0).approximation().unwrap().segments;
    let footprint = regular_polygon_area(segments, 8.0);
    let expected = footprint * 100.0 / (30.0f64).to_radians().cos();
    let removed = before - volume(res.body());
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
    let analytic = res.analytic_volume_mm3().unwrap();
    let expect_result = before - expected;
    assert!(analytic.lo <= expect_result && expect_result <= analytic.hi);
    assert!(res.face_table().iter().any(|id| id == "cut:f.h12.w0.s0"));
}

// G14.2 slice 1: directed oblique cutters through flat walls.

fn drill_directed(
    body: &Body,
    cavity: Cavity,
    axis: (f64, f64),
    from: f64,
    to: f64,
    ordinal: u16,
) -> OpResult {
    member_through_hole(
        body,
        &MemberHoleCutter::directed(cavity, axis.0, axis.1, from, to, ordinal),
    )
    .unwrap()
}

fn refused_directed(
    body: &Body,
    cavity: Cavity,
    axis: (f64, f64),
    from: f64,
    to: f64,
) -> beamcraft_kernel::ops::OpFailure {
    member_through_hole(
        body,
        &MemberHoleCutter::directed(cavity, axis.0, axis.1, from, to, 9),
    )
    .unwrap_err()
}

#[test]
fn oblique_web_hole_removes_area_times_thickness_over_cosine() {
    let body = i_body(true);
    let before = volume(&body);
    let dz = 4.0 / 5.0;
    let res = drill_directed(&body, circle(L / 2.0, 0.0, R), (3.0, 4.0), -15.0, 15.0, 5);
    assert_sealed(&res);
    assert_eq!(res.kind(), OpKind::MemberThroughHole);
    assert_eq!(genus(&res), 1);
    let removed = before - volume(res.body());
    let expected = polygon_area() * TW / dz;
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
    let analytic = res.analytic_volume_mm3().unwrap();
    let expect_result = before - expected;
    assert!(analytic.lo <= expect_result && expect_result <= analytic.hi);
    assert!(res.face_table().iter().any(|id| id == "cut:f.h5.w0.s0"));
}

#[test]
fn oblique_flange_pair_removes_both_walls_over_cosine() {
    let body = i_body(false);
    let before = volume(&body);
    let dz = 20.0 / (1.0f64 + 400.0f64).sqrt();
    let res = drill_directed(
        &body,
        circle(L / 2.0, B / 4.0, R),
        (1.0, 20.0),
        -80.0,
        80.0,
        8,
    );
    assert_sealed(&res);
    assert_eq!(genus(&res), 2);
    let removed = before - volume(res.body());
    let expected = 2.0 * polygon_area() * TF / dz;
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
}

#[test]
fn horizontal_hole_through_a_hollow_section_crosses_the_side_walls() {
    let body = hollow_body();
    let before = volume(&body);
    let res = drill_directed(&body, circle(L / 2.0, 0.0, R), (1.0, 0.0), -80.0, 80.0, 6);
    assert_sealed(&res);
    // The hollow section itself is genus 1; two crossed walls add two more.
    assert_eq!(genus(&res), 3);
    let removed = before - volume(res.body());
    let expected = 2.0 * polygon_area() * 10.0;
    assert!(
        (removed - expected).abs() < 1e-6 * expected,
        "{removed} vs {expected}"
    );
}

#[test]
fn the_unit_direction_reproduces_new_bit_for_bit() {
    let body = i_body(true);
    let plain = member_through_hole(
        &body,
        &MemberHoleCutter::new(circle(L / 2.0, 0.0, R), -TW / 2.0 - 1.0, TW / 2.0 + 1.0, 7),
    )
    .unwrap();
    let unit = member_through_hole(
        &body,
        &MemberHoleCutter::directed(
            circle(L / 2.0, 0.0, R),
            0.0,
            1.0,
            -TW / 2.0 - 1.0,
            TW / 2.0 + 1.0,
            7,
        ),
    )
    .unwrap();
    assert_eq!(plain.blob(), unit.blob());
    assert_eq!(plain.mesh_digest(), unit.mesh_digest());
}

#[test]
fn an_oblique_range_ending_inside_a_wall_is_refused() {
    let body = i_body(true);
    let err = refused_directed(&body, circle(L / 2.0, 0.0, R), (3.0, 4.0), -15.0, 0.0);
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::CutterNotThrough);
}

#[test]
fn a_horizontal_footprint_over_a_parallel_face_is_refused() {
    let body = hollow_body();
    let err = refused_directed(&body, circle(L / 2.0, -60.0, R), (1.0, 0.0), -80.0, 80.0);
    assert_eq!(err.code, OpCode::ContourInvalid);
    assert_eq!(err.field, OpField::Cutter);
}

#[test]
fn degenerate_directions_are_refused() {
    let body = i_body(true);
    let err = refused_directed(&body, circle(L / 2.0, 0.0, R), (0.0, 0.0), -6.0, 6.0);
    assert_eq!(err.code, OpCode::ContourInvalid);
    let err = refused_directed(&body, circle(L / 2.0, 0.0, R), (1e-320, 0.0), -6.0, 6.0);
    assert_eq!(err.code, OpCode::ContourInvalid);
    let err = refused_directed(&body, circle(L / 2.0, 0.0, R), (f64::NAN, 1.0), -6.0, 6.0);
    assert_eq!(err.code, OpCode::NonFinite);
    let err = refused_directed(&body, circle(L / 2.0, 0.0, R), (1e300, 1.0), -6.0, 6.0);
    assert_eq!(err.code, OpCode::AboveLimit);
}

#[test]
fn an_oblique_hole_in_a_tapered_body_is_refused() {
    let body = tapered_body();
    // Slice 3 serves single-strip chains (sloped flat walls), so the oblique cutter is
    // no longer blanket-refused: its range genuinely ends inside the material (the
    // section line enters through the bottom wall beyond the range start and leaves
    // through the sloped wall inside it), and the honest refusal is the blind one.
    let err = refused_directed(&body, circle(L / 2.0, 0.0, R), (3.0, 4.0), -80.0, 80.0);
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::CutterNotThrough);
}

#[test]
fn a_perpendicular_hole_after_an_oblique_one_is_refused() {
    let body = i_body(true);
    let first = drill_directed(&body, circle(L / 2.0, 0.0, R), (3.0, 4.0), -15.0, 15.0, 1);
    assert_sealed(&first);
    // An oblique cutter leaves band walls that are parallel to no coordinate axis, so the
    // body no longer satisfies this operation's X-extrusion contract; the strip registry of
    // the later slices owns the re-merge.
    let err = refused(
        first.body(),
        circle(L / 4.0, 30.0, 5.0),
        -TW / 2.0 - 1.0,
        TW / 2.0 + 1.0,
    );
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::BodyNotPrism);
}
