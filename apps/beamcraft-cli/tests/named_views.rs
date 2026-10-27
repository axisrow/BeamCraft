// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V12 model anchors re-resolve by identity and unavailable sources remain explicit.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn view(dir: &std::path::Path, c: &str, a: &[&str]) -> Json {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg(c)
        .arg("--workspace-dir")
        .arg(dir)
        .args(a)
        .output()
        .unwrap();
    let j = beamcraft_json::decode(&o.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    assert!(o.status.success(), "{c}: {j:?}");
    j
}
fn model(dir: &std::path::Path, c: &str, a: &[&str]) -> Json {
    let (code, j) = StoreFixture::run_in(dir.to_str().unwrap(), c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
const GRID: &str = "019a7bf0-0000-7000-8000-000000c00001";
const AXIS: &str = "019a7bf0-0000-7000-8000-000000c00002";
const LEVEL: &str = "019a7bf0-0000-7000-8000-000000c00003";
#[test]
fn grid_level_and_three_d_views_restore_and_missing_anchors_are_unresolved() {
    let f = StoreFixture::new("named-views");
    assert_eq!(
        f.run(
            "create-plate",
            &["--width", "100", "--length", "50", "--thickness", "10"]
        )
        .0,
        0
    );
    let dir = f.root.join("draft");
    assert_eq!(
        f.run(
            "create-workspace",
            &["--workspace-dir", dir.to_str().unwrap()]
        )
        .0,
        0
    );
    model(
        &dir,
        "create-grid",
        &[
            "--name",
            "Grid",
            "--new-id",
            GRID,
            "--u-expression",
            "0",
            "--u-axis-ids",
            AXIS,
        ],
    );
    model(
        &dir,
        "create-level",
        &["--name", "Level", "--elevation", "3000", "--new-id", LEVEL],
    );
    let g = view(
        &dir,
        "create-view",
        &[
            "--name",
            "Section",
            "--kind",
            "grid-line",
            "--grid",
            "Grid",
            "--axis",
            AXIS,
            "--work-area",
            "-100,-100,-100,100,100,100",
        ],
    );
    let gid = text(g.get("view").unwrap(), "id");
    let l = view(
        &dir,
        "create-view",
        &["--name", "Plan", "--kind", "level", "--level", LEVEL],
    );
    let lid = text(l.get("view").unwrap(), "id");
    view(&dir, "create-view", &["--name", "3D", "--kind", "3d"]);
    let before = view(&dir, "restore-view", &["--id", &gid]);
    model(&dir, "update-grid", &["--id", GRID, "--origin", "1000,0"]);
    let after = view(&dir, "restore-view", &["--id", &gid]);
    let x = |j: &Json| {
        j.get("view")
            .unwrap()
            .get("work_plane")
            .unwrap()
            .array()
            .unwrap()[0]
            .array()
            .unwrap()[0]
            .float()
            .unwrap()
    };
    assert!((x(&after) - x(&before) - 1000.).abs() < 1e-8);
    assert_eq!(after.get("status").unwrap().string().unwrap(), "resolved");
    assert_ne!(after.get("work_area").unwrap(), &Json::Null);
    view(
        &dir,
        "update-view",
        &[
            "--id",
            &gid,
            "--side",
            "negative",
            "--depth",
            "500",
            "--work-area",
            "none",
        ],
    );
    let stored = view(&dir, "inspect-view", &["--id", &gid]);
    let restored = view(&dir, "restore-view", &["--id", &gid]);
    assert_eq!(stored.get("view"), restored.get("view"));
    assert_eq!(restored.get("work_area").unwrap(), &Json::Null);
    let personal = std::fs::read(dir.join("views.usda")).unwrap();
    model(&dir, "delete-element", &["--id", GRID]);
    model(&dir, "delete-element", &["--id", LEVEL]);
    for id in [&gid, &lid] {
        let r = view(&dir, "restore-view", &["--id", id]);
        assert_eq!(text(&r, "status"), "unresolved");
        assert_eq!(text(&r, "outcome"), "view_unresolved");
    }
    assert_eq!(std::fs::read(dir.join("views.usda")).unwrap(), personal);
    assert_eq!(
        view(&dir, "list-views", &[])
            .get("views")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        3
    );
}
