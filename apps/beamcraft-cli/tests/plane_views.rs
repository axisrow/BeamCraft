// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V09 constructed cameras are reproducible personal state, not model edits.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn run(dir: &std::path::Path, c: &str, args: &[&str]) -> (i32, Json) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg(c)
        .arg("--workspace-dir")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap(),
    )
}
fn ok(dir: &std::path::Path, c: &str, args: &[&str]) -> Json {
    let (code, j) = run(dir, c, args);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
#[test]
fn plane_and_two_point_cameras_restore_their_frames_and_depth() {
    let f = StoreFixture::new("plane-views");
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
    let before = StoreFixture::run_in(dir.to_str().unwrap(), "inspect-elements", &[]).1;
    for args in [
        vec!["--kind", "plane", "--plane", "10,20,30,1,0,0,0,0,1"],
        vec![
            "--kind",
            "two-points",
            "--start",
            "10,20,30",
            "--end",
            "110,20,60",
            "--side",
            "negative",
        ],
    ] {
        let mut a = vec!["--name", "Section", "--depth", "1000", "--width", "2000"];
        a.extend(args);
        let created = ok(&dir, "create-view", &a);
        let id = text(created.get("view").unwrap(), "id");
        let restored = ok(&dir, "restore-view", &["--id", &id]);
        assert_eq!(created.get("view"), restored.get("view"));
        assert_eq!(created.get("construction"), restored.get("construction"));
        ok(&dir, "update-view", &["--id", &id, "--width", "3000"]);
        let restored = ok(&dir, "restore-view", &["--id", &id]);
        assert_eq!(
            restored
                .get("view")
                .unwrap()
                .get("camera")
                .unwrap()
                .get("width_mm")
                .unwrap()
                .float()
                .unwrap(),
            3000.
        );
    }
    assert_eq!(
        ok(&dir, "list-views", &[])
            .get("views")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        StoreFixture::run_in(dir.to_str().unwrap(), "inspect-elements", &[]).1,
        before
    );
    let bytes = std::fs::read(dir.join("views.usda")).unwrap();
    assert_eq!(
        run(
            &dir,
            "create-view",
            &[
                "--name",
                "bad",
                "--kind",
                "two-points",
                "--start",
                "0,0,0",
                "--end",
                "0,0,100"
            ]
        )
        .0,
        2
    );
    assert_eq!(std::fs::read(dir.join("views.usda")).unwrap(), bytes);
}
