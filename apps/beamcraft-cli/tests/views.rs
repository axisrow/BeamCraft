// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V12 named-view acceptance: saving, listing, applying and removing views through the real
//! binary. A view is display state: the model head proves itself unchanged across every view
//! operation, and an applied view returns exactly the pose that was saved.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use support::{at, text};

fn ok(store: &str, c: &str, a: &[&str]) -> Json {
    let (code, j) = support::StoreFixture::run_in(store, c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}

fn reject(store: &str, c: &str, a: &[&str]) {
    let (code, j) = support::StoreFixture::run_in(store, c, a);
    assert_eq!(code, 2, "{c}: {j:?}");
    assert_eq!(text(&j, "outcome"), "rejected");
}

#[test]
fn named_views_save_apply_and_remove_without_touching_the_store() {
    let f = support::StoreFixture::new("v12-views");
    let store = f.root.join("store").to_str().unwrap().to_owned();
    let views = f.root.join("views.json");
    let file = views.to_str().unwrap();
    ok(
        &store,
        "create-grid",
        &[
            "--name",
            "G",
            "--u-expression",
            "0 2*6000",
            "--v-expression",
            "0 7500 4500",
        ],
    );
    let head = ok(&store, "open-revision", &[]);

    // A grid-line plane view resolves once, against the read-only store.
    let saved = ok(
        &store,
        "save-view",
        &[
            "--file",
            file,
            "--name",
            "Elev A",
            "--plane-grid",
            "G",
            "--plane-line",
            "A",
            "--centre",
            "0,0,3000",
            "--distance",
            "25000",
        ],
    );
    assert_eq!(text(&saved, "outcome"), "view_saved");
    let pose = at(&saved, &["pose"]);
    // Line A is the first v line: local (0,0) to (12000,0), so the target is its world midpoint
    // at the centre height and the level view faces the plane normal (+y).
    let target = at(pose, &["target_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(target, [6000.0, 0.0, 3000.0]);
    assert_eq!(at(pose, &["pitch"]).float().unwrap(), 0.0);
    assert!((at(pose, &["yaw"]).float().unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    assert_eq!(at(pose, &["distance_mm"]).float().unwrap(), 25000.0);

    // An orbit view carries its pose and a work area exactly as given.
    ok(
        &store,
        "save-view",
        &[
            "--file",
            file,
            "--name",
            "Steel 3D",
            "--target",
            "100,-200,3000",
            "--yaw",
            "0.7",
            "--pitch",
            "0.85",
            "--distance",
            "18000",
            "--ortho",
            "true",
            "--work-area",
            "0,0,0,12000,7500,6000",
        ],
    );
    let applied = ok(
        &store,
        "apply-view",
        &["--file", file, "--name", "Steel 3D"],
    );
    assert_eq!(text(&applied, "outcome"), "view_applied");
    // The applied pose is the saved pose, field for field.
    let pose = at(&applied, &["pose"]);
    assert_eq!(
        at(pose, &["target_mm"])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [100.0, -200.0, 3000.0]
    );
    assert_eq!(at(pose, &["yaw"]).float().unwrap(), 0.7);
    assert_eq!(at(pose, &["pitch"]).float().unwrap(), 0.85);
    assert_eq!(at(pose, &["distance_mm"]).float().unwrap(), 18000.0);
    assert!(at(pose, &["orthographic"]).boolean().unwrap());
    let wa = at(&applied, &["work_area"]);
    assert_eq!(
        at(wa, &["max"])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [12000.0, 7500.0, 6000.0]
    );

    // Both projections and both kinds list in name order, and the level view resolves.
    let listed = ok(&store, "list-views", &["--file", file]);
    let names = support::list(&listed, "views")
        .iter()
        .map(|v| text(v, "name"))
        .collect::<Vec<_>>();
    assert_eq!(names, ["Elev A", "Steel 3D"]);
    assert_eq!(at(&listed, &["view_count"]).unsigned().unwrap(), 2);
    let level = ok(
        &store,
        "save-view",
        &[
            "--file",
            file,
            "--name",
            "Plan",
            "--plane-level",
            "true",
            "--centre",
            "6000,6000,3000",
            "--distance",
            "30000",
        ],
    );
    let level_pose = at(&level, &["pose"]);
    assert_eq!(at(level_pose, &["pitch"]).float().unwrap(), 1.52);
    // The eye stands at the first point and looks at the second, so the yaw faces -x.
    let two = ok(
        &store,
        "save-view",
        &[
            "--file",
            file,
            "--name",
            "Along",
            "--from",
            "0,0,0",
            "--to",
            "6000,0,0",
            "--distance",
            "12000",
        ],
    );
    assert!(
        (at(at(&two, &["pose"]), &["yaw"]).float().unwrap() - std::f64::consts::PI).abs() < 1e-9
    );

    // The model head is byte-identical across every view operation.
    assert_eq!(ok(&store, "open-revision", &[]), head);

    // Removal deletes exactly the named view; unknown names and mixed modes refuse.
    let removed = ok(&store, "remove-view", &["--file", file, "--name", "Along"]);
    assert_eq!(at(&removed, &["view_count"]).unsigned().unwrap(), 3);
    assert_eq!(ok(&store, "open-revision", &[]), head);
    reject(&store, "apply-view", &["--file", file, "--name", "Along"]);
    reject(
        &store,
        "save-view",
        &[
            "--file",
            file,
            "--name",
            "Bad",
            "--plane-level",
            "true",
            "--target",
            "0,0,0",
            "--yaw",
            "0",
            "--pitch",
            "0.5",
            "--distance",
            "1000",
        ],
    );
    reject(&store, "apply-view", &["--file", file, "--name", "Missing"]);
    assert_eq!(ok(&store, "open-revision", &[]), head);
    let final_list = ok(&store, "list-views", &["--file", file]);
    assert_eq!(at(&final_list, &["view_count"]).unsigned().unwrap(), 3);
}
