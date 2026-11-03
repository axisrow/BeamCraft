// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G10 capacity: certification holds tens of holes per plate within the default limits.
//!
//! The measured before/after for the change that landed this lives in the PR and the issue;
//! this test pins the acceptance floor: a 1000x500x10 plate admits twenty-four round holes
//! (16 edges each), punched one by one under the default operation limits, and the one-shot
//! extrusion of the same profile certifies as a single body.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use crate::common::{close, p, rect, regular_polygon_area};
use crate::common_ops::{assert_sealed, plate};

use beamcraft_kernel::ops::{Body, HoleCutter, through_hole};
use beamcraft_kernel::profile::{Cavity, CurveSpec, extrude};

const HOLES: u16 = 24;
const SEGMENTS: u32 = 16;
const RADIUS: f64 = 12.0;
const THICKNESS: f64 = 10.0;

fn circle(cx: f64, cy: f64) -> Cavity {
    Cavity::circle(
        p(cx, cy),
        RADIUS,
        CurveSpec {
            max_sagitta_mm: RADIUS,
            min_segments: SEGMENTS,
            max_segments: SEGMENTS,
        },
    )
    .unwrap()
}

/// Six columns by four rows across the 1000x500 outline, far from every edge.
fn hole_centre(k: u16) -> (f64, f64) {
    let (col, row) = (u32::from(k) % 6, u32::from(k) / 6);
    (
        100.0 + 160.0 * f64::from(col),
        80.0 + 120.0 * f64::from(row),
    )
}

fn cavity_volume_mm3() -> f64 {
    f64::from(HOLES) * regular_polygon_area(SEGMENTS, RADIUS) * THICKNESS
}

#[test]
fn a_plate_certifies_tens_of_round_holes_within_the_default_limits() {
    let outline = rect(0.0, 0.0, 1000.0, 500.0);
    let cavities: Vec<Cavity> = (0..HOLES)
        .map(|k| {
            let (x, y) = hole_centre(k);
            circle(x, y)
        })
        .collect();
    let expected = 1000.0 * 500.0 * THICKNESS - cavity_volume_mm3();

    // One shot: the extrusion exists only after its shell certified, so the single
    // certification of the whole 24-hole body is inside this call.
    let ex = extrude(&outline, &cavities, THICKNESS).unwrap();
    assert!(close(ex.volume_mm3(), expected, 1e-9));

    // Hole by hole: every call re-certifies the whole growing body under the
    // default operation limits, and nothing is ever refused.
    let mut body: Body = plate(1000.0, 500.0, THICKNESS);
    for k in 0..HOLES {
        let (x, y) = hole_centre(k);
        let cutter = HoleCutter::new(circle(x, y), -1.0, THICKNESS + 1.0, k + 1);
        let res = through_hole(&body, &cutter).unwrap();
        let facts = assert_sealed(&res);
        assert!(close(
            facts.volume,
            1000.0 * 500.0 * THICKNESS
                - f64::from(k + 1) * regular_polygon_area(SEGMENTS, RADIUS) * THICKNESS,
            1e-9
        ));
        body = res.into_body();
    }
    assert!(close(ex.volume_mm3(), body.volume_mm3().lo, 1e-9));
}
