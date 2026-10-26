// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! T01.4 grid query and placement regression scenarios.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use beamcraft_engine::grid_query::*;
use beamcraft_engine::query::pick::{SnapKind, SnapQuery, snap_with_grids};
use beamcraft_engine::query::snapshot::{ViewPin, ViewSnapshot};
use beamcraft_model::{
    digest::Sha256,
    grid::{DrawnGridLine, Grid, GridFamily},
    id::ElementId,
};
fn id(n: u8) -> ElementId {
    ElementId::parse(&format!("01990000-0000-7000-8000-{n:012x}")).unwrap()
}
fn grid() -> Grid {
    Grid {
        id: id(1),
        name: "G".into(),
        translation_mm: [1000., 2000.],
        orientation_zw: [0., 1.],
        u: GridFamily::Drawn(vec![DrawnGridLine {
            axis_id: id(2),
            label: "U".into(),
            start_mm: [0., 0.],
            end_mm: [3000., 4000.],
        }]),
        v: GridFamily::Drawn(vec![DrawnGridLine {
            axis_id: id(3),
            label: "V".into(),
            start_mm: [3000., 0.],
            end_mm: [0., 4000.],
        }]),
    }
}
#[test]
fn drawn_crossings_endpoints_rotations_and_invalid_pairs() {
    let mut g = grid();
    assert_eq!(grid_point(&g, "U", "V").unwrap(), [2500., 4000.]);
    assert_eq!(grid_point(&g, "V", "U").unwrap(), [2500., 4000.]);
    assert_eq!(
        grid_point(&g, "U", "U").unwrap_err().message,
        "GRID_INTERSECTION_UNDEFINED"
    );
    assert_eq!(
        grid_point(&g, "?", "U").unwrap_err().message,
        "GRID_AXIS_UNKNOWN"
    );
    let GridFamily::Drawn(v) = &mut g.v else {
        panic!()
    };
    v[0].start_mm = [3000., 0.];
    v[0].end_mm = [3000., 4000.];
    assert_eq!(grid_point(&g, "U", "V").unwrap(), [4000., 6000.]);
    g.orientation_zw = [0.5_f64.sqrt(), 0.5_f64.sqrt()];
    assert_eq!(grid_point(&g, "U", "V").unwrap(), [-3000., 5000.]);
    let p = grid_line_plane(&g, "V").unwrap();
    assert!((p.normal_world[1].abs() - 1.).abs() < 1e-12);
    let mut other = g.clone();
    other.id = id(4);
    let GridFamily::Drawn(u) = &mut other.u else {
        panic!()
    };
    u[0].axis_id = id(5);
    let GridFamily::Drawn(v) = &mut other.v else {
        panic!()
    };
    v[0].axis_id = id(6);
    assert_eq!(
        resolve_grid(&[g, other], "G").unwrap_err().message,
        "GRID_NAME_AMBIGUOUS"
    );
}
#[test]
fn snaps_preserve_height_filter_and_bound_results() {
    let g = grid();
    let pin = ViewPin::new(Sha256::digest(b"grid").unwrap(), 1);
    let snapshot = ViewSnapshot::build(pin, Vec::new(), &Default::default()).unwrap();
    let q = SnapQuery {
        point_mm: [2500., 4000., 3000.],
        radius_mm: 10.,
        kinds: vec![SnapKind::GridIntersection, SnapKind::GridLine],
        max_hits: 2,
    };
    let hits = snap_with_grids(&snapshot, &q, std::slice::from_ref(&g), None).unwrap();
    assert_eq!(hits.candidates.len(), 2);
    assert!(hits.truncated);
    assert_eq!(hits.candidates[0].kind, SnapKind::GridIntersection);
    for c in hits.candidates {
        assert_eq!(c.point_world_mm[2], 3000.);
        assert!(c.feature_id.is_none());
        assert!(c.grid.is_some());
    }
    let empty = std::collections::BTreeSet::new();
    assert!(
        snap_with_grids(&snapshot, &q, &[g], Some(&empty))
            .unwrap()
            .candidates
            .is_empty()
    );
}

#[test]
fn parallel_collinear_and_outside_segments_have_no_unique_intersection() {
    let mut g = grid();
    for (start, end) in [
        ([0., 10.], [3000., 4010.]),
        ([0., 0.], [3000., 4000.]),
        ([3000., 5000.], [3000., 6000.]),
    ] {
        let GridFamily::Drawn(v) = &mut g.v else {
            panic!()
        };
        v[0].start_mm = start;
        v[0].end_mm = end;
        assert_eq!(
            grid_point(&g, "U", "V").unwrap_err().message,
            "GRID_INTERSECTION_UNDEFINED"
        );
    }
}
