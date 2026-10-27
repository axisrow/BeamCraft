// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.3 independently checked identity preservation and removal dependency decisions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use beamcraft_engine::grid_update::update;
use beamcraft_model::{
    error::ErrorCode,
    grid::{Grid, GridFamily, RegularGridFamily},
    grid_update::{RegularGridPatch, UpdateGrid},
    id::ElementId,
};
use std::collections::BTreeSet;
fn id(n: u8) -> ElementId {
    ElementId::parse(&format!("01990000-0000-7000-8000-{n:012x}")).unwrap()
}
fn source() -> Grid {
    Grid {
        id: id(1),
        name: "Regular".to_owned(),
        translation_mm: [0.0; 2],
        orientation_zw: [0.0, 1.0],
        u: GridFamily::Regular(RegularGridFamily {
            expression: "0 2*6000".to_owned(),
            labels: vec!["A".into(), "B".into(), "C".into()],
            axis_ids: vec![id(2), id(3), id(4)],
            span_mm: [-2000.0, 2000.0],
            positions_mm: vec![0.0, 6000.0, 12000.0],
        }),
        v: GridFamily::Drawn(Vec::new()),
    }
}
fn request(expression: &str, labels: &[&str]) -> UpdateGrid {
    UpdateGrid {
        target_id: id(1),
        name: None,
        origin_mm: None,
        orientation_zw: None,
        u: Some(RegularGridPatch {
            expression: Some(expression.to_owned()),
            labels: Some(labels.iter().map(|s| s.to_string()).collect()),
            new_axis_ids: Vec::new(),
            span_mm: None,
        }),
        v: None,
        fit_spans: false,
    }
}
#[test]
fn moving_one_regular_axis_retains_its_id_and_the_other_axes() {
    let source = source();
    let before = source.clone();
    let result = update(
        &request("0 6500 5500", &["A", "B", "C"]),
        &source,
        &BTreeSet::new(),
        &BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(source, before);
    assert_eq!(
        result.grid.axis_ids().collect::<Vec<_>>(),
        [id(2), id(3), id(4)]
    );
    let GridFamily::Regular(f) = result.grid.u else {
        panic!()
    };
    assert_eq!(f.positions_mm, [0.0, 6500.0, 12000.0]);
    assert_eq!(f.span_mm, [-2000.0, 2000.0]);
}
#[test]
fn only_removed_referenced_axes_block_the_update() {
    let source = source();
    let before = source.clone();
    let request = request("0 12000", &["A", "C"]);
    assert_eq!(
        update(
            &request,
            &source,
            &BTreeSet::new(),
            &BTreeSet::from([id(3)])
        )
        .unwrap_err()
        .code,
        ErrorCode::DependencyConflict
    );
    assert_eq!(source, before);
    let kept = update(
        &request,
        &source,
        &BTreeSet::new(),
        &BTreeSet::from([id(2)]),
    )
    .unwrap();
    assert_eq!(kept.grid.axis_ids().collect::<Vec<_>>(), [id(2), id(4)]);
}

#[test]
fn fitting_regular_spans_covers_the_complete_new_rectangle_without_changing_axes() {
    let mut source = source();
    source.v = GridFamily::Regular(RegularGridFamily {
        expression: "0 2*5000".into(),
        labels: vec!["1".into(), "2".into(), "3".into()],
        axis_ids: vec![id(5), id(6), id(7)],
        span_mm: [-2000.0, 14000.0],
        positions_mm: vec![0.0, 5000.0, 10000.0],
    });
    let before = source.clone();
    let mut edit = request("0 2*15000", &["A", "B", "C"]);
    edit.v = Some(RegularGridPatch {
        expression: Some("0 2*25000".into()),
        labels: Some(vec!["1".into(), "2".into(), "3".into()]),
        ..Default::default()
    });
    let old_fingerprint = edit.content_fingerprint().unwrap();
    edit.fit_spans = true;
    assert_ne!(old_fingerprint, edit.content_fingerprint().unwrap());
    let fitted = update(&edit, &source, &BTreeSet::new(), &BTreeSet::new())
        .unwrap()
        .grid;
    assert_eq!(source, before);
    assert_eq!(
        fitted.axis_ids().collect::<Vec<_>>(),
        source.axis_ids().collect::<Vec<_>>()
    );
    let (GridFamily::Regular(u), GridFamily::Regular(v)) = (&fitted.u, &fitted.v) else {
        panic!()
    };
    assert_eq!(u.span_mm, [-2000.0, 52000.0]);
    assert_eq!(v.span_mm, [-2000.0, 32000.0]);
    // Advanced explicit limits remain an intentional override.
    edit.u.as_mut().unwrap().span_mm = Some([-1000.0, 60000.0]);
    let explicit = update(&edit, &source, &BTreeSet::new(), &BTreeSet::new())
        .unwrap()
        .grid;
    let GridFamily::Regular(u) = explicit.u else {
        panic!()
    };
    assert_eq!(u.span_mm, [-1000.0, 60000.0]);
}
