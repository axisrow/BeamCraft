// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Independent references only: expected volumes are closed forms (area times path length,
// the Pappus value of the arc, the annulus formula) computed here, the blob is reread with
// the G03 test reader, and the mitre planes are checked against their own bisectors.

//! G12 sweep fixtures: a section swept along polylines and arcs, mitred at the corners.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod common;

use beamcraft_geom::Point3;
use beamcraft_kernel::section::Representation;
use beamcraft_kernel::sweep::{PathCode, Sweep, SweepFailure, SweepPath, SweepResult, sweep};
use beamcraft_model::section::{CustomSection, ParametricSection, SectionFamily, SectionShape};
use core::f64::consts::PI;

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
