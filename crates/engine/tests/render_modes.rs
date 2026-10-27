// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Pure CPU presentation mode bounds and snapshot invariance.
#![allow(clippy::unwrap_used)]
use beamcraft_engine::{
    Limits, ViewPin, ViewSnapshot,
    render::{self, Mode},
    synthetic,
};
use beamcraft_model::digest::Sha256;
#[test]
fn modes_are_bounded_read_only_queries_over_the_same_snapshot() {
    let snapshot = ViewSnapshot::build(
        ViewPin::new(Sha256::digest(b"render-mode-test").unwrap(), 1),
        synthetic::plate_scene(2000, false).unwrap(),
        &Limits::default(),
    )
    .unwrap();
    let token = snapshot.token().clone();
    let ids: Vec<_> = snapshot.elements().iter().map(|e| e.id).collect();
    let started = std::time::Instant::now();
    for mode in [
        Mode::Wireframe,
        Mode::Transparent,
        Mode::Grey,
        Mode::Shaded,
        Mode::SelectedOnly,
    ] {
        let p = render::query(&snapshot, mode, &ids, &ids).unwrap();
        assert_eq!(p.visible, ids);
    }
    eprintln!(
        "five CPU presentation queries, 2000 elements: {:?}",
        started.elapsed()
    );
    assert!(
        render::query(&snapshot, Mode::SelectedOnly, &[], &ids)
            .unwrap()
            .visible
            .is_empty()
    );
    assert!(
        render::query(
            &snapshot,
            Mode::Shaded,
            &vec![ids.first().copied().unwrap(); 65_537],
            &ids
        )
        .is_err()
    );
    assert_eq!(snapshot.token(), &token);
}
