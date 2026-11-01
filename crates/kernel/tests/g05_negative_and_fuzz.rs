// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G05 negative, cancellation, mapping and hostile-input tests.
//!
//! Every rejection returns a typed failure with no geometry; the fuzz tests use a fixed-seed
//! generator so a failure is reproducible, and passing proves no panic and no hang.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::common;

use beamcraft_kernel::diagnostic::Stage;
use beamcraft_kernel::profile::diagnostic::{Defect, ProfileCode, ProfileFailure, ProfileField};
use beamcraft_kernel::profile::{
    Cavity, CurveSpec, Point2, Profile, extrude, extrude_profile, extrude_with_cancel,
};
use beamcraft_model::error::ErrorCode;
use common::*;
use std::cell::RefCell;

fn base_outer() -> Vec<Point2> {
    rect(0.0, 0.0, 100.0, 100.0)
}

fn base_hole() -> Cavity {
    hole_rect(10.0, 10.0, 20.0, 20.0)
}

fn reject(outer: &[Point2], cavities: &[Cavity], t: f64) -> ProfileFailure {
    extrude(outer, cavities, t).unwrap_err()
}

fn first(f: &ProfileFailure) -> (ProfileCode, Stage, Defect) {
    let d = f.diagnostics()[0];
    (d.code, d.stage, d.defect)
}

fn has(f: &ProfileFailure, defect: Defect) -> bool {
    f.diagnostics().iter().any(|d| d.defect == defect)
}

fn ccw(mut v: Vec<Point2>) -> Vec<Point2> {
    if shoelace(&v) < 0.0 {
        v.reverse();
    }
    v
}

#[test]
fn valid_baseline_and_caps_are_inclusive() {
    let (outer, hole) = (base_outer(), [base_hole()]);
    assert_valid_extrusion(&extrude(&outer, &hole, 5.0).unwrap(), &outer, &hole, 5.0);
    // Thickness exactly at the 1,000,000 mm cap is accepted; one step above is not.
    assert!(extrude(&outer, &hole, 1_000_000.0).is_ok());
    let above = f64::from_bits(1_000_000.0f64.to_bits() + 1);
    assert_eq!(
        first(&reject(&outer, &hole, above)),
        (ProfileCode::AboveLimit, Stage::Caps, Defect::None)
    );
    // A coordinate exactly at 1e9 is accepted; one step above is not.
    let at_cap = rect(1.0e9 - 100.0, 1.0e9 - 100.0, 1.0e9, 1.0e9);
    assert!(extrude(&at_cap, &[], 1.0).is_ok());
    let over = rect(
        1.0e9 - 100.0,
        1.0e9 - 100.0,
        1.0e9,
        f64::from_bits(1.0e9f64.to_bits() + 1),
    );
    assert_eq!(first(&reject(&over, &[], 1.0)).0, ProfileCode::AboveLimit);
    // D = 2,000,000 exactly is accepted (every edge is at most 1e6); D above it is a limit.
    let at_d_cap = vec![
        p(0.0, 0.0),
        p(1.0e6, 0.0),
        p(2.0e6, 0.0),
        p(1.5e6, 1000.0),
        p(0.5e6, 1000.0),
    ];
    assert!(extrude(&at_d_cap, &[], 1.0).is_ok());
    let past_d_cap = vec![
        p(0.0, 0.0),
        p(1.0e6, 0.0),
        p(2.0e6, 0.0),
        p(2.1e6, 0.0),
        p(2.1e6, 1000.0),
        p(1.1e6, 1000.0),
        p(0.1e6, 1000.0),
        p(0.0, 1000.0),
    ];
    let f = reject(&past_d_cap, &[], 1.0);
    assert_eq!(
        first(&f),
        (ProfileCode::AboveLimit, Stage::Caps, Defect::None)
    );
    assert_eq!(f.diagnostics()[0].field, ProfileField::LocalSize);
}

#[test]
fn non_finite_input_is_rejected_first_in_every_position() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut outer = base_outer();
        outer[2].y = bad;
        let f = reject(&outer, &[base_hole()], 1.0);
        assert_eq!(
            first(&f),
            (ProfileCode::NonFinite, Stage::Finiteness, Defect::None)
        );
        assert_eq!(f.diagnostics()[0].field, ProfileField::OuterContour);
        assert_eq!(f.diagnostics()[0].index, Some(2));
        assert_eq!(f.status(), "invalid_input");
        assert_eq!(
            f.diagnostics()[0].code.f04_code(),
            ErrorCode::NonFiniteNumber
        );
        let cavity = Cavity::polygon(vec![
            p(10.0, 10.0),
            p(10.0, bad),
            p(20.0, 20.0),
            p(20.0, 10.0),
        ]);
        let f = reject(&base_outer(), &[base_hole(), cavity], 1.0);
        let d = f.diagnostics()[0];
        assert_eq!(
            (d.code, d.field, d.cavity, d.index),
            (
                ProfileCode::NonFinite,
                ProfileField::CavityContour,
                Some(1),
                Some(1)
            )
        );
        let f = reject(&base_outer(), &[], bad);
        assert_eq!(f.diagnostics()[0].field, ProfileField::Thickness);
        assert_eq!(f.diagnostics()[0].code, ProfileCode::NonFinite);
    }
    // Finiteness precedes caps and ranges: NaN plus an above-cap value and a negative thickness.
    let mut outer = base_outer();
    outer[1].x = f64::NAN;
    outer[2].x = 2.0e9;
    let f = reject(&outer, &[], -1.0);
    assert!(f.diagnostics().iter().all(|d| d.stage == Stage::Finiteness));
    // At most 32 records, in input order.
    let many: Vec<Point2> = (0..100).map(|_| p(f64::NAN, 0.0)).collect();
    let f = reject(&many, &[], 1.0);
    assert_eq!(f.diagnostics().len(), 32);
    let indices: Vec<_> = f.diagnostics().iter().map(|d| d.index.unwrap()).collect();
    assert_eq!(indices, (0..32).collect::<Vec<_>>());
}

#[test]
fn counts_and_caps_are_checked_before_any_traversal() {
    let few = vec![p(0.0, 0.0), p(1.0, 0.0)];
    let f = reject(&few, &[], 1.0);
    assert_eq!(
        first(&f),
        (
            ProfileCode::ContourInvalid,
            Stage::Caps,
            Defect::TooFewPoints
        )
    );
    let f = reject(&base_outer(), &[Cavity::polygon(few)], 1.0);
    assert_eq!(f.diagnostics()[0].cavity, Some(0));
    assert_eq!(first(&f).2, Defect::TooFewPoints);
    // 1024 points per contour is admitted (the face cap decides later); 1025 is a limit.
    let ring = |n: usize, r: f64| -> Vec<Point2> {
        (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                p(r * a.cos(), r * a.sin())
            })
            .collect()
    };
    assert!(Profile::admit(&ring(1024, 1.0e5), &[]).is_ok());
    let f = reject(&ring(1025, 1.0e5), &[], 1.0);
    assert_eq!(
        first(&f),
        (ProfileCode::AboveLimit, Stage::Caps, Defect::None)
    );
    assert_eq!(f.diagnostics()[0].field, ProfileField::OuterContour);
    assert_eq!(f.status(), "limit_exceeded");
    // More than 256 cavities, and more than 131,072 points in total.
    let tiny = |k: f64| Cavity::polygon(vec![p(k, 1.0), p(k, 2.0), p(k + 1.0, 2.0)]);
    let cavities: Vec<Cavity> = (0..257).map(|k| tiny(f64::from(k) * 3.0)).collect();
    let f = reject(&base_outer(), &cavities, 1.0);
    assert_eq!(f.diagnostics()[0].field, ProfileField::CavityCount);
    let big = Cavity::polygon(ring(1024, 10.0));
    let cavities: Vec<Cavity> = (0..129).map(|_| big.clone()).collect();
    let f = reject(&ring(1024, 1.0e5), &cavities, 1.0);
    assert_eq!(f.diagnostics()[0].field, ProfileField::TotalPoints);
    assert_eq!(f.diagnostics()[0].code, ProfileCode::AboveLimit);
}

#[test]
fn thickness_ranges_and_same_stage_aggregation() {
    let (outer, hole) = (base_outer(), [base_hole()]);
    for (t, code) in [
        (0.0, ProfileCode::NotPositive),
        (-0.0, ProfileCode::NotPositive),
        (-3.0, ProfileCode::NotPositive),
        (f64::MIN, ProfileCode::NotPositive),
        (0.0005, ProfileCode::BelowMinimum),
        (
            f64::from_bits(0.001f64.to_bits() - 1),
            ProfileCode::BelowMinimum,
        ),
    ] {
        let f = reject(&outer, &hole, t);
        assert_eq!(first(&f), (code, Stage::Ranges, Defect::None), "t = {t}");
        assert_eq!(f.diagnostics()[0].field, ProfileField::Thickness);
        assert_eq!(
            f.diagnostics()[0].code.f04_code(),
            ErrorCode::InvalidArgument
        );
    }
    assert!(extrude(&outer, &hole, 0.001).is_ok());
    // The thickness failure and a contour failure of the same stage are both reported.
    let mut bad = base_outer();
    bad[1] = bad[0];
    let f = reject(&bad, &[], -1.0);
    assert_eq!(f.diagnostics()[0].field, ProfileField::Thickness);
    assert!(has(&f, Defect::EdgeTooShort));
    assert!(f.diagnostics().iter().all(|d| d.stage == Stage::Ranges));
}

#[test]
fn contour_defect_classes_have_precise_diagnostics() {
    let hole = [base_hole()];
    let check = |outer: &[Point2], cavities: &[Cavity], defect: Defect, code: ProfileCode| {
        let f = reject(outer, cavities, 2.0);
        assert_eq!(
            first(&f),
            (code, Stage::Ranges, defect),
            "{:?}",
            f.diagnostics()
        );
        assert!(!f.diagnostics().is_empty() && f.diagnostics().len() <= 32);
        f
    };
    let invalid = ProfileCode::ContourInvalid;
    // Repeated consecutive point, repeated closure, sub-minimum edge (0.001 mm floor).
    let mut o = base_outer();
    o.insert(1, o[0]);
    check(&o, &hole, Defect::EdgeTooShort, invalid);
    let mut o = base_outer();
    o.push(o[0]);
    let f = check(&o, &hole, Defect::EdgeTooShort, invalid);
    assert_eq!(f.diagnostics()[0].index, Some(4));
    let mut o = base_outer();
    o[1] = p(0.0009, 0.0);
    check(&o, &hole, Defect::EdgeTooShort, invalid);
    // Collinear (zero area) contour.
    let line = vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)];
    check(&line, &[], Defect::ZeroArea, invalid);
    let degenerate = Cavity::polygon(vec![p(10.0, 10.0), p(20.0, 10.0), p(30.0, 10.0)]);
    check(&base_outer(), &[degenerate], Defect::ZeroArea, invalid);
    // Orientation: clockwise outer, counter-clockwise cavity (F06 section 2: rejected, never reordered).
    let mut cw = base_outer();
    cw.reverse();
    check(&cw, &hole, Defect::WrongWinding, invalid);
    let ccw_cavity = Cavity::polygon(rect(10.0, 10.0, 20.0, 20.0));
    check(&base_outer(), &[ccw_cavity], Defect::WrongWinding, invalid);
    // Minimum area below eps^2 with every edge above the floor.
    let sliver = Cavity::polygon(vec![p(10.0, 10.0), p(10.5, 10.0000001), p(11.0, 10.0)]);
    check(&base_outer(), &[sliver], Defect::AreaTooSmall, invalid);
    // Self-intersection: an unequal figure eight.
    let eight = ccw(vec![p(0.0, 0.0), p(80.0, 80.0), p(80.0, 0.0), p(0.0, 50.0)]);
    check(&eight, &[], Defect::SelfIntersection, invalid);
    // Non-consecutive duplicate point (a pinch).
    let pinch = ccw(vec![
        p(0.0, 0.0),
        p(100.0, 0.0),
        p(50.0, 50.0),
        p(100.0, 100.0),
        p(0.0, 100.0),
        p(50.0, 50.0),
    ]);
    check(&pinch, &[], Defect::SelfIntersection, invalid);
    // A vertex touching a non-adjacent edge.
    let touch = ccw(vec![
        p(0.0, 0.0),
        p(100.0, 0.0),
        p(100.0, 100.0),
        p(50.0, 0.0),
        p(0.0, 100.0),
    ]);
    check(&touch, &[], Defect::SelfIntersection, invalid);
    // Adjacent backtracking along one line.
    let fold = vec![p(0.0, 0.0), p(100.0, 0.0), p(50.0, 0.0), p(50.0, 100.0)];
    let f = reject(&fold, &[], 2.0);
    assert!(has(&f, Defect::Backtracking), "{:?}", f.diagnostics());
    // A slit narrower than the floor between non-adjacent edges.
    let slit = vec![
        p(0.0, 0.0),
        p(100.0, 0.0),
        p(100.0, 100.0),
        p(50.0005, 100.0),
        p(50.0005, 20.001),
        p(50.0, 20.0),
        p(50.0, 100.0),
        p(0.0, 100.0),
    ];
    check(&slit, &[], Defect::NarrowFeature, invalid);
}

#[test]
fn contour_relationship_defect_classes() {
    let invalid = ProfileCode::ContourInvalid;
    let check = |cavities: &[Cavity], defect: Defect, code: ProfileCode| {
        let f = reject(&base_outer(), cavities, 2.0);
        assert_eq!(
            first(&f),
            (code, Stage::Ranges, defect),
            "{:?}",
            f.diagnostics()
        );
        f
    };
    // Crossing, edge-overlap, vertex-touch and near-miss between cavity and outer contour.
    check(
        &[hole_rect(90.0, 40.0, 110.0, 60.0)],
        Defect::ContoursIntersect,
        invalid,
    );
    check(
        &[hole_rect(80.0, 40.0, 100.0, 60.0)],
        Defect::ContoursIntersect,
        invalid,
    );
    let diamond = Cavity::polygon(vec![
        p(100.0, 50.0),
        p(90.0, 40.0),
        p(80.0, 50.0),
        p(90.0, 60.0),
    ]);
    check(&[diamond], Defect::ContoursIntersect, invalid);
    check(
        &[hole_rect(0.0005, 40.0, 20.0, 60.0)],
        Defect::ContoursTooClose,
        invalid,
    );
    // Two cavities crossing, touching at a corner, and closer than the floor.
    check(
        &[
            hole_rect(10.0, 10.0, 30.0, 30.0),
            hole_rect(20.0, 20.0, 40.0, 40.0),
        ],
        Defect::ContoursIntersect,
        invalid,
    );
    check(
        &[
            hole_rect(10.0, 10.0, 20.0, 20.0),
            hole_rect(20.0, 20.0, 30.0, 30.0),
        ],
        Defect::ContoursIntersect,
        invalid,
    );
    check(
        &[
            hole_rect(10.0, 10.0, 20.0, 20.0),
            hole_rect(20.0005, 10.0, 30.0, 20.0),
        ],
        Defect::ContoursTooClose,
        invalid,
    );
    // Outside the outer contour, and an island inside another cavity (valid but unsupported).
    check(
        &[hole_rect(200.0, 200.0, 210.0, 210.0)],
        Defect::CavityOutside,
        invalid,
    );
    let f = check(
        &[
            hole_rect(10.0, 10.0, 60.0, 60.0),
            hole_rect(20.0, 20.0, 30.0, 30.0),
        ],
        Defect::CavityNested,
        ProfileCode::UnsupportedContour,
    );
    assert_eq!(f.status(), "unsupported");
    assert_eq!(
        f.diagnostics()[0].code.f04_code(),
        ErrorCode::UnsupportedGeometry
    );
    assert_eq!(f.diagnostics()[0].cavity, Some(1));
    // Invalid contours outrank the support decision: nested + outside reports the invalid one.
    let f = reject(
        &base_outer(),
        &[
            hole_rect(10.0, 10.0, 60.0, 60.0),
            hole_rect(20.0, 20.0, 30.0, 30.0),
            hole_rect(300.0, 300.0, 310.0, 310.0),
        ],
        2.0,
    );
    assert_eq!(first(&f).2, Defect::CavityOutside);
}

#[test]
fn curve_generators_reject_bad_requests_with_typed_codes() {
    let spec = CurveSpec::DEFAULT;
    let at = p(0.0, 0.0);
    let code = |r: Result<Cavity, ProfileFailure>| {
        let f = r.unwrap_err();
        assert_eq!(f.diagnostics()[0].field, ProfileField::Curve);
        (f.diagnostics()[0].code, f.diagnostics()[0].defect)
    };
    assert_eq!(
        code(Cavity::circle(at, 0.0, spec)),
        (ProfileCode::NotPositive, Defect::CurveInput)
    );
    assert_eq!(
        code(Cavity::circle(at, -1.0, spec)),
        (ProfileCode::NotPositive, Defect::CurveInput)
    );
    assert_eq!(
        code(Cavity::circle(at, f64::NAN, spec)).0,
        ProfileCode::NonFinite
    );
    assert_eq!(
        code(Cavity::circle(p(f64::INFINITY, 0.0), 1.0, spec)).0,
        ProfileCode::NonFinite
    );
    assert_eq!(
        code(Cavity::circle(p(2.0e9, 0.0), 1.0, spec)).0,
        ProfileCode::AboveLimit
    );
    assert_eq!(
        code(Cavity::circle(at, 1.0e6 + 1.0, spec)).0,
        ProfileCode::AboveLimit
    );
    let sag = |s: f64| CurveSpec {
        max_sagitta_mm: s,
        ..spec
    };
    assert_eq!(
        code(Cavity::circle(at, 5.0, sag(0.0))).0,
        ProfileCode::NotPositive
    );
    assert_eq!(
        code(Cavity::circle(at, 5.0, sag(f64::NAN))).0,
        ProfileCode::NonFinite
    );
    let too_many = CurveSpec {
        max_segments: 1021,
        ..spec
    };
    assert_eq!(
        code(Cavity::circle(at, 5.0, too_many)).0,
        ProfileCode::AboveLimit
    );
    let inverted = CurveSpec {
        min_segments: 200,
        max_segments: 100,
        ..spec
    };
    assert_eq!(
        code(Cavity::circle(at, 5.0, inverted)).0,
        ProfileCode::ContourInvalid
    );
    let unreachable = CurveSpec {
        max_sagitta_mm: 1e-6,
        min_segments: 8,
        max_segments: 512,
    };
    assert_eq!(
        code(Cavity::circle(at, 1.0e5, unreachable)),
        (ProfileCode::AboveLimit, Defect::SagittaUnreachable)
    );
    assert_eq!(
        code(Cavity::slot(at, 5.0, 5.0, p(1.0, 0.0), spec)).0,
        ProfileCode::NotPositive
    );
    assert_eq!(
        code(Cavity::slot(at, 5.0, 6.0, p(1.0, 0.0), spec)).0,
        ProfileCode::NotPositive
    );
    assert_eq!(
        code(Cavity::slot(at, 9.0, 3.0, p(0.0, 0.0), spec)).0,
        ProfileCode::NotPositive
    );
    assert_eq!(
        code(Cavity::slot(at, 9.0, 3.0, p(f64::NAN, 1.0), spec)).0,
        ProfileCode::NonFinite
    );
    assert_eq!(
        code(Cavity::ellipse(at, 0.0, 3.0, p(1.0, 0.0), spec)).0,
        ProfileCode::NotPositive
    );
    assert_eq!(
        code(Cavity::ellipse(at, 4.0, 3.0, p(1e-160, 0.0), spec)).0,
        ProfileCode::NotPositive
    );
    // A hole too small for the plate's floor is rejected at admission, not generation.
    let tiny = Cavity::circle(p(50.0, 50.0), 0.0001, spec).unwrap();
    let f = reject(&base_outer(), &[tiny], 2.0);
    assert_eq!(
        first(&f),
        (
            ProfileCode::ContourInvalid,
            Stage::Ranges,
            Defect::EdgeTooShort
        )
    );
    // The hard segment bounds: the minimum is rounded up, the maximum down, to multiples of four.
    let pinned = CurveSpec {
        max_sagitta_mm: 10.0,
        min_segments: 9,
        max_segments: 13,
    };
    assert_eq!(Cavity::circle(at, 5.0, pinned).unwrap().points().len(), 12);
    let floor = CurveSpec {
        max_sagitta_mm: 10.0,
        min_segments: 0,
        max_segments: 1020,
    };
    assert_eq!(Cavity::circle(at, 5.0, floor).unwrap().points().len(), 8);
}

#[test]
fn every_code_maps_to_its_f06_token_and_f04_code() {
    use ProfileCode::*;
    for (code, token, f04, status) in [
        (
            NonFinite,
            "GEO_NON_FINITE",
            ErrorCode::NonFiniteNumber,
            "invalid_input",
        ),
        (
            NotPositive,
            "GEO_NOT_POSITIVE",
            ErrorCode::InvalidArgument,
            "invalid_input",
        ),
        (
            BelowMinimum,
            "GEO_BELOW_MINIMUM",
            ErrorCode::InvalidArgument,
            "invalid_input",
        ),
        (
            AboveLimit,
            "GEO_ABOVE_LIMIT",
            ErrorCode::LimitExceeded,
            "limit_exceeded",
        ),
        (
            ContourInvalid,
            "GEO_CONTOUR_INVALID",
            ErrorCode::InvalidArgument,
            "invalid_input",
        ),
        (
            UnsupportedContour,
            "GEO_UNSUPPORTED_CONTOUR",
            ErrorCode::UnsupportedGeometry,
            "unsupported",
        ),
        (
            BufferLimit,
            "GEO_BUFFER_LIMIT",
            ErrorCode::LimitExceeded,
            "limit_exceeded",
        ),
        (
            TopologyLimit,
            "GEO_TOPOLOGY_LIMIT",
            ErrorCode::LimitExceeded,
            "limit_exceeded",
        ),
        (
            NumericUncertain,
            "GEO_NUMERIC_UNCERTAIN",
            ErrorCode::UnsupportedGeometry,
            "unsupported",
        ),
        (
            SelfCheckFailed,
            "GEO_SELF_CHECK_FAILED",
            ErrorCode::UnsupportedGeometry,
            "unsupported",
        ),
        (
            Cancelled,
            "GEO_CANCELLED",
            ErrorCode::Cancelled,
            "cancelled",
        ),
    ] {
        assert_eq!(code.as_str(), token);
        assert_eq!(code.f04_code(), f04);
        let _ = status;
    }
    // Fixed tokens, shorter than 64 bytes, never echoing input.
    for field in [
        ProfileField::Thickness,
        ProfileField::OuterContour,
        ProfileField::CavityContour,
        ProfileField::CavityCount,
        ProfileField::TotalPoints,
        ProfileField::LocalSize,
        ProfileField::Curve,
        ProfileField::Mesh,
        ProfileField::Volume,
        ProfileField::Cancellation,
    ] {
        assert!(field.as_str().len() < 64 && field.as_str().is_ascii());
    }
    let f = reject(&base_outer(), &[], f64::NAN);
    assert_eq!(f.to_string(), "profile evaluation failed");
}

#[test]
fn cancellation_at_every_point_discards_the_candidate() {
    let (outer, hole) = (
        base_outer(),
        [Cavity::circle(p(50.0, 50.0), 10.0, CurveSpec::DEFAULT).unwrap()],
    );
    let seen = RefCell::new(Vec::new());
    let ok = extrude_with_cancel(&outer, &hole, 4.0, |s| {
        seen.borrow_mut().push(s);
        false
    })
    .unwrap();
    // Exactly P0..P3 in order, and nothing after the seal.
    assert_eq!(
        *seen.borrow(),
        [
            Stage::Validation,
            Stage::Build,
            Stage::Checks,
            Stage::Quantities
        ]
    );
    assert_valid_extrusion(&ok, &outer, &hole, 4.0);
    for (index, stage) in [
        Stage::Validation,
        Stage::Build,
        Stage::Checks,
        Stage::Quantities,
    ]
    .into_iter()
    .enumerate()
    {
        let polls = RefCell::new(0usize);
        let f = extrude_with_cancel(&outer, &hole, 4.0, |_| {
            let hit = *polls.borrow() == index;
            *polls.borrow_mut() += 1;
            hit
        })
        .unwrap_err();
        assert_eq!(first(&f), (ProfileCode::Cancelled, stage, Defect::None));
        assert_eq!(f.diagnostics()[0].field, ProfileField::Cancellation);
        assert_eq!(f.status(), "cancelled");
        assert_eq!(f.diagnostics()[0].code.f04_code(), ErrorCode::Cancelled);
        assert_eq!(*polls.borrow(), index + 1, "no poll after a cancellation");
    }
    // Cancellation at entry precedes validation: an invalid input still reports cancelled.
    let f = extrude_with_cancel(&outer, &[], f64::NAN, |_| true).unwrap_err();
    assert_eq!(first(&f).0, ProfileCode::Cancelled);
    // The admitted-profile entry point polls the same four points.
    let profile = Profile::admit(&outer, &hole).unwrap();
    let seen = RefCell::new(Vec::new());
    extrude_profile(&profile, 4.0, |s| {
        seen.borrow_mut().push(s);
        false
    })
    .unwrap();
    assert_eq!(seen.borrow().len(), 4);
    let f = extrude_profile(&profile, 0.0, |_| false).unwrap_err();
    assert_eq!(first(&f).0, ProfileCode::NotPositive);
    let f = extrude_profile(&profile, f64::NAN, |_| false).unwrap_err();
    assert_eq!(first(&f).0, ProfileCode::NonFinite);
    let f = extrude_profile(&profile, 2.0e6, |_| false).unwrap_err();
    assert_eq!(first(&f).0, ProfileCode::AboveLimit);
    let f = extrude_profile(&profile, 1e-9, |_| false).unwrap_err();
    assert_eq!(first(&f).0, ProfileCode::BelowMinimum);
}

fn random_star(rng: &mut Rng, vertices: usize) -> Vec<Point2> {
    // Strictly increasing angles with bounded gaps around the origin: simple and counter-clockwise.
    let step = std::f64::consts::TAU / vertices as f64;
    (0..vertices)
        .map(|k| {
            let angle = (k as f64 + rng.range(0.1, 0.9)) * step;
            let radius = rng.range(60.0, 100.0);
            p(radius * angle.cos(), radius * angle.sin())
        })
        .collect()
}

fn random_holes(rng: &mut Rng) -> Vec<Cavity> {
    let mut holes = Vec::new();
    for gy in -1..=1 {
        for gx in -1..=1 {
            if rng.below(3) == 0 {
                continue;
            }
            let (cx, cy) = (
                f64::from(gx) * 14.0 + rng.range(-1.5, 1.5),
                f64::from(gy) * 14.0 + rng.range(-1.5, 1.5),
            );
            let hole = match rng.below(4) {
                0 => Cavity::circle(p(cx, cy), rng.range(2.0, 5.0), CurveSpec::DEFAULT).unwrap(),
                1 => Cavity::slot(
                    p(cx, cy),
                    rng.range(8.0, 10.0),
                    rng.range(2.0, 4.0),
                    p(rng.range(-1.0, 1.0), 1.0),
                    CurveSpec::DEFAULT,
                )
                .unwrap(),
                2 => Cavity::ellipse(
                    p(cx, cy),
                    rng.range(3.0, 5.0),
                    rng.range(2.0, 3.0),
                    p(1.0, rng.range(-1.0, 1.0)),
                    CurveSpec::DEFAULT,
                )
                .unwrap(),
                _ => {
                    let r = rng.range(2.0, 5.0);
                    Cavity::polygon(vec![
                        p(cx - r, cy - r),
                        p(cx - r, cy + r),
                        p(cx + r, cy + r),
                        p(cx + r, cy - r),
                    ])
                }
            };
            holes.push(hole);
        }
    }
    holes
}

#[test]
fn seeded_valid_profiles_always_extrude_and_survive_exact_transforms() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..120 {
        let vertices = 12 + rng.below(50) as usize;
        let outer = random_star(&mut rng, vertices);
        let holes = random_holes(&mut rng);
        let t = rng.range(0.5, 40.0);
        let ex = match extrude(&outer, &holes, t) {
            Ok(ex) => ex,
            Err(f) => panic!("case {case}: valid profile rejected: {:?}", f.diagnostics()),
        };
        assert_valid_extrusion(&ex, &outer, &holes, t);
        // Rigid exact motions of the same profile: translation by a dyadic vector, quarter turn.
        let shift = |q: Point2| p(q.x + 1024.0, q.y - 4096.0);
        let spin = |q: Point2| p(-q.y, q.x);
        for f in [&shift as &dyn Fn(Point2) -> Point2, &spin] {
            let o2: Vec<Point2> = outer.iter().map(|q| f(*q)).collect();
            let c2: Vec<Cavity> = holes
                .iter()
                .map(|c| Cavity::polygon(c.points().iter().map(|q| f(*q)).collect()))
                .collect();
            let moved = extrude(&o2, &c2, t).unwrap();
            assert_valid_extrusion(&moved, &o2, &c2, t);
            assert!(close(moved.volume_mm3(), ex.volume_mm3(), 1e-9));
            assert_eq!(moved.mesh().triangles().len(), ex.mesh().triangles().len());
        }
    }
}

#[test]
fn hostile_random_contours_never_panic_or_hang_and_valid_survivors_are_certified() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let mut accepted = 0;
    for _ in 0..1500 {
        let point = |rng: &mut Rng| -> Point2 {
            match rng.below(40) {
                0 => p(f64::NAN, rng.range(-5.0, 5.0)),
                1 => p(rng.range(-5.0, 5.0), f64::INFINITY),
                2 => p(1.0e300, -1.0e300),
                3 => p(f64::from_bits(1), 0.0),
                4 => p(-0.0, -0.0),
                5 => p(rng.range(-5.0, 5.0).round(), rng.range(-5.0, 5.0).round()),
                _ => p(rng.range(-50.0, 50.0), rng.range(-50.0, 50.0)),
            }
        };
        let n = rng.below(9) as usize;
        let outer: Vec<Point2> = (0..n).map(|_| point(&mut rng)).collect();
        let cavities: Vec<Cavity> = (0..rng.below(4))
            .map(|_| Cavity::polygon((0..rng.below(7)).map(|_| point(&mut rng)).collect()))
            .collect();
        let t = match rng.below(8) {
            0 => f64::NAN,
            1 => -1.0,
            2 => 0.0,
            _ => rng.range(0.001, 20.0),
        };
        if let Ok(ex) = extrude(&outer, &cavities, t) {
            accepted += 1;
            assert_valid_extrusion(&ex, &outer, &cavities, t);
        }
    }
    assert!(accepted > 20, "{accepted} accepted");
}

#[test]
fn hostile_perturbations_of_valid_profiles_are_safe() {
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    for _ in 0..200 {
        let vertices = 12 + rng.below(30) as usize;
        let outer = random_star(&mut rng, vertices);
        let holes = random_holes(&mut rng);
        let (mut o, mut cs) = (outer, holes);
        for _ in 0..1 + rng.below(3) {
            let target = rng.below(cs.len() as u64 + 1) as usize;
            let mut points: Vec<Point2> = if target == 0 {
                o.clone()
            } else {
                cs[target - 1].points().to_vec()
            };
            let k = rng.below(points.len() as u64) as usize;
            match rng.below(6) {
                0 => {
                    points[k] = p(
                        points[k].x + rng.range(-30.0, 30.0),
                        points[k].y + rng.range(-30.0, 30.0),
                    )
                }
                1 => {
                    let other = (k + 1) % points.len();
                    points.swap(k, other);
                }
                2 => points.insert(k, points[k]),
                3 => points[k].x = f64::NAN,
                4 => points.reverse(),
                _ => points.rotate_left(k),
            }
            if target == 0 {
                o = points;
            } else {
                cs[target - 1] = Cavity::polygon(points);
            }
        }
        if let Ok(ex) = extrude(&o, &cs, 3.0) {
            assert_valid_extrusion(&ex, &o, &cs, 3.0);
        }
    }
}

#[test]
fn maximum_size_hostile_and_valid_contours_finish() {
    let mut rng = Rng(77);
    // Random 1024-point contours: rejected quickly by the bounded exact pair scan.
    let noise = |rng: &mut Rng| -> Vec<Point2> {
        (0..1024)
            .map(|_| p(rng.range(-1.0e4, 1.0e4), rng.range(-1.0e4, 1.0e4)))
            .collect()
    };
    let outer = noise(&mut rng);
    assert!(extrude(&outer, &[], 1.0).is_err());
    assert!(extrude(&outer, &[Cavity::polygon(noise(&mut rng))], 1.0).is_err());
    // A valid 1020-point radial zigzag (many reflex vertices) triangulates within the work budget.
    let zig: Vec<Point2> = (0..1020)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 1020.0;
            let r = if k % 2 == 0 { 1.0e5 } else { 0.8e5 };
            p(r * a.cos(), r * a.sin())
        })
        .collect();
    let ex = extrude(&zig, &[], 12.0).unwrap();
    assert_valid_extrusion(&ex, &zig, &[], 12.0);
}

#[test]
fn clearance_decision_is_the_same_before_and_after_a_lossless_translation() {
    // Regression: the clearance (0.0009999462 mm) between a slanted outer edge and a cavity edge
    // is below the 0.001 mm floor, and must stay below it when both are translated by ~1e9.
    let g = 1.0 / 8_388_608.0;
    let on_grid = |v: f64| (v / g).round() * g;
    for shift in [0.0, 999_999_800.0] {
        for (y0, too_close) in [
            (1.7511576414108276, true),
            (on_grid(1.7511576414108276) + 8.0 * g, false),
        ] {
            assert_eq!(
                y0,
                on_grid(y0),
                "the cavity heights are exactly representable at the cap"
            );
            let at = |x: f64, y: f64| p(x + shift, y + shift);
            let outer = vec![at(0.0, 0.0), at(12.0, 7.0), at(0.0, 40.0)];
            let cavity = Cavity::polygon(vec![at(3.0, y0), at(3.0, 10.0), at(6.0, y0 + 1.75)]);
            let result = extrude(&outer, &[cavity], 2.0);
            if too_close {
                let f = result.unwrap_err();
                assert_eq!(
                    first(&f).2,
                    Defect::ContoursTooClose,
                    "shift {shift}: {:?}",
                    f.diagnostics()
                );
            } else {
                assert!(
                    result.is_ok(),
                    "shift {shift}: {:?}",
                    result.err().map(|f| f.diagnostics().to_vec())
                );
            }
        }
    }
}

#[test]
fn every_contour_edge_is_capped_at_one_million_millimetres() {
    // F04 section 3 / F06 section 2: each edge, closure and cavity edges included, is at most 1e6 mm.
    let at_cap = rect(0.0, 0.0, 1.0e6, 10.0);
    assert!(extrude(&at_cap, &[], 1.0).is_ok());
    let over = f64::from_bits(1.0e6f64.to_bits() + 1);
    let f = reject(&rect(0.0, 0.0, over, 10.0), &[], 1.0);
    assert_eq!(
        first(&f),
        (ProfileCode::AboveLimit, Stage::Caps, Defect::None)
    );
    assert_eq!(f.diagnostics()[0].field, ProfileField::OuterContour);
    // Both long edges are reported, bottom (0) and top (2), in input order.
    let indices: Vec<_> = f.diagnostics().iter().map(|d| d.index).collect();
    assert_eq!(indices, [Some(0), Some(2)]);
    assert_eq!(f.status(), "limit_exceeded");
    // The closing edge counts: the last point back to the first.
    let closing = vec![p(0.0, 0.0), p(0.0, over), p(-10.0, over)];
    let f = reject(&closing, &[], 1.0);
    assert!(
        f.diagnostics()
            .iter()
            .any(|d| d.code == ProfileCode::AboveLimit && d.index == Some(0))
    );
    // Cavity edges, including the closing edge, inside a large 16-gon (every outer edge < 1e6).
    let big: Vec<Point2> = (0..16)
        .map(|k| {
            let a = std::f64::consts::TAU * f64::from(k) / 16.0;
            p(0.99e6 * a.cos(), 0.99e6 * a.sin())
        })
        .collect();
    let long_cavity =
        |half: f64| Cavity::polygon(vec![p(-half, 0.0), p(0.0, 50_000.0), p(half, 0.0)]);
    assert!(extrude(&big, &[long_cavity(5.0e5)], 1.0).is_ok());
    let f = reject(&big, &[long_cavity(500_000.5)], 1.0);
    let d = f.diagnostics()[0];
    assert_eq!(
        (d.code, d.stage, d.field, d.cavity, d.index),
        (
            ProfileCode::AboveLimit,
            Stage::Caps,
            ProfileField::CavityContour,
            Some(0),
            Some(2)
        )
    );
}

#[test]
fn every_failure_of_the_ranges_and_contour_stage_is_reported_together() {
    // Reported regression: a thickness failure used to hide the figure-eight's self-intersection.
    let eight = ccw(vec![p(0.0, 0.0), p(80.0, 80.0), p(80.0, 0.0), p(0.0, 50.0)]);
    let f = reject(&eight, &[], -1.0);
    let found: Vec<_> = f
        .diagnostics()
        .iter()
        .map(|d| (d.field, d.defect))
        .collect();
    assert_eq!(found[0], (ProfileField::Thickness, Defect::None));
    assert!(
        found.contains(&(ProfileField::OuterContour, Defect::SelfIntersection)),
        "{found:?}"
    );
    assert!(f.diagnostics().iter().all(|d| d.stage == Stage::Ranges));
    // Independent defects of different contours, in input order: outer figure-eight, a clockwise
    // outer is not involved; cavity 0 is a short-edge contour, cavity 1 is counter-clockwise.
    let short = Cavity::polygon(vec![
        p(10.0, 10.0),
        p(10.0, 10.0005),
        p(20.0, 20.0),
        p(20.0, 10.0),
    ]);
    let wrong = Cavity::polygon(rect(30.0, 30.0, 40.0, 40.0));
    let outer = ccw(vec![
        p(0.0, 0.0),
        p(100.0, 100.0),
        p(100.0, 0.0),
        p(0.0, 60.0),
    ]);
    let f = reject(&outer, &[short, wrong], 2.0);
    let found: Vec<_> = f
        .diagnostics()
        .iter()
        .map(|d| (d.cavity, d.defect))
        .collect();
    assert_eq!(
        found,
        [
            (None, Defect::SelfIntersection),
            (Some(0), Defect::EdgeTooShort),
            (Some(1), Defect::WrongWinding)
        ],
        "{found:?}"
    );
    // A pair check runs once both contours are simple even if another contour is not.
    let eight_cavity = Cavity::polygon(vec![
        p(40.0, 40.0),
        p(60.0, 60.0),
        p(60.0, 40.0),
        p(40.0, 60.0),
    ]);
    let f = reject(
        &base_outer(),
        &[eight_cavity, hole_rect(90.0, 40.0, 110.0, 60.0)],
        2.0,
    );
    let found: Vec<_> = f
        .diagnostics()
        .iter()
        .map(|d| (d.cavity, d.defect))
        .collect();
    assert!(
        found.contains(&(Some(1), Defect::ContoursIntersect)),
        "{found:?}"
    );
    // Containment is skipped for a crossing pair (no spurious "outside"), and the island
    // support decision only appears when nothing invalid was found.
    assert!(!found.contains(&(Some(1), Defect::CavityOutside)));
    assert!(!found.iter().any(|(_, d)| *d == Defect::CavityNested));
}
