// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic frames and views, MPL-2.0.

//! View construction against evaluated geometry: visible picking, surface and part frames.

#![allow(clippy::unwrap_used)]
use beamcraft_engine::query::pick::SelectionFilter;
use beamcraft_engine::view_clipping::{ClipPlane, Clipping};
use beamcraft_engine::views::{self, ModelView, PartSide};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_engine::{
    ElementKind, KindFilter, Limits, Representation, ViewPin, ViewSettings, ViewSnapshot,
};

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
fn a_display_cut_hides_the_pick_like_the_picture() {
    let snapshot = snapshot();
    let bounds = snapshot.engineering_bounds().unwrap();
    let [ax, ay, _] = bounds.min;
    let [bx, by, bz] = bounds.max;
    let ray = [(ax + bx) / 2.0, (ay + by) / 2.0, bz + 1000.0];
    let down = [0.0, 0.0, -1.0];
    let v = plan();
    let visible = |cut: Option<&Clipping>| {
        views::pick_visible_except(
            &snapshot,
            &v,
            ray,
            down,
            SelectionFilter::ALL,
            &std::collections::BTreeSet::new(),
            cut,
            false,
        )
        .unwrap()
        .is_some()
    };
    let kept = |cut: Option<&Clipping>| {
        views::pick_kept(&snapshot, ray, down, SelectionFilter::ALL, cut, false)
            .unwrap()
            .is_some()
    };
    // The top-face hit sits above the mid plane. Keeping the upper half keeps the hit.
    // Keeping the lower half hides the top face but leaves the plate's section on the plane,
    // which the picture shows, so the click selects the plate there; a cut that keeps
    // nothing of the plate selects nothing, for the view pick and the free pick alike (issue
    // 569 item 2): a click never selects what the cut hides.
    let mid = (bounds.min[2] + bz) / 2.0;
    let keep_above = Clipping {
        planes: vec![ClipPlane::new([0.0, 0.0, mid], [0.0, 0.0, 1.0]).unwrap()],
        area: None,
    };
    let keep_below = Clipping {
        planes: vec![ClipPlane::new([0.0, 0.0, mid], [0.0, 0.0, -1.0]).unwrap()],
        area: None,
    };
    let above_all = Clipping {
        planes: vec![ClipPlane::new([0.0, 0.0, bz + 10.0], [0.0, 0.0, 1.0]).unwrap()],
        area: None,
    };
    assert!(visible(None) && visible(Some(&keep_above)));
    assert!(visible(Some(&keep_below)) && kept(Some(&keep_below)));
    let section = views::pick_kept(
        &snapshot,
        ray,
        down,
        SelectionFilter::ALL,
        Some(&keep_below),
        false,
    )
    .unwrap()
    .unwrap();
    assert!(
        (section.point_world_mm[2] - mid).abs() < 1e-6,
        "the hit lies on the section"
    );
    assert!(!visible(Some(&above_all)) && !kept(Some(&above_all)));
    assert!(kept(None) && kept(Some(&keep_above)));
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
    let hit = views::pick_visible(&snapshot, &v, ray, down, SelectionFilter::ALL, false).unwrap();
    assert!(hit.is_some());
    assert!(views::surface_plane(&snapshot, Some(&v), ray, down, None).is_ok());
    v.hidden_kinds = KindFilter::SHOW_ALL.hiding(ElementKind::Plate);
    assert!(
        views::pick_visible(&snapshot, &v, ray, down, SelectionFilter::ALL, false)
            .unwrap()
            .is_none()
    );
    assert!(views::surface_plane(&snapshot, Some(&v), ray, down, None).is_err());
    v.hidden_kinds = KindFilter::SHOW_ALL;
    v.plane = WorkPlane::parallel("xy", bz + 500.0).unwrap();
    v.depth = Some([1.0, 1.0]);
    assert!(
        views::pick_visible(&snapshot, &v, ray, down, SelectionFilter::ALL, false)
            .unwrap()
            .is_none(),
        "the plate lies outside the thin slab"
    );
    // A slab through the plate's thickness: the click straight down meets the top face
    // above the slab and the bottom face below it, both hidden, and selects the section the
    // slab draws between them (a column clicked in a plan).
    let mid = (bounds.min[2] + bz) / 2.0;
    v.plane = WorkPlane::parallel("xy", mid).unwrap();
    v.depth = Some([0.5, 0.5]);
    let hit = views::pick_visible(&snapshot, &v, ray, down, SelectionFilter::ALL, false)
        .unwrap()
        .expect("the cut section is picked");
    assert!((hit.point_world_mm[2] - (mid + 0.5)).abs() < 1e-6);
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

#[test]
fn a_views_representation_and_kinds_feed_view_settings_without_touching_the_token() {
    let snapshot = snapshot();
    let token = snapshot.token();
    let mut plan = plan();
    plan.representation = Representation::Simplified;
    plan.hidden_kinds = KindFilter::SHOW_ALL.hiding(ElementKind::Reference);
    plan.colour_set = Some("Part classes".into());
    plan.validate().unwrap();
    let settings = ViewSettings {
        hidden_kinds: plan.hidden_kinds,
        representation: plan.representation,
    };
    let projection = snapshot.project(&settings).unwrap();
    assert_eq!(
        projection.token(),
        token,
        "display state never moves the token"
    );
    assert_eq!(projection.settings(), settings);
    assert!(
        projection
            .items()
            .iter()
            .all(|p| p.representation == Representation::Simplified)
    );
    assert_eq!(snapshot.token(), token);
}
