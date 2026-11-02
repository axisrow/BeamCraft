// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic frames and views, MPL-2.0.

//! View construction against evaluated geometry: visible picking, surface and part frames.

#![allow(clippy::unwrap_used)]
use beamcraft_engine::query::pick::SelectionFilter;
use beamcraft_engine::views::{self, ModelView, PartSide};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_engine::{ElementKind, KindFilter, Limits, ViewPin, ViewSnapshot};

fn snapshot() -> ViewSnapshot {
    let pin = ViewPin::new(
        beamcraft_model::digest::Sha256::digest(b"view-pick").unwrap(),
        1,
    );
    ViewSnapshot::build(
        pin,
        beamcraft_engine::synthetic::plate_scene(1, false).unwrap(),
        &Limits::default(),
    )
    .unwrap()
}

fn plan() -> ModelView {
    let id = beamcraft_model::id::Uuid7::parse("019a7bf0-0000-7000-8000-000000000123").unwrap();
    ModelView::new(
        id,
        "Plan",
        WorkPlane::parallel("xy", 3000.0).unwrap(),
        10_000.0,
    )
}

#[test]
fn degenerate_points_make_no_plane() {
    let plane =
        views::three_points([10.0, 20.0, 30.0], [11.0, 20.0, 30.0], [10.0, 20.0, 33.0]).unwrap();
    assert_eq!(plane.normal(), [0.0, -1.0, 0.0]);
    assert_eq!(plane.origin(), [10.0, 20.0, 30.0]);
    assert!(views::three_points([0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]).is_err());
    assert!(views::two_points([0.0; 3], [0.0, 0.0, 10.0]).is_err());
    assert!(views::two_points([0.0; 3], [f64::NAN, 0.0, 0.0]).is_err());
    let section = views::two_points_in_plane(
        [0.0; 3],
        [100.0, 0.0, 0.0],
        WorkPlane::parallel("xz", 0.0).unwrap(),
    )
    .unwrap();
    assert_eq!(section.y(), [0.0, -1.0, 0.0]);
}

#[test]
fn a_visible_pick_obeys_depth_and_hidden_kinds() {
    let snapshot = snapshot();
    let token = snapshot.token();
    let bounds = snapshot.engineering_bounds().unwrap();
    let [ax, ay, _] = bounds.min;
    let [bx, by, bz] = bounds.max;
    let ray = [(ax + bx) / 2.0, (ay + by) / 2.0, bz + 1000.0];
    let down = [0.0, 0.0, -1.0];
    let mut v = plan();
    let hit = views::pick_visible(&snapshot, &v, ray, down, SelectionFilter::ALL).unwrap();
    assert!(hit.is_some());
    assert!(views::surface_plane(&snapshot, Some(&v), ray, down, None).is_ok());
    v.hidden_kinds = KindFilter::SHOW_ALL.hiding(ElementKind::Plate);
    assert!(
        views::pick_visible(&snapshot, &v, ray, down, SelectionFilter::ALL)
            .unwrap()
            .is_none()
    );
    assert!(views::surface_plane(&snapshot, Some(&v), ray, down, None).is_err());
    v.hidden_kinds = KindFilter::SHOW_ALL;
    v.plane = WorkPlane::parallel("xy", bz + 500.0).unwrap();
    v.depth = Some([1.0, 1.0]);
    assert!(
        views::pick_visible(&snapshot, &v, ray, down, SelectionFilter::ALL)
            .unwrap()
            .is_none(),
        "the plate lies outside the thin slab"
    );
    assert_eq!(snapshot.token(), token, "views never change the snapshot");
}

#[test]
fn a_surface_view_lies_on_the_face_and_a_part_view_frames_the_part() {
    let snapshot = snapshot();
    let bounds = snapshot.engineering_bounds().unwrap();
    let [ax, ay, _] = bounds.min;
    let [bx, by, bz] = bounds.max;
    let ray = [(ax + bx) / 2.0, (ay + by) / 2.0, bz + 1000.0];
    let face = views::surface_plane(&snapshot, None, ray, [0.0, 0.0, -1.0], None).unwrap();
    assert!((face.origin()[2] - bz).abs() < 1e-6);
    assert!((face.normal()[2].abs() - 1.0).abs() < 1e-9);
    let part = snapshot.elements().first().unwrap().id;
    for side in [
        PartSide::Front,
        PartSide::Top,
        PartSide::End,
        PartSide::Spatial,
    ] {
        let (plane, span) = views::part_plane(&snapshot, &part, side).unwrap();
        assert!(span >= 1.0);
        let centre = plane.origin();
        assert!(centre[0] >= ax && centre[0] <= bx, "{side:?}");
    }
    let (group, _) = views::group_plane(&snapshot, &[part], PartSide::Top).unwrap();
    assert!((group.normal()[2].abs() - 1.0).abs() < 1e-9);
    assert!(views::group_plane(&snapshot, &[], PartSide::Top).is_err());
}
