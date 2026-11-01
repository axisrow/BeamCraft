// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G09 work bounds, cancellation and seeded hostile-input tests: every budget at its exact
//! value and one below, a cancellation at every poll of both operations, and deterministic
//! fuzzing whose every success is re-verified by the independent oracles.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use crate::common;
use crate::common_mesh;
use crate::common_ops;

use beamcraft_kernel::diagnostic::Stage;
use beamcraft_kernel::ops::{
    Body, CutPlane, HoleCutter, Keep, OpCode, OpDefect, OpFailure, OpField, OpLimits, OpResult,
    plane_cut, plane_cut_with, through_hole, through_hole_with,
};
use beamcraft_kernel::profile::{Cavity, CurveSpec, Point2, extrude};
use beamcraft_kernel::topology::Limits;
use beamcraft_kernel::trimesh::TriMesh;
use common::{close, hole_rect, p, rect, shoelace};
use common_mesh::Rng;
use common_ops::*;

fn circle(cx: f64, cy: f64, r: f64) -> Cavity {
    Cavity::circle(p(cx, cy), r, CurveSpec::DEFAULT).unwrap()
}

fn with(limits: impl Fn(&mut OpLimits)) -> OpLimits {
    let mut l = OpLimits::DEFAULT;
    limits(&mut l);
    l
}

type Run<'a> = dyn Fn(&OpLimits, &mut dyn FnMut(Stage) -> bool) -> Result<OpResult, OpFailure> + 'a;

fn cut_runner<'a>(body: &'a Body, plane: CutPlane, keep: Keep) -> Box<Run<'a>> {
    Box::new(move |limits, cancel| plane_cut_with(body, &plane, keep, 0, limits, cancel))
}

fn hole_runner<'a>(body: &'a Body, cutter: HoleCutter) -> Box<Run<'a>> {
    Box::new(move |limits, cancel| through_hole_with(body, &cutter, limits, cancel))
}

fn never(_: Stage) -> bool {
    false
}

#[test]
fn every_budget_succeeds_at_its_value_and_fails_one_below_without_a_result() {
    let t = 4.0;
    let holed = Body::from_extrusion(
        &extrude(
            &rect(0.0, 0.0, 60.0, 40.0),
            &[circle(12.0, 12.0, 4.0), hole_rect(30.0, 10.0, 40.0, 20.0)],
            t,
        )
        .unwrap(),
    )
    .unwrap();
    let hole = || HoleCutter::new(circle(48.0, 28.0, 5.0), -1.0, 5.0, 0);
    // A horizontal plane through the holed plate: section = outer loop + two holes.
    let plane = CutPlane::new([0.0, 0.0, 1.5], [0.0, 0.0, 1.0]);
    let runs: [(&str, Box<Run>); 2] = [
        ("hole", hole_runner(&holed, hole())),
        ("cut", cut_runner(&holed, plane, Keep::Negative)),
    ];
    for (name, run) in &runs {
        let base = run(&OpLimits::DEFAULT, &mut never).unwrap();
        let s = *base.stats();
        assert!(
            s.triangles_out > 0 && s.loops >= 3 && s.cap_points > 0,
            "{name}"
        );
        // (limit that must pass, limit one below that must fail, defect of the failure)
        let checks: [(OpLimits, OpLimits, OpDefect); 3] = [
            (
                with(|l| l.max_triangles = s.triangles_out),
                with(|l| l.max_triangles = s.triangles_out - 1),
                OpDefect::BudgetTriangles,
            ),
            (
                with(|l| l.max_loops = s.loops),
                with(|l| l.max_loops = s.loops - 1),
                OpDefect::BudgetLoops,
            ),
            (
                with(|l| l.max_cap_points = s.cap_points),
                with(|l| l.max_cap_points = s.cap_points - 1),
                OpDefect::BudgetCapPoints,
            ),
        ];
        for (ok, bad, defect) in checks {
            let same = run(&ok, &mut never).unwrap();
            assert_eq!(same.blob(), base.blob(), "{name} {defect:?}");
            let e = run(&bad, &mut never).unwrap_err();
            assert_eq!((e.code, e.defect), (OpCode::AboveLimit, defect), "{name}");
            assert_eq!(e.status(), "limit_exceeded");
        }
    }
    // The exact-predicate budget: a plane through vertices needs exact signs.
    let cube = cuboid([0.0; 3], [4.0; 3]);
    let through = CutPlane::new([4.0, 0.0, 0.0], [1.0, 1.0, 0.0]);
    let base = plane_cut(&cube, &through, Keep::Negative, 0).unwrap();
    let used = base.stats().exact_predicates;
    assert!(used > 0);
    let limit = |n: u64| with(|l| l.max_exact_predicates = n);
    let ok = plane_cut_with(&cube, &through, Keep::Negative, 0, &limit(used), never).unwrap();
    assert_eq!(ok.blob(), base.blob());
    let e =
        plane_cut_with(&cube, &through, Keep::Negative, 0, &limit(used - 1), never).unwrap_err();
    assert_eq!(
        (e.code, e.field, e.defect),
        (
            OpCode::NumericUncertain,
            OpField::Limits,
            OpDefect::BudgetExact
        )
    );
    let e = plane_cut_with(&cube, &through, Keep::Negative, 0, &limit(0), never).unwrap_err();
    assert_eq!(e.defect, OpDefect::BudgetExact);
    // A budget of zero is fine when no sign needs exact evaluation.
    let generic = CutPlane::new([1.3, 0.0, 0.0], [1.0, 0.7, 0.2]);
    let r = plane_cut_with(&cube, &generic, Keep::Negative, 0, &limit(0), never).unwrap();
    assert_eq!(r.stats().exact_predicates, 0);
    // The certification budget (G06): no pair may be examined, so nothing can be certified.
    let starved = with(|l| {
        l.analysis = Limits {
            max_candidate_pairs: 0,
            ..Limits::DEFAULT
        }
    });
    let e = plane_cut_with(&cube, &generic, Keep::Negative, 0, &starved, never).unwrap_err();
    assert_eq!(
        (e.code, e.defect),
        (OpCode::NumericUncertain, OpDefect::ResultUncertain)
    );
}

#[test]
fn a_body_beyond_the_certification_budget_is_refused_at_admission() {
    // 16 holes of 120 edges: 12 + 484 * 16 = 7756 triangles, below the F06 cap of 524,288, but
    // the long cap triangles overlap so many boxes that the default G06 pair budget cannot
    // certify the shell. It is refused as not certified; it is never treated as exact.
    let spec = CurveSpec {
        max_sagitta_mm: 1.0e9,
        min_segments: 120,
        max_segments: 120,
    };
    let cavities: Vec<Cavity> = (0..16)
        .map(|i| {
            let (col, row) = (f64::from(i % 4), f64::from(i / 4));
            Cavity::circle(p(25.0 + col * 50.0, 25.0 + row * 50.0), 10.0, spec).unwrap()
        })
        .collect();
    let ex = extrude(&rect(0.0, 0.0, 200.0, 200.0), &cavities, 5.0).unwrap();
    assert_eq!(ex.mesh().triangles().len(), 12 + 484 * 16);
    let e = Body::from_extrusion(&ex).unwrap_err();
    assert_eq!((e.code, e.field), (OpCode::MeshNotCertified, OpField::Body));
    assert_eq!(e.status(), "unsupported");
}
#[test]
fn cancellation_at_every_poll_discards_the_candidate_and_nothing_polls_after_sealing() {
    let t = 4.0;
    let plate_body = plate(60.0, 40.0, t);
    let cut_body = cuboid([0.0; 3], [6.0, 5.0, 4.0]);
    let runs: [(&str, Box<Run>); 3] = [
        (
            "hole",
            hole_runner(
                &plate_body,
                HoleCutter::new(circle(30.0, 20.0, 6.0), -1.0, 5.0, 0),
            ),
        ),
        (
            "cut",
            cut_runner(
                &cut_body,
                CutPlane::new([1.0, 1.0, 1.0], [1.0, 2.0, 3.0]),
                Keep::Negative,
            ),
        ),
        (
            "cut-through-vertices",
            cut_runner(
                &cut_body,
                CutPlane::new([3.0, 0.0, 0.0], [1.0, 1.0, 0.0]),
                Keep::Positive,
            ),
        ),
    ];
    for (name, run) in &runs {
        let mut stages: Vec<Stage> = Vec::new();
        let reference = run(&OpLimits::DEFAULT, &mut |s| {
            stages.push(s);
            false
        })
        .unwrap();
        assert!(stages.len() >= 8, "{name}: {stages:?}");
        assert_eq!(stages[0], Stage::Validation, "{name}");
        assert_eq!(stages[1], Stage::Build, "{name}");
        assert_eq!(*stages.last().unwrap(), Stage::Quantities, "{name}");
        assert!(stages.contains(&Stage::Checks));
        for at in 0..stages.len() {
            let mut seen = 0;
            let result = run(&OpLimits::DEFAULT, &mut |s| {
                assert_eq!(s, stages[seen], "{name}: poll order is deterministic");
                seen += 1;
                seen == at + 1
            });
            let e = result.unwrap_err();
            assert_eq!(e.code, OpCode::Cancelled, "{name} at {at}");
            assert_eq!(e.status(), "cancelled");
            assert_eq!(seen, at + 1, "{name}: no poll after the cancelled one");
        }
        // Never cancelling reproduces the reference bytes: cancellation left no state behind.
        let again = run(&OpLimits::DEFAULT, &mut never).unwrap();
        assert_eq!(again.blob(), reference.blob());
    }
}

// ---- seeded hostile fuzzing ----

fn pick(rng: &mut Rng, items: &[f64]) -> f64 {
    items[rng.below(items.len() as u64) as usize]
}

fn unit(rng: &mut Rng) -> f64 {
    (rng.next() >> 11) as f64 / (1u64 << 53) as f64
}

fn random_bodies(rng: &mut Rng) -> Vec<Body> {
    let mut bodies = vec![
        cuboid([0.0; 3], [4.0, 6.0, 5.0]),
        cuboid([-3.5, 10.0, 2.25], [9.0, 14.5, 20.0]),
        plate(23.0, 47.0, 3.0),
        Body::from_extrusion(
            &extrude(
                &[
                    p(0.0, 0.0),
                    p(23.0, 0.0),
                    p(23.0, 20.0),
                    p(10.0, 20.0),
                    p(10.0, 47.0),
                    p(0.0, 47.0),
                ],
                &[],
                3.0,
            )
            .unwrap(),
        )
        .unwrap(),
        Body::from_extrusion(
            &extrude(&rect(0.0, 0.0, 40.0, 30.0), &[circle(20.0, 15.0, 5.0)], 8.0).unwrap(),
        )
        .unwrap(),
    ];
    // A convex random polygon prism with a hole.
    let n = 5 + rng.below(6) as usize;
    let pts: Vec<Point2> = (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            p(50.0 + 30.0 * a.cos(), 50.0 + 20.0 * a.sin())
        })
        .collect();
    if let Ok(ex) = extrude(&pts, &[circle(50.0, 50.0, 6.0)], 7.0) {
        bodies.push(Body::from_extrusion(&ex).unwrap());
    }
    bodies
}

fn random_plane(rng: &mut Rng, body: &Body) -> CutPlane {
    let v = body.mesh().vertices();
    let vertex = |rng: &mut Rng| v[rng.below(v.len() as u64) as usize];
    let special = [0.0, 0.0, 1.0, -1.0, 0.5, 1e-300, 5e-324, 1e9, 3.0, -7.25];
    let mut normal = [
        pick(rng, &special),
        pick(rng, &special),
        pick(rng, &special),
    ];
    if rng.chance(1, 4) {
        normal = [unit(rng) - 0.5, unit(rng) - 0.5, unit(rng) - 0.5];
    }
    let point = match rng.below(5) {
        0 => vertex(rng),
        1 => {
            // The midpoint of two vertices (often on an edge or a face).
            let (a, b) = (vertex(rng), vertex(rng));
            [
                (a[0] + b[0]) / 2.0,
                (a[1] + b[1]) / 2.0,
                (a[2] + b[2]) / 2.0,
            ]
        }
        2 => {
            // A plane through three vertices: the normal is their cross product.
            let (a, b, c) = (vertex(rng), vertex(rng), vertex(rng));
            let (u, w) = (
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            normal = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            a
        }
        3 => [
            unit(rng) * 60.0 - 5.0,
            unit(rng) * 60.0 - 5.0,
            unit(rng) * 20.0 - 2.0,
        ],
        _ => [
            pick(rng, &special),
            pick(rng, &special),
            pick(rng, &special),
        ],
    };
    CutPlane::new(point, normal)
}

#[test]
fn seeded_hostile_planes_never_panic_and_every_success_is_verified() {
    let mut rng = Rng(0x00c0_ffee_dead_beef);
    let (mut ok, mut refused) = (0usize, 0usize);
    let mut halves = 0usize;
    let mut codes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let started = std::time::Instant::now();
    for round in 0..2 {
        for body in random_bodies(&mut rng) {
            let total = body.volume_mm3();
            for _ in 0..8 {
                let plane = random_plane(&mut rng, &body);
                let mut volumes = Vec::new();
                for keep in [Keep::Negative, Keep::Positive] {
                    match plane_cut(&body, &plane, keep, (round % 50) as u16) {
                        Ok(res) => {
                            ok += 1;
                            let f = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                assert_sealed(&res)
                            }))
                            .unwrap_or_else(|_| {
                                panic!("{plane:?} {keep:?} {:?}", body.face_table())
                            });
                            assert!(f.volume < total.hi && f.volume > 0.0);
                            let again =
                                plane_cut(&body, &plane, keep, (round % 50) as u16).unwrap();
                            assert_eq!(again.blob(), res.blob());
                            assert!(res.plane_deviation_mm().unwrap() <= 1e-6);
                            volumes.push(f.volume);
                        }
                        Err(e) => {
                            refused += 1;
                            codes.insert(format!("{}:{}", e.code.as_str(), e.defect.as_str()));
                            assert!(!e.status().is_empty());
                        }
                    }
                }
                if let [a, b] = volumes[..] {
                    halves += 1;
                    // The halves share their cap vertices: the volumes add up to the body's.
                    assert!(
                        close(a + b, (total.lo + total.hi) / 2.0, 1e-9),
                        "{a} + {b} vs {total:?}"
                    );
                }
            }
        }
    }
    println!(
        "planes in {:?}: ok {ok}, refused {refused}, complete pairs {halves}, codes {codes:?}",
        started.elapsed()
    );
    assert!(
        ok > 40 && halves > 15,
        "the fuzz must reach the success path ({ok}, {halves})"
    );
    assert!(codes.len() >= 5, "and several distinct refusals: {codes:?}");
}

fn random_cavity(rng: &mut Rng) -> Cavity {
    let spec = CurveSpec::DEFAULT;
    // Mostly inside the 40 x 30 plate, sometimes anywhere (touching, crossing or outside).
    let (cx, cy) = if rng.chance(3, 4) {
        (12.0 + unit(rng) * 16.0, 10.0 + unit(rng) * 10.0)
    } else {
        (unit(rng) * 50.0 - 5.0, unit(rng) * 40.0 - 5.0)
    };
    let (a, b) = (1.0 + unit(rng) * 4.0, 1.0 + unit(rng) * 4.0);
    match rng.below(7) {
        0 => Cavity::circle(p(cx, cy), a, spec).unwrap_or_else(|_| hole_rect(0.0, 0.0, 1.0, 1.0)),
        1 => Cavity::slot(p(cx, cy), a + b + 1.0, a.min(b), p(1.0, unit(rng)), spec)
            .unwrap_or_else(|_| hole_rect(0.0, 0.0, 1.0, 1.0)),
        2 => Cavity::ellipse(p(cx, cy), a.max(b), a.min(b), p(unit(rng), 1.0), spec)
            .unwrap_or_else(|_| hole_rect(0.0, 0.0, 1.0, 1.0)),
        3 => hole_rect(cx, cy, cx + a, cy + b),
        4 => {
            // A random star-shaped polygon, clockwise, sometimes with a repeated point.
            let n = 3 + rng.below(9) as usize;
            let mut pts: Vec<Point2> = (0..n)
                .map(|k| {
                    let ang = -std::f64::consts::TAU * k as f64 / n as f64;
                    let r = 1.0 + unit(rng) * 6.0;
                    p(cx + r * ang.cos(), cy + r * ang.sin())
                })
                .collect();
            if rng.chance(1, 6) {
                pts.push(pts[0]);
            }
            Cavity::polygon(pts)
        }
        5 => {
            // Counter-clockwise or self-intersecting garbage.
            Cavity::polygon(vec![
                p(cx, cy),
                p(cx + a, cy),
                p(cx + a, cy + b),
                p(cx, cy + b),
            ])
        }
        _ => Cavity::polygon(vec![
            p(cx, cy),
            p(cx + a, cy + b),
            p(cx + a, cy),
            p(cx, cy + b),
        ]),
    }
}

#[test]
fn seeded_hostile_cutters_never_panic_and_every_success_is_verified() {
    let mut rng = Rng(0x5eed_5eed_0bad_cafe);
    let (mut ok, mut refused) = (0usize, 0usize);
    let mut codes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let (w, l, t) = (40.0, 30.0, 8.0);
    for round in 0..60 {
        let mut body = plate(w, l, t);
        let mut expected = body.volume_mm3();
        let mut genus = 0u32;
        for hole in 0..4u16 {
            let cavity = random_cavity(&mut rng);
            let (from, to) = match rng.below(8) {
                0 => (0.0, t + 1.0),
                1 => (-1.0, t),
                2 => (-0.0005, t + 1.0),
                3 => (3.0, 5.0),
                4 | 5 => (-1.0, t + 1.0),
                _ => (-2.0, t + 2.0),
            };
            let pts: Vec<Point2> = cavity.points().to_vec();
            let cutter = HoleCutter::new(cavity, from, to, hole + (round % 5) as u16 * 10);
            match through_hole(&body, &cutter) {
                Ok(res) => {
                    ok += 1;
                    let f = assert_sealed(&res);
                    genus += 1;
                    assert_eq!(res.genus(), Some(genus));
                    assert_eq!(f.euler, 2 - 2 * i64::from(genus));
                    // Independent area (shoelace of the submitted polygon) times thickness.
                    let removed = -shoelace(&pts) * t;
                    let before = (expected.lo + expected.hi) / 2.0;
                    assert!(
                        close(f.volume, before - removed, 1e-9),
                        "{} {} {removed}",
                        f.volume,
                        before
                    );
                    assert!(
                        res.analytic_volume_mm3()
                            .unwrap()
                            .overlaps(res.volume_mm3())
                    );
                    let again = through_hole(&body, &cutter).unwrap();
                    assert_eq!(again.blob(), res.blob());
                    body = res.into_body();
                    expected = body.volume_mm3();
                }
                Err(e) => {
                    refused += 1;
                    codes.insert(format!("{}:{}", e.code.as_str(), e.defect.as_str()));
                }
            }
        }
    }
    println!("cutters: ok {ok}, refused {refused}, codes {codes:?}");
    assert!(ok > 50 && refused > 60, "{ok} {refused}");
    assert!(codes.len() >= 4, "{codes:?}");
}

#[test]
fn mutated_bodies_are_refused_or_cut_without_panics() {
    let mut rng = Rng(0xfeed_f00d_1234_8765);
    let blob = {
        let cube = common_mesh::cuboid([0.0; 3], [4.0; 3]);
        common_mesh::blob(&cube.v, &cube.t, &CUBOID_FACES, 6, 0)
    };
    let ids: Vec<String> = ["b:f.a", "b:f.b", "b:f.c", "b:f.d", "b:f.e", "b:f.f"]
        .map(str::to_owned)
        .to_vec();
    let (mut admitted, mut rejected) = (0, 0);
    for _ in 0..600 {
        let mut bytes = blob.clone();
        for _ in 0..1 + rng.below(4) {
            let at = 32 + rng.below(bytes.len() as u64 - 32) as usize;
            match rng.below(4) {
                0 => bytes[at] ^= 1 << rng.below(8),
                1 => bytes[at] = rng.next() as u8,
                2 => {
                    // Overwrite a whole vertex coordinate with a special value.
                    let slot = (at - 32) / 8 * 8 + 32;
                    if slot + 8 <= 32 + 24 * 8 {
                        let v = pick(&mut rng, &[0.0, 1.0, 2.0, 4.0, 1e-300, 1e9, -3.5]);
                        bytes[slot..slot + 8].copy_from_slice(&v.to_le_bytes());
                    }
                }
                _ => {
                    let slot = 32 + 24 * 8 + (rng.below(36) as usize) * 4;
                    bytes[slot..slot + 4].copy_from_slice(&(rng.below(8) as u32).to_le_bytes());
                }
            }
        }
        let Ok(mesh) = TriMesh::from_blob(&bytes) else {
            rejected += 1;
            continue;
        };
        match Body::from_mesh(mesh, ids.clone(), vec![None; 6], None) {
            Err(_) => rejected += 1,
            Ok(body) => {
                admitted += 1;
                let plane = random_plane(&mut rng, &body);
                if let Ok(res) = plane_cut(&body, &plane, Keep::Negative, 0) {
                    assert_sealed(&res);
                }
                let cutter = HoleCutter::new(hole_rect(1.0, 1.0, 2.0, 2.0), -1.0, 5.0, 0);
                if let Ok(res) = through_hole(&body, &cutter) {
                    assert_sealed(&res);
                }
            }
        }
    }
    assert!(admitted >= 1 && rejected > 100, "{admitted} {rejected}");
}
