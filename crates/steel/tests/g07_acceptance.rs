// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G07 acceptance probes of the first kernel scope.
//!
//! These tests close gaps found while tracing the kernel validation plan and the frozen F06
//! cases to evidence: straight-member meshes against the published F06 vertex lists, systematic
//! hostile-scalar sweeps over every public builder and operation, open or non-manifold meshes
//! that must never receive an exact status, a near-tolerance plane cut, and pinned digests that
//! show byte identity across runs and, once CI runs them, across targets.
//!
//! Expected values come from the F06 case file (copied as plain numbers), closed forms and the
//! independent readers in `common`, `common_mesh` and `common_ops`; none calls production math.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use crate::common;
use crate::common_mesh;
use crate::common_ops;

use beamcraft_geom::Point3;
use beamcraft_kernel::diagnostic::{Code, Stage};
use beamcraft_kernel::measure::{QuantityStatus, measure};
use beamcraft_kernel::ops::{
    Body, CutPlane, HoleCutter, Keep, OpCode, OpDefect, OpField, plane_cut, through_hole,
};
use beamcraft_kernel::profile::{Cavity, CurveSpec, Point2, extrude};
use beamcraft_kernel::topology::analyze;
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_kernel::verdict::{MeshCode, ShellStatus};
use beamcraft_kernel::{PlateResult, RectangularPlate, rectangular_plate};
use beamcraft_model::digest::Sha256;
use beamcraft_model::error::ErrorCode;
use common::{assert_valid_extrusion, mesh_volume, parse_blob, topology};
use common_mesh::{Buf, cuboid};
use common_ops::assert_sealed;
use steel::kernel::StraightMember;
use steel::kernel::straight_member;

fn member(start: [f64; 3], end: [f64; 3], w: f64, h: f64, roll: Option<f64>) -> StraightMember {
    StraightMember {
        start: Point3::new(start[0], start[1], start[2]),
        end: Point3::new(end[0], end[1], end[2]),
        width_mm: w,
        height_mm: h,
        roll_rad: roll,
        placement: steel::model::command::SectionPlacement::default(),
    }
}

fn vertices_of(r: &PlateResult) -> Vec<[f64; 3]> {
    r.mesh()
        .vertices()
        .iter()
        .map(|p| [p.x, p.y, p.z])
        .collect()
}

fn area_of(v: &[[f64; 3]], t: &[[u32; 3]]) -> f64 {
    t.iter()
        .map(|tri| {
            let [a, b, c] = tri.map(|i| v[i as usize]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let x = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            0.5 * (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt()
        })
        .sum()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// Independent structural facts of a sealed box: blob readable, closed, Euler 2, outward.
fn assert_sealed_box(r: &PlateResult) -> f64 {
    let blob = parse_blob(r.mesh().blob());
    assert_eq!(blob.vertices.len(), 8);
    assert_eq!(blob.triangles.len(), 12);
    assert_eq!(blob.face_count, 6);
    let topo = topology(&blob.triangles, 8);
    assert!(topo.closed && topo.components == 1 && topo.vertices_used == 8);
    assert_eq!(8_i64 - topo.undirected_edges as i64 + 12, 2);
    let volume = mesh_volume(&blob.vertices, &blob.triangles);
    assert!(volume > 0.0 && volume.is_finite(), "outward winding");
    volume
}

// ------------------------------------------------------------- F06 member meshes

/// F06-POS-04 to POS-08: vertex lists, volume and area copied from the frozen case file.
#[test]
fn f06_member_cases_match_the_published_vertices_volumes_and_areas() {
    type Case = (
        &'static str,
        [[f64; 3]; 2],
        [f64; 2],
        [[f64; 3]; 8],
        f64,
        f64,
    );
    let cases: [Case; 5] = [
        (
            "POS-04",
            [[0.0, 0.0, 0.0], [4000.0, 0.0, 0.0]],
            [200.0, 300.0],
            [
                [0.0, -100.0, -150.0],
                [0.0, 100.0, -150.0],
                [0.0, -100.0, 150.0],
                [0.0, 100.0, 150.0],
                [4000.0, -100.0, -150.0],
                [4000.0, 100.0, -150.0],
                [4000.0, -100.0, 150.0],
                [4000.0, 100.0, 150.0],
            ],
            240_000_000.0,
            4_120_000.0,
        ),
        (
            "POS-05",
            [[0.0, 0.0, 0.0], [0.0, 0.0, 3000.0]],
            [200.0, 300.0],
            [
                [-100.0, -150.0, 0.0],
                [100.0, -150.0, 0.0],
                [-100.0, 150.0, 0.0],
                [100.0, 150.0, 0.0],
                [-100.0, -150.0, 3000.0],
                [100.0, -150.0, 3000.0],
                [-100.0, 150.0, 3000.0],
                [100.0, 150.0, 3000.0],
            ],
            180_000_000.0,
            3_120_000.0,
        ),
        (
            "POS-06",
            [[0.0, 0.0, 3000.0], [0.0, 0.0, 0.0]],
            [200.0, 300.0],
            [
                [100.0, -150.0, 3000.0],
                [-100.0, -150.0, 3000.0],
                [100.0, 150.0, 3000.0],
                [-100.0, 150.0, 3000.0],
                [100.0, -150.0, 0.0],
                [-100.0, -150.0, 0.0],
                [100.0, 150.0, 0.0],
                [-100.0, 150.0, 0.0],
            ],
            180_000_000.0,
            3_120_000.0,
        ),
        (
            "POS-07",
            [[0.0, 0.0, 0.0], [0.0, 3000.0, 4000.0]],
            [200.0, 300.0],
            [
                [100.0, 120.0, -90.0],
                [-100.0, 120.0, -90.0],
                [100.0, -120.0, 90.0],
                [-100.0, -120.0, 90.0],
                [100.0, 3120.0, 3910.0],
                [-100.0, 3120.0, 3910.0],
                [100.0, 2880.0, 4090.0],
                [-100.0, 2880.0, 4090.0],
            ],
            300_000_000.0,
            5_120_000.0,
        ),
        (
            "POS-08",
            [[0.0, 0.0, 0.0], [3000.0, 4000.0, 0.0]],
            [100.0, 300.0],
            [
                [40.0, -30.0, -150.0],
                [-40.0, 30.0, -150.0],
                [40.0, -30.0, 150.0],
                [-40.0, 30.0, 150.0],
                [3040.0, 3970.0, -150.0],
                [2960.0, 4030.0, -150.0],
                [3040.0, 3970.0, 150.0],
                [2960.0, 4030.0, 150.0],
            ],
            150_000_000.0,
            4_060_000.0,
        ),
    ];
    for (name, axis, [w, h], expected, volume, area) in cases {
        let r = straight_member(member(axis[0], axis[1], w, h, None)).unwrap();
        let got = vertices_of(&r);
        for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
            for k in 0..3 {
                assert!(
                    (g[k] - e[k]).abs() <= 1e-9,
                    "{name}: vertex {i} component {k}: {g:?} vs {e:?}"
                );
            }
        }
        let mesh_v = assert_sealed_box(&r);
        assert!(common::close(r.volume_mm3(), volume, 1e-12), "{name}");
        assert!(common::close(mesh_v, volume, 1e-9), "{name}");
        let mesh_area = area_of(&got, r.mesh().triangles());
        assert!(common::close(mesh_area, area, 1e-9), "{name}: area");
    }
}

/// F06-POS-09, POS-10, POS-41, POS-42, POS-46 and POS-52: at the parallel threshold the mesh is
/// still a rectangular prism whose end sections are perpendicular to the axis. A fixed +Y
/// fallback (the historical defect) leaves a cosine of 5e-7 and fails this bound.
#[test]
fn f06_near_vertical_members_keep_sections_perpendicular_to_the_axis() {
    let inputs: [(&str, [f64; 3], Option<f64>); 6] = [
        ("POS-09", [0.0005, 0.0, 1000.0], None),
        ("POS-10", [0.002, 0.0, 1000.0], None),
        ("POS-41", [0.0, 0.08000002, 160_000.08], None),
        ("POS-42", [0.0, 0.002, 1000.0], None),
        ("POS-46", [0.5, 0.0, 500_000.0], None),
        (
            "POS-52",
            [0.0, 0.08000002, 160_000.08],
            Some(core::f64::consts::FRAC_PI_2),
        ),
    ];
    for (name, d, roll) in inputs {
        let r = straight_member(member([0.0; 3], d, 200.0, 300.0, roll)).unwrap();
        assert_sealed_box(&r);
        let v = vertices_of(&r);
        // Vertex index bits: xb = 1, yb = 2, zb = 4 (F06 section 6).
        let e1 = sub(v[1], v[0]);
        let e2 = sub(v[2], v[0]);
        let e3 = sub(v[4], v[0]);
        let cos = |a: [f64; 3], b: [f64; 3]| dot(a, b) / (norm(a) * norm(b));
        assert!(cos(e1, e2).abs() <= 1e-12, "{name}: width against height");
        assert!(cos(e1, e3).abs() <= 1e-12, "{name}: width against axis");
        assert!(cos(e2, e3).abs() <= 1e-12, "{name}: height against axis");
        assert!((norm(e1) - 200.0).abs() <= 1e-9 * 200.0, "{name}: width");
        assert!((norm(e2) - 300.0).abs() <= 1e-9 * 300.0, "{name}: height");
        // The axis edge is the submitted p2 - p1 and the section is centred on p1.
        for k in 0..3 {
            assert!((e3[k] - d[k]).abs() <= 1e-9, "{name}: axis edge");
            let centre = (v[0][k] + v[1][k] + v[2][k] + v[3][k]) / 4.0;
            assert!(
                centre.abs() <= 1e-9,
                "{name}: start section is centred on p1"
            );
        }
        let unit = |a: [f64; 3], s: f64| a.map(|x| x / s);
        let (x, y) = (unit(e1, 200.0), unit(e2, 300.0));
        let near = |g: [f64; 3], e: [f64; 3]| g.iter().zip(e).all(|(a, b)| (a - b).abs() <= 1e-12);
        match name {
            // Column rule: Y_m is exactly +Y.
            "POS-09" | "POS-46" => assert!(near(y, [0.0, 1.0, 0.0]), "{name}: {y:?}"),
            // Horizontal rule just outside the threshold.
            "POS-10" => {
                assert!(
                    near(y, [-0.999_999_999_998, 0.0, 1.999_999_999_996e-6]),
                    "{y:?}"
                );
                assert!(near(x, [0.0, 1.0, 0.0]), "{x:?}");
            }
            "POS-42" => {
                assert!(
                    near(y, [0.0, -0.999_999_999_998, 1.999_999_999_996e-6]),
                    "{y:?}"
                );
                assert!(near(x, [-1.0, 0.0, 0.0]), "{x:?}");
            }
            // Projected +Y fallback: perpendicular to the axis, exact rational frame.
            "POS-41" => {
                assert!(near(x, [1.0, 0.0, 0.0]), "{x:?}");
                assert!(
                    near(y, [0.0, 0.999_999_999_999_875, -4.999_998_75e-7]),
                    "{y:?}"
                );
            }
            // Roll pi/2 after the column rule: X_m = old Y_m, Y_m = -old X_m.
            "POS-52" => {
                assert!(
                    near(x, [0.0, 0.999_999_999_999_875, -4.999_998_75e-7]),
                    "{x:?}"
                );
                assert!(near(y, [-1.0, 0.0, 0.0]), "{y:?}");
            }
            other => panic!("unlisted case {other}"),
        }
    }
}

/// F06-POS-51 and POS-14: a quarter-turn roll swaps the section axes, and the minimum axis
/// length is accepted exactly at its inclusive limit while a shorter one is `GEO_AXIS_TOO_SHORT`.
#[test]
fn f06_member_roll_and_axis_minimum() {
    let r = straight_member(member(
        [0.0; 3],
        [4000.0, 0.0, 0.0],
        200.0,
        300.0,
        Some(core::f64::consts::FRAC_PI_2),
    ))
    .unwrap();
    assert_sealed_box(&r);
    let v = vertices_of(&r);
    for k in 0..3 {
        let lo = v.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
        let hi = v.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
        let (elo, ehi) = [(0.0, 4000.0), (-150.0, 150.0), (-100.0, 100.0)][k];
        assert!(
            (lo - elo).abs() < 1e-9 && (hi - ehi).abs() < 1e-9,
            "axis {k}"
        );
    }
    // F06-POS-14: member axis 0.001 mm, section 200 x 300: volume 60 mm3.
    let minimal = straight_member(member([0.0; 3], [0.001, 0.0, 0.0], 200.0, 300.0, None)).unwrap();
    assert_eq!(minimal.volume_mm3(), 60.0);
    assert_sealed_box(&minimal);
    // F06-NEG-04 (0.0009) and the next float below the floor: GEO_AXIS_TOO_SHORT, range stage.
    for axis_x in [0.0009, 0.001_f64.next_down(), 0.0] {
        let failure =
            straight_member(member([0.0; 3], [axis_x, 0.0, 0.0], 200.0, 300.0, None)).unwrap_err();
        let d = failure.diagnostics().next().unwrap();
        assert_eq!(d.code, Code::AxisTooShort, "{axis_x}");
        assert_eq!(d.code.as_str(), "GEO_AXIS_TOO_SHORT");
        assert_eq!(d.code.f04_code(), ErrorCode::InvalidArgument);
        assert_eq!(d.stage, Stage::Ranges);
        assert_eq!(failure.status(), "invalid_input");
    }
}

/// F06-POS-15, POS-50, POS-57, POS-58 and the member failure cases NEG-04 to NEG-07, NEG-43,
/// NEG-44, NEG-47, NEG-48 and NEG-51 to NEG-53, through the real builder: the diagnostic code,
/// its F04 mapping, the status and the evaluation stage are the published ones.
#[test]
fn f06_member_scaled_floor_cap_and_failure_cases() {
    // POS-15: the section dimension exactly at its 1,000,000 mm cap is accepted.
    let cap = straight_member(member(
        [0.0; 3],
        [0.0, 0.0, 1000.0],
        1_000_000.0,
        1_000_000.0,
        None,
    ))
    .unwrap();
    assert_eq!(cap.volume_mm3(), 1.0e15);
    assert_sealed_box(&cap);
    // POS-50: an explicit zero roll equals an omitted roll, byte for byte.
    let plain = member([0.0; 3], [4000.0, 0.0, 0.0], 200.0, 300.0, None);
    let zero = StraightMember {
        roll_rad: Some(0.0),
        ..plain
    };
    assert_eq!(
        straight_member(plain).unwrap().mesh().blob(),
        straight_member(zero).unwrap().mesh().blob()
    );
    // POS-57 and POS-58: the scaled floor accepts a dimension exactly above it.
    straight_member(member(
        [0.0; 3],
        [0.0791015625, 0.0, 0.0],
        393_216.0,
        524_288.0,
        None,
    ))
    .unwrap();
    straight_member(member(
        [0.0; 3],
        [524_288.0, 0.0, 0.0],
        0.1259765625,
        300.0,
        None,
    ))
    .unwrap();

    let ok = member([0.0; 3], [0.0, 0.0, 1000.0], 200.0, 300.0, None);
    type Expect = (
        &'static str,
        StraightMember,
        Code,
        Stage,
        &'static str,
        ErrorCode,
    );
    let cases: [Expect; 12] = [
        (
            "NEG-04",
            member([0.0; 3], [0.0009, 0.0, 0.0], 200.0, 300.0, None),
            Code::AxisTooShort,
            Stage::Ranges,
            "invalid_input",
            ErrorCode::InvalidArgument,
        ),
        (
            "NEG-05",
            StraightMember {
                end: ok.start,
                ..ok
            },
            Code::AxisTooShort,
            Stage::Ranges,
            "invalid_input",
            ErrorCode::InvalidArgument,
        ),
        (
            "NEG-06",
            StraightMember {
                width_mm: 1_000_000.001,
                ..ok
            },
            Code::AboveLimit,
            Stage::Caps,
            "limit_exceeded",
            ErrorCode::LimitExceeded,
        ),
        (
            "NEG-07",
            StraightMember {
                height_mm: 0.0,
                ..ok
            },
            Code::NotPositive,
            Stage::Ranges,
            "invalid_input",
            ErrorCode::InvalidArgument,
        ),
        (
            "NEG-43",
            StraightMember {
                roll_rad: Some(f64::NAN),
                ..ok
            },
            Code::NonFinite,
            Stage::Finiteness,
            "invalid_input",
            ErrorCode::NonFiniteNumber,
        ),
        (
            "NEG-44",
            StraightMember {
                roll_rad: Some(7.0),
                ..ok
            },
            Code::AboveLimit,
            Stage::Caps,
            "limit_exceeded",
            ErrorCode::LimitExceeded,
        ),
        (
            "NEG-47",
            member(
                [0.0; 3],
                [0.0771484375, 0.0, 0.0],
                393_216.0,
                524_288.0,
                None,
            ),
            Code::AxisTooShort,
            Stage::Ranges,
            "invalid_input",
            ErrorCode::InvalidArgument,
        ),
        (
            "NEG-48",
            member([0.0; 3], [524_288.0, 0.0, 0.0], 0.1240234375, 300.0, None),
            Code::BelowMinimum,
            Stage::Ranges,
            "invalid_input",
            ErrorCode::InvalidArgument,
        ),
        (
            "NEG-51",
            member([0.0; 3], [1_000_000.5, 0.0, 0.0], 200.0, 300.0, None),
            Code::AboveLimit,
            Stage::Caps,
            "limit_exceeded",
            ErrorCode::LimitExceeded,
        ),
        (
            "NEG-52",
            member([0.0; 3], [1000.0, 0.0, 0.0], -3_000_000.0, 300.0, None),
            Code::NotPositive,
            Stage::Ranges,
            "invalid_input",
            ErrorCode::InvalidArgument,
        ),
        (
            "NEG-53",
            member([0.0; 3], [1000.0, 0.0, 0.0], -1e308, 300.0, None),
            Code::NotPositive,
            Stage::Ranges,
            "invalid_input",
            ErrorCode::InvalidArgument,
        ),
        (
            "NEG-01 member",
            StraightMember {
                width_mm: f64::INFINITY,
                ..ok
            },
            Code::NonFinite,
            Stage::Finiteness,
            "invalid_input",
            ErrorCode::NonFiniteNumber,
        ),
    ];
    for (name, input, code, stage, status, f04) in cases {
        let failure = straight_member(input).unwrap_err();
        let d = failure.diagnostics().next().unwrap();
        assert_eq!(
            (d.code, d.stage, failure.status(), d.code.f04_code()),
            (code, stage, status, f04),
            "{name}"
        );
        assert_eq!(failure.diagnostics().count(), 1, "{name}: one diagnostic");
    }
}

// ------------------------------------------------------------- hostile scalar sweeps

const SPECIAL: [f64; 23] = [
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    0.0,
    -0.0,
    5e-324,
    -5e-324,
    f64::MIN_POSITIVE,
    1e-300,
    1e-9,
    0.001,
    0.000_999_99,
    1.0,
    1e6,
    1_000_001.0,
    1e9,
    1_000_000_001.0,
    4e9,
    4_000_000_001.0,
    1e300,
    f64::MAX,
    f64::MIN,
    -1.0,
];

fn plate_ok(r: &PlateResult) {
    let v = assert_sealed_box(r);
    assert!(common::close(v, r.volume_mm3(), 1e-6), "volume agrees");
    let again = r.mesh().blob();
    assert_eq!(again.len(), 392);
}

/// Every plate dimension triple and every one-at-a-time or pairwise member scalar over the
/// special values: never a panic, a non-finite input is never accepted, and every success is a
/// valid closed outward box that the independent reader accepts.
#[test]
fn plate_and_member_builders_survive_every_special_scalar() {
    let mut accepted = 0_usize;
    for &w in &SPECIAL {
        for &l in &SPECIAL {
            for &t in &SPECIAL {
                let result = rectangular_plate(RectangularPlate {
                    width_mm: w,
                    length_mm: l,
                    thickness_mm: t,
                });
                if ![w, l, t].iter().all(|x| x.is_finite()) {
                    let failure = result.unwrap_err();
                    assert_eq!(failure.diagnostics().next().unwrap().code, Code::NonFinite);
                } else if let Ok(r) = result {
                    accepted += 1;
                    assert!(w > 0.0 && l > 0.0 && t > 0.0, "{w} {l} {t}");
                    plate_ok(&r);
                }
            }
        }
    }
    assert!(accepted >= 10, "{accepted} valid plates in the sweep");

    // Member: a valid base, two scalars replaced at a time.
    let base = [0.0, 0.0, 0.0, 100.0, 200.0, 3000.0, 200.0, 300.0, 0.5];
    let build = |s: &[f64; 9]| {
        straight_member(member(
            [s[0], s[1], s[2]],
            [s[3], s[4], s[5]],
            s[6],
            s[7],
            Some(s[8]),
        ))
    };
    let mut member_ok = 0_usize;
    for a in 0..9 {
        for b in a..9 {
            for &x in &SPECIAL {
                for &y in &SPECIAL {
                    let mut s = base;
                    s[a] = x;
                    s[b] = y;
                    let result = build(&s);
                    if s.iter().any(|q| !q.is_finite()) {
                        let failure = result.unwrap_err();
                        assert_eq!(failure.diagnostics().next().unwrap().code, Code::NonFinite);
                    } else if let Ok(r) = result {
                        member_ok += 1;
                        plate_ok(&r);
                    }
                }
            }
        }
    }
    assert!(member_ok > 100, "{member_ok} valid members in the sweep");
}

/// Plane cuts and through holes over the special values, one input component at a time.
/// Non-finite values are never accepted; every success passes the full independent re-read.
#[test]
fn cut_hole_and_profile_operations_survive_every_special_scalar() {
    let plate = common_ops::plate(100.0, 50.0, 10.0);
    let base_point = [30.0, 20.0, 5.0];
    let base_normal = [1.0, 0.3, 0.2];
    let mut cuts_ok = 0_usize;
    for slot in 0..6 {
        for &x in &SPECIAL {
            let mut point = base_point;
            let mut normal = base_normal;
            if slot < 3 {
                point[slot] = x;
            } else {
                normal[slot - 3] = x;
            }
            for keep in [Keep::Positive, Keep::Negative] {
                let result = plane_cut(&plate, &CutPlane::new(point, normal), keep, 0);
                if !x.is_finite() {
                    assert!(
                        result.is_err(),
                        "non-finite plane accepted: {point:?} {normal:?}"
                    );
                } else if let Ok(r) = result {
                    cuts_ok += 1;
                    assert_sealed(&r);
                }
            }
        }
    }
    assert!(cuts_ok >= 2, "{cuts_ok} cuts sealed");

    let spec = CurveSpec {
        max_sagitta_mm: 1.0,
        min_segments: 16,
        max_segments: 16,
    };
    let mut holes_ok = 0_usize;
    for &z0 in &SPECIAL {
        for &z1 in &SPECIAL {
            let cavity = Cavity::circle(Point2::new(50.0, 25.0), 5.0, spec).unwrap();
            let result = through_hole(&plate, &HoleCutter::new(cavity, z0, z1, 0));
            if !(z0.is_finite() && z1.is_finite()) {
                assert!(result.is_err(), "non-finite cutter range accepted");
            } else if let Ok(r) = result {
                holes_ok += 1;
                assert_sealed(&r);
                assert!(z0 < 0.0 && z1 > 10.0);
            }
        }
    }
    assert!(holes_ok >= 1, "{holes_ok} holes sealed");

    // Cavity generators and the extrusion: every (centre, radius, sagitta, thickness) value.
    let outer = vec![
        Point2::new(0.0, 0.0),
        Point2::new(100.0, 0.0),
        Point2::new(100.0, 50.0),
        Point2::new(0.0, 50.0),
    ];
    let mut extruded = 0_usize;
    for slot in 0..5 {
        for &x in &SPECIAL {
            let mut v = [50.0, 25.0, 5.0, 1.0, 10.0];
            v[slot] = x;
            let curve = CurveSpec {
                max_sagitta_mm: v[3],
                min_segments: 16,
                max_segments: 64,
            };
            let Ok(cavity) = Cavity::circle(Point2::new(v[0], v[1]), v[2], curve) else {
                // A non-finite, non-positive or over-cap request is a typed refusal.
                continue;
            };
            assert!(
                [v[0], v[1], v[2], v[3]].iter().all(|q| q.is_finite()),
                "a non-finite circle request was accepted: {v:?}"
            );
            match extrude(&outer, std::slice::from_ref(&cavity), v[4]) {
                Ok(ex) => {
                    extruded += 1;
                    assert_valid_extrusion(&ex, &outer, std::slice::from_ref(&cavity), v[4]);
                }
                Err(_) => {
                    // Outside the plate, too large, thickness out of range: a typed refusal.
                }
            }
            if !x.is_finite() {
                assert!(extrude(&outer, std::slice::from_ref(&cavity), v[4]).is_err());
            }
        }
    }
    assert!(extruded >= 3, "{extruded} extrusions sealed");
}

// ------------------------------------------------------------- never an exact status

fn tri_buf(v: Vec<[f64; 3]>, t: Vec<[u32; 3]>) -> Buf {
    Buf { v, t }
}

/// Closed-looking but unwelded, T-junctioned, stray-triangle, duplicated and folded meshes get
/// a defect status and no engineering value (F06 section 8; F06-NEG-18, NEG-19).
#[test]
fn unwelded_junctioned_stray_duplicated_and_folded_meshes_never_get_an_exact_status() {
    let cube = cuboid([0.0; 3], [10.0; 3]);

    // (a) Every face has private corners: no shared vertices, so no shared edges. The kernel
    // does not weld by position, so this is open, not a cube.
    let mut v = Vec::new();
    let mut t = Vec::new();
    for tri in &cube.t {
        let base = v.len() as u32;
        for &i in tri {
            v.push(cube.v[i as usize]);
        }
        t.push([base, base + 1, base + 2]);
    }
    let unwelded = tri_buf(v, t);

    // (b) A closed cube plus a separate single triangle.
    let mut stray = cube.clone();
    let n = stray.v.len() as u32;
    stray
        .v
        .extend([[20.0, 0.0, 0.0], [30.0, 0.0, 0.0], [20.0, 10.0, 0.0]]);
    stray.t.push([n, n + 1, n + 2]);

    // (c) T-junction: one triangle of face x0 is split at an edge midpoint that the neighbouring
    // face does not share.
    let mut junction = cube.clone();
    let mid = junction.v.len() as u32;
    junction.v.push([0.0, 0.0, 5.0]);
    let split = junction.t.remove(0); // [0, 4, 6]
    junction.t.push([split[0], mid, split[2]]);
    junction.t.push([mid, split[1], split[2]]);

    // (d) The last triangle replaced by a copy of another one.
    let mut duplicated = cube.clone();
    let copy = duplicated.t[10];
    duplicated.t[11] = copy;

    // (e) Closed, consistently oriented, positive volume, but the top corner is pulled through
    // the bottom face: a self-intersecting shell.
    let mut folded = cube.clone();
    folded.v[7] = [10.0, 10.0, -5.0];

    let expected: [(&str, &Buf, &[&str]); 5] = [
        ("unwelded cube", &unwelded, &["Open", "NonManifold"]),
        ("stray triangle", &stray, &["Open"]),
        ("T-junction", &junction, &["Open"]),
        ("duplicated triangle", &duplicated, &["NonManifold", "Open"]),
        ("folded shell", &folded, &["Overlapping"]),
    ];
    for (name, buf, statuses) in expected {
        let mesh = TriMesh::from_buffers(&buf.v, &buf.t).unwrap();
        let analysis = analyze(&mesh).unwrap();
        let got = format!("{:?}", analysis.status());
        assert!(!analysis.is_certified(), "{name}: certified");
        assert!(
            statuses.contains(&got.as_str()),
            "{name}: unexpected status {got}"
        );
        assert!(analysis.status().code().is_some(), "{name}: a typed code");
        let m = measure(&mesh, &analysis).unwrap();
        assert!(
            m.volume_mm3().is_none() && m.area_mm2().is_none() && m.centroid_mm().is_none(),
            "{name}: a value for an uncertified mesh"
        );
        assert_eq!(m.quantity_status(), QuantityStatus::Unsupported, "{name}");
        // The body gate refuses it with a mesh code, never a partial body.
        let blob = common_mesh::blob(&buf.v, &buf.t, &vec![0; buf.t.len()], 1, 0);
        let parsed = TriMesh::from_blob(&blob).unwrap();
        let refused = Body::from_mesh(parsed, vec!["x:f.a".to_owned()], vec![None], None);
        let failure = refused.err().unwrap();
        assert!(
            matches!(
                failure.code,
                OpCode::MeshNotClosed | OpCode::MeshNotCertified
            ),
            "{name}: {:?}",
            failure.code
        );
    }
    // The pristine cube is certified by the same pipeline: the negative controls are real.
    let mesh = TriMesh::from_buffers(&cube.v, &cube.t).unwrap();
    assert!(analyze(&mesh).unwrap().is_certified());
    assert_eq!(MeshCode::NotClosed.as_str(), "GEO_MESH_NOT_CLOSED");
}

// ------------------------------------------------------------- near tolerance

/// A plane that shaves a corner closer than `eps_geom` would create edges below the floor. The
/// pilot must refuse it with a typed failure and no result; a shave clearly above the floor
/// seals and is verified. (Near-tolerance behaviour of the cut.)
#[test]
fn a_plane_cut_leaving_edges_below_the_floor_is_refused_and_one_above_is_sealed() {
    let body = common_ops::cuboid([0.0; 3], [100.0; 3]);
    let eps = body.tolerance().eps_geom_mm();
    assert!((0.001..0.0011).contains(&eps), "eps_geom {eps}");
    // Corner shave x + y + z = d: the three new edges are d*sqrt(2) long.
    let shave = |d: f64| {
        plane_cut(
            &body,
            &CutPlane::new([d, 0.0, 0.0], [1.0, 1.0, 1.0]),
            Keep::Negative,
            0,
        )
    };
    let tiny = shave(0.5 * eps / 2.0_f64.sqrt());
    // A sliver corner is refused.
    let failure = tiny.unwrap_err();
    assert_eq!(failure.code, OpCode::OpUnsupported);
    assert_eq!(failure.field, OpField::Section);
    assert_eq!(failure.defect, OpDefect::SectionNotAdmissible);
    let fine = shave(10.0 * eps / 2.0_f64.sqrt());
    // A shave clearly above the floor seals.
    let r = fine.unwrap();
    assert_sealed(&r);
}

// ------------------------------------------------------------- pinned digests

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).unwrap().to_string()
}

/// Pinned SHA-256 digests of sealed blobs built only from `+ - * /`, `sqrt` and the kernel's own
/// integer-reduced series: they must be identical in every run, on every target and under every
/// build profile. They were measured on x86_64 Windows with Rust 1.96.0 by
/// `cargo run --release -p beamcraft-kernel --example g07_bench`. A mismatch is either an
/// intended algorithm change (bump `algorithm`, review, repin) or a determinism defect.
/// Roll about a non-zero angle uses the platform `sin`/`cos` and is deliberately not pinned.
#[test]
fn sealed_blobs_have_pinned_digests() {
    let plate = rectangular_plate(RectangularPlate {
        width_mm: 1000.0,
        length_mm: 500.0,
        thickness_mm: 10.0,
    })
    .unwrap();
    // The F06-POS-02 vector (also pinned in rectangular_plate.rs).
    assert_eq!(
        plate.mesh().mesh_digest().to_string(),
        "dce6476e9f58de6bfd38bef48e169972e4ddf22264b806f12f933b557a30ed86"
    );
    let diagonal =
        straight_member(member([0.0; 3], [3000.0, 4000.0, 0.0], 100.0, 300.0, None)).unwrap();
    assert_eq!(
        diagonal.mesh().mesh_digest().to_string(),
        "418c0f5cc131873717dd6c1da6e233774a821b4358b813dc520950ded96edcc6"
    );

    let outer = vec![
        Point2::new(0.0, 0.0),
        Point2::new(1000.0, 0.0),
        Point2::new(1000.0, 500.0),
        Point2::new(0.0, 500.0),
    ];
    let spec = |n: u32| CurveSpec {
        max_sagitta_mm: 10.0,
        min_segments: n,
        max_segments: n,
    };
    let cavity = Cavity::circle(Point2::new(40.0, 40.0), 10.0, spec(64)).unwrap();
    let extrusion = extrude(&outer, &[cavity], 10.0).unwrap();
    assert_eq!(
        digest(extrusion.mesh().blob()),
        "c3e8f109895c7a701a040b10eced19d24dcb7c1c289daa08bf4df4f241cde585"
    );

    let body = Body::from_plate(&plate).unwrap();
    let cut = plane_cut(
        &body,
        &CutPlane::new([300.0, 200.0, 5.0], [1.0, 0.4, 0.3]),
        Keep::Positive,
        0,
    )
    .unwrap();
    assert_eq!(
        cut.mesh_digest().to_string(),
        "c4e7eadbd916a99d488df6889ca5bca775496805c5732a7ee4113d519f431239"
    );
    let round = Cavity::circle(Point2::new(500.0, 250.0), 20.0, spec(64)).unwrap();
    let hole = through_hole(&body, &HoleCutter::new(round, -1.0, 11.0, 0)).unwrap();
    assert_eq!(
        hole.mesh_digest().to_string(),
        "d13e69572a12d1326dc0f3679e4b97332d96a91c3a184832542fa95aa642d2a2"
    );
    // The same call again, from a fresh body, gives the same bytes.
    let body2 = Body::from_plate(&plate).unwrap();
    let round2 = Cavity::circle(Point2::new(500.0, 250.0), 20.0, spec(64)).unwrap();
    let hole2 = through_hole(&body2, &HoleCutter::new(round2, -1.0, 11.0, 0)).unwrap();
    assert_eq!(hole.blob(), hole2.blob());
}

/// The mesh status vocabulary used above stays the closed set documented by G06.
#[test]
fn a_certified_cube_reports_the_certified_status_only() {
    let cube = cuboid([0.0; 3], [10.0; 3]);
    let mesh = TriMesh::from_buffers(&cube.v, &cube.t).unwrap();
    assert_eq!(analyze(&mesh).unwrap().status(), ShellStatus::Certified);
}
