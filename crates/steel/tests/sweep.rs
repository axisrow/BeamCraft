// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Independent references only: expected volumes are closed forms (area times path length,
// the Pappus value of the arc, the annulus formula) computed here, the blob is reread with
// the G03 test reader, and the mitre planes are checked against their own bisectors.

//! G12 sweep fixtures: a section swept along polylines and arcs, mitred at the corners.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::common;

use beamcraft_geom::Point3;
use core::f64::consts::PI;
use steel::kernel::section::Representation;
use steel::kernel::sweep::{PathCode, Sweep, SweepFailure, SweepPath, SweepResult, sweep};
use steel::model::section::{CustomSection, ParametricSection, SectionFamily, SectionShape};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// A centred square section: its centroid sits exactly on the path.
fn square(side: f64) -> SectionShape {
    let h = side / 2.0;
    SectionShape::Custom(CustomSection {
        outer: vec![[-h, -h], [h, -h], [h, h], [-h, h]],
        cavities: Vec::new(),
    })
}

fn chs() -> SectionShape {
    SectionShape::Parametric(ParametricSection::new(SectionFamily::Chs, vec![114.0, 5.0]).unwrap())
}

fn run(path: SweepPath, shape: SectionShape, deviation: f64) -> SweepResult {
    sweep(&Sweep {
        path,
        shape,
        representation: Representation::Detailed,
        chord_deviation_mm: deviation,
    })
    .unwrap()
}

fn volume(result: &SweepResult) -> f64 {
    result.volume_mm3().midpoint()
}

fn close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{actual} vs {expected}"
    );
}

#[test]
fn a_straight_sweep_matches_the_prism_closed_form_and_reads_back() {
    let r = run(
        SweepPath::Polyline(vec![p(0.0, 0.0, 0.0), p(0.0, 0.0, 1000.0)]),
        square(200.0),
        0.1,
    );
    close(volume(&r), 200.0 * 200.0 * 1000.0, 1e-9);
    assert_eq!(r.segment_count(), 1);
    assert!(!r.approximated());
    assert!(r.chord_deviation_bound_mm().is_none());
    assert!(r.volume_mm3().contains(200.0 * 200.0 * 1000.0));
    // Independent read-back: the blob parses, is one closed shell, and its
    // divergence-theorem volume is the prism volume.
    let blob = common::parse_blob(r.blob());
    assert_eq!(blob.vertices.len(), 8);
    assert_eq!(blob.triangles.len(), 12);
    let topology = common::topology(&blob.triangles, blob.vertices.len());
    assert!(topology.closed && topology.components == 1);
    close(
        common::mesh_volume(&blob.vertices, &blob.triangles),
        200.0 * 200.0 * 1000.0,
        1e-9,
    );
}

#[test]
fn mitred_corners_keep_area_times_path_length_exactly() {
    // Two mitres in different planes; every section here is centred on the path.
    let path = SweepPath::Polyline(vec![
        p(0.0, 0.0, 0.0),
        p(3000.0, 0.0, 0.0),
        p(3000.0, 0.0, 4000.0),
        p(3000.0, 5000.0, 4000.0),
    ]);
    let r = run(path, square(200.0), 0.1);
    close(volume(&r), 200.0 * 200.0 * 12000.0, 1e-9);
    assert_eq!(r.segment_count(), 3);
    let blob = common::parse_blob(r.blob());
    close(
        common::mesh_volume(&blob.vertices, &blob.triangles),
        200.0 * 200.0 * 12000.0,
        1e-9,
    );
    // Ring planes: every vertex lies in the start plane (x = 0), one of the two mitre
    // bisector planes (x + z = 3000, y + z = 4000) or the end plane (y = 5000);
    // canonical order is sorted, so vertices are not grouped by ring any more.
    let m = core::f64::consts::FRAC_1_SQRT_2;
    for v in &blob.vertices {
        let planes = [
            v[0].abs(),
            (v[0] + v[2] - 3000.0) * m,
            (v[1] + v[2] - 4000.0) * m,
            (v[1] - 5000.0).abs(),
        ];
        let best = planes.iter().fold(f64::INFINITY, |a, &d| a.min(d.abs()));
        assert!(best < 1e-6, "{v:?}");
    }
}

#[test]
fn a_hollow_section_sweeps_with_a_genus_one_handle() {
    let r = run(
        SweepPath::Polyline(vec![p(0.0, 0.0, 0.0), p(500.0, 0.0, 0.0)]),
        chs(),
        0.1,
    );
    // Exact annulus closed form, within the chord-approximation bounds of both circles
    // (every chord region is thinner than the sagitta bound `max(0.25, 0.005 r)`).
    let (ro, ri) = (57.0_f64, 52.0_f64);
    let bound = |rad: f64| 2.0 * PI * rad * 0.25_f64.max(0.005 * rad);
    let exact = PI * (ro * ro - ri * ri);
    let v = volume(&r);
    assert!(
        v >= (exact - bound(ro) - bound(ri)) * 500.0 && v <= (exact + bound(ri)) * 500.0,
        "{v} vs {exact}"
    );
    assert!(r.approximated());
    // The seal certified the topology it was promised: one shell of genus one.
    let shell = r.body().analysis().shells().first().unwrap();
    assert_eq!(shell.genus, Some(1));
    let blob = common::parse_blob(r.blob());
    let topology = common::topology(&blob.triangles, blob.vertices.len());
    assert!(topology.closed && topology.components == 1);
}

#[test]
fn a_quarter_arc_matches_the_pappus_volume_within_the_stated_deviation() {
    let r2 = 2.0_f64.sqrt();
    let r = run(
        SweepPath::Arc {
            start: p(1000.0, 0.0, 0.0),
            on: p(1000.0 / r2, 1000.0 / r2, 0.0),
            end: p(0.0, 1000.0, 0.0),
        },
        square(200.0),
        0.1,
    );
    // Pappus: the true rotated sweep has volume A * R_bar * theta with R_bar = 1000.
    let (area, radius, theta) = (40000.0_f64, 1000.0_f64, PI / 2.0);
    let true_volume = area * radius * theta;
    let n = f64::from(r.segment_count());
    // The mitred faceting is exact: V_mesh = A * (chord polygon at the centroid radius).
    let chord_volume = area * 2.0 * n * radius * (theta / (2.0 * n)).sin();
    close(volume(&r), chord_volume, 1e-9);
    // Every chord is shorter than its arc, and the deficit is bounded by the third-order
    // chord formula `A * (R*theta - 2nR sin(theta/2n)) <= A * R * theta^3 / (24 n^2)`.
    assert!(volume(&r) < true_volume);
    let deficit = true_volume - chord_volume;
    assert!(
        deficit <= area * radius * theta * theta * theta / (24.0 * n * n),
        "deficit {deficit}"
    );
    // The stated deviation is honoured and reported for the swept surface.
    let bound = r.chord_deviation_bound_mm().unwrap();
    assert!(bound > 0.0 && bound <= 0.1, "{bound}");
    assert!(r.approximated());
}

#[test]
fn a_large_radius_arc_keeps_the_stated_deviation_past_the_per_turn_cap() {
    // A 50 m radius needs about 2240 edges per full turn for 0.05 mm, more than G05's
    // per-turn cap of 1020, yet this 11.5 degree arc needs only about 70 facets. It used to be
    // refused as unreachable; it now sweeps at the same bound.
    let radius = 50_000.0_f64;
    let half = 0.1_f64;
    let r = run(
        SweepPath::Arc {
            start: p(radius * (-half).cos(), radius * (-half).sin(), 0.0),
            on: p(radius, 0.0, 0.0),
            end: p(radius * half.cos(), radius * half.sin(), 0.0),
        },
        square(200.0),
        0.05,
    );
    let bound = r.chord_deviation_bound_mm().unwrap();
    assert!(bound > 0.0 && bound <= 0.05, "{bound}");
    let theta = 2.0 * half;
    let n = f64::from(r.segment_count());
    // The facets are the fewest the bound allows at the outermost fibre radius
    // R + 2 * r_s (r_s = 100 * sqrt(2) for the square), within one multiple-of-four step
    // of the per-turn count.
    let outer = radius + 2.0 * 100.0 * 2.0_f64.sqrt();
    let least = (theta / (8.0 * 0.05 / outer).sqrt()).ceil();
    assert!(n >= least && n <= least + 1.0, "{n} facets, least {least}");
    let area = 40000.0_f64;
    close(
        volume(&r),
        area * 2.0 * n * radius * (theta / (2.0 * n)).sin(),
        1e-9,
    );
}

#[test]
fn an_arc_within_the_per_turn_cap_keeps_its_facets() {
    // The quarter arc of radius 1000 at 0.1 mm: G05 picks the per-turn count as before
    // (the least multiple of four with r (TAU/N)^2 / 8 <= 0.1 at r = 1000 + 200 sqrt(2)),
    // so existing arcs keep their meshes.
    let r2 = 2.0_f64.sqrt();
    let r = run(
        SweepPath::Arc {
            start: p(1000.0, 0.0, 0.0),
            on: p(1000.0 / r2, 1000.0 / r2, 0.0),
            end: p(0.0, 1000.0, 0.0),
        },
        square(200.0),
        0.1,
    );
    let outer = 1000.0 + 200.0 * r2;
    let mut per_turn = 8.0_f64;
    while outer * (2.0 * PI / per_turn).powi(2) / 8.0 > 0.1 {
        per_turn += 4.0;
    }
    let expected = (PI / 2.0 / (2.0 * PI / per_turn)).ceil();
    assert_eq!(f64::from(r.segment_count()), expected);
}

#[test]
fn polyline_sweep_bytes_are_reproducible() {
    let path = SweepPath::Polyline(vec![
        p(0.0, 0.0, 0.0),
        p(300.0, 0.0, 0.0),
        p(300.0, 0.0, 400.0),
    ]);
    let a = run(path.clone(), square(50.0), 0.1);
    let b = run(path, square(50.0), 0.1);
    assert_eq!(a.blob(), b.blob());
    assert_eq!(volume(&a), volume(&b));
}

#[test]
fn bad_paths_are_refused_with_typed_codes() {
    let cases: Vec<(SweepPath, PathCode)> = vec![
        (
            SweepPath::Polyline(vec![p(0.0, 0.0, 0.0)]),
            PathCode::TooFewPoints,
        ),
        (
            SweepPath::Polyline(vec![p(0.0, 0.0, 0.0), p(0.0, 0.0, 0.0), p(100.0, 0.0, 0.0)]),
            PathCode::ZeroLengthSegment,
        ),
        (
            SweepPath::Polyline(vec![p(0.0, 0.0, 0.0), p(100.0, 0.0, 0.0), p(0.0, 0.0, 0.0)]),
            PathCode::DegenerateTurn,
        ),
        (
            SweepPath::Polyline(vec![p(0.0, 0.0, 0.0), p(f64::NAN, 0.0, 0.0)]),
            PathCode::NonFinite,
        ),
        (
            SweepPath::Polyline(vec![p(1.1e9, 0.0, 0.0), p(0.0, 0.0, 0.0)]),
            PathCode::AboveLimit,
        ),
        (
            SweepPath::Arc {
                start: p(0.0, 0.0, 0.0),
                on: p(100.0, 0.0, 0.0),
                end: p(200.0, 0.0, 0.0),
            },
            PathCode::CollinearArc,
        ),
    ];
    for (path, code) in cases {
        let failure = sweep(&Sweep {
            path,
            shape: square(200.0),
            representation: Representation::Detailed,
            chord_deviation_mm: 0.1,
        })
        .unwrap_err();
        assert_eq!(failure, SweepFailure::Path(code), "{code:?}");
    }
    // A nonpositive chord deviation.
    let deviation = sweep(&Sweep {
        path: SweepPath::Polyline(vec![p(0.0, 0.0, 0.0), p(100.0, 0.0, 0.0)]),
        shape: square(200.0),
        representation: Representation::Detailed,
        chord_deviation_mm: 0.0,
    })
    .unwrap_err();
    assert_eq!(deviation, SweepFailure::Path(PathCode::BadDeviation));
    // A deviation unreachable inside the G05 segment cap (a kilometre-radius arc at a
    // nanometre deviation).
    let r2 = 2.0_f64.sqrt();
    let unreachable = sweep(&Sweep {
        path: SweepPath::Arc {
            start: p(1.0e6, 0.0, 0.0),
            on: p(1.0e6 / r2, 1.0e6 / r2, 0.0),
            end: p(0.0, 1.0e6, 0.0),
        },
        shape: square(200.0),
        representation: Representation::Detailed,
        chord_deviation_mm: 1.0e-9,
    })
    .unwrap_err();
    assert_eq!(
        unreachable,
        SweepFailure::Path(PathCode::DeviationUnreachable)
    );
    // More rings than the mesh budget (1500 chords times 4 section points).
    let long: Vec<Point3> = (0..1500).map(|i| p(f64::from(i), 0.0, 0.0)).collect();
    let too_many = sweep(&Sweep {
        path: SweepPath::Polyline(long),
        shape: square(200.0),
        representation: Representation::Detailed,
        chord_deviation_mm: 0.1,
    })
    .unwrap_err();
    assert_eq!(too_many, SweepFailure::Path(PathCode::TooManyVertices));
}

#[test]
fn a_closing_arc_point_is_refused_as_coincident() {
    // start == end leaves the end chord zero: the three-point arc form cannot express a
    // full circle, and the refusal is typed, not a repair.
    let failure = sweep(&Sweep {
        path: SweepPath::Arc {
            start: p(1000.0, 0.0, 0.0),
            on: p(-1000.0, 0.0, 0.0),
            end: p(1000.0, 0.0, 0.0),
        },
        shape: square(200.0),
        representation: Representation::Detailed,
        chord_deviation_mm: 0.1,
    })
    .unwrap_err();
    assert_eq!(failure, SweepFailure::Path(PathCode::CollinearArc));
}

/// The circumcentre of three in-plane (z = 0) points, solved independently of the kernel.
fn circumcentre(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> ([f64; 2], f64) {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    let sq = |p: [f64; 2]| p[0] * p[0] + p[1] * p[1];
    let ux = (sq(a) * (b[1] - c[1]) + sq(b) * (c[1] - a[1]) + sq(c) * (a[1] - b[1])) / d;
    let uy = (sq(a) * (c[0] - b[0]) + sq(b) * (a[0] - c[0]) + sq(c) * (b[0] - a[0])) / d;
    let r = ((a[0] - ux).powi(2) + (a[1] - uy).powi(2)).sqrt();
    ([ux, uy], r)
}

/// Issue 542: the faceted arc must lie on the authored circle and run from the authored
/// start through the authored on-point to the authored end, not on a mirror image of it.
/// A volume check alone is blind to a reflected arc, so every vertex is checked here.
#[test]
fn an_arc_mesh_lies_on_its_authored_circle_between_its_authored_ends() {
    let half = 50.0_f64;
    let band = half * 2.0_f64.sqrt() + 0.2;
    let cases: [[[f64; 2]; 3]; 4] = [
        // The shallow arc of the issue, far from the origin.
        [[0.0, 40000.0], [1500.0, 40300.0], [3000.0, 40000.0]],
        // The same arc bulging the other way.
        [[0.0, 40000.0], [1500.0, 39700.0], [3000.0, 40000.0]],
        // An asymmetric on-point.
        [[0.0, 0.0], [400.0, 600.0], [2000.0, 300.0]],
        // A three-quarter arc: the travel passes through `on` the long way round.
        [[1000.0, 0.0], [0.0, 1000.0], [0.0, -1000.0]],
    ];
    for [a, b, c] in cases {
        let r = run(
            SweepPath::Arc {
                start: p(a[0], a[1], 0.0),
                on: p(b[0], b[1], 0.0),
                end: p(c[0], c[1], 0.0),
            },
            square(2.0 * half),
            0.1,
        );
        let blob = common::parse_blob(r.blob());
        let (centre, radius) = circumcentre(a, b, c);
        for v in &blob.vertices {
            let d = ((v[0] - centre[0]).powi(2) + (v[1] - centre[1]).powi(2)).sqrt();
            assert!(
                (d - radius).abs() <= band,
                "{a:?}/{b:?}/{c:?}: vertex {v:?} is {d} from the centre, radius {radius}"
            );
        }
        // The authored ends and the on-point each lie within a section of the mesh.
        for q in [a, b, c] {
            let nearest = blob
                .vertices
                .iter()
                .map(|v| ((v[0] - q[0]).powi(2) + (v[1] - q[1]).powi(2)).sqrt())
                .fold(f64::INFINITY, f64::min);
            assert!(
                nearest <= band,
                "{a:?}/{b:?}/{c:?}: {q:?} is {nearest} off the mesh"
            );
        }
    }
}
