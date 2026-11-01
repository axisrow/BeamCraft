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
    let outer: Vec<Point2> = (0..N)
        .map(|i| {
            let a = 2.0 * PI * f64::from(i) / f64::from(N);
            Point2::new(B / 2.0 * a.cos(), B / 2.0 * a.sin())
        })
        .collect();
    x_extruded(&outer, &[circle(0.0, 0.0, B / 2.0 - TW)])
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

#[test]
fn hole_in_a_tube_is_refused() {
    let body = tube_body();
    let err = refused(
        &body,
        circle(L / 2.0, 0.0, R),
        -H / 2.0 - 1.0,
        H / 2.0 + 1.0,
    );
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::BodyWallUnsupported);
    assert_eq!(err.field, OpField::Body);
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
    let err = refused_directed(&body, circle(L / 2.0, 0.0, R), (3.0, 4.0), -80.0, 80.0);
    assert_eq!(err.code, OpCode::OpUnsupported);
    assert_eq!(err.defect, OpDefect::BodyWallUnsupported);
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
