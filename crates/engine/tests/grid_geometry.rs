// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Independent bounds and identity checks for display-only grid tubes.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use beamcraft_engine::grid_geometry::guide_mesh;
use beamcraft_model::{
    grid::{DrawnGridLine, Grid, GridFamily},
    id::ElementId,
};
#[test]
fn tube_diameter_endpoints_and_geometry_digest_are_independent_of_labels_and_frame() {
    let id = |n| ElementId::parse(&format!("01990000-0000-7000-8000-{n:012x}")).unwrap();
    let mut g = Grid {
        id: id(1),
        name: "G".into(),
        translation_mm: [1000., 2000.],
        orientation_zw: [0., 1.],
        u: GridFamily::Drawn(vec![DrawnGridLine {
            axis_id: id(2),
            label: "U".into(),
            start_mm: [0., 0.],
            end_mm: [1000., 0.],
        }]),
        v: GridFamily::Drawn(vec![]),
    };
    let mesh = guide_mesh(&g).unwrap();
    assert_eq!(mesh.points_mm.len(), 32);
    assert_eq!(mesh.face_counts.len(), 18);
    assert_eq!(mesh.indices.len(), 96);
    for p in &mesh.points_mm {
        assert!(p[0] == 0. || p[0] == 1000.);
        assert!((p[1].hypot(p[2]) - 25.).abs() < 1e-5);
    }
    assert!(
        mesh.indices
            .iter()
            .all(|i| *i >= 0 && (*i as usize) < mesh.points_mm.len())
    );
    g.name = "Renamed".into();
    g.translation_mm = [-2000., 4000.];
    g.orientation_zw = [1., 0.];
    let GridFamily::Drawn(lines) = &mut g.u else {
        panic!()
    };
    lines[0].label = "Another".into();
    assert_eq!(mesh, guide_mesh(&g).unwrap());
}
