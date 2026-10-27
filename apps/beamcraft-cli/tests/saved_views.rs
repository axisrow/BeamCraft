// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! W01 persistence across processes, personal isolation and optimistic concurrency.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn ok(f: &StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c} {a:?}: {j:?}");
    j
}
fn run(dir: &std::path::Path, c: &str, a: &[&str]) -> (i32, Json) {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg(c)
        .arg("--workspace-dir")
        .arg(dir)
        .args(a)
        .output()
        .unwrap();
    (
        o.status.code().unwrap(),
        beamcraft_json::decode(&o.stdout, &beamcraft_json::Limits::DEFAULT).unwrap(),
    )
}
fn success(dir: &std::path::Path, c: &str, a: &[&str]) -> Json {
    let (code, j) = run(dir, c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
#[test]
fn save_update_restore_and_delete_never_change_model_or_publish_personal_state() {
    let f = StoreFixture::new("personal-views");
    ok(
        &f,
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    let dir = f.root.join("draft");
    ok(
        &f,
        "create-workspace",
        &["--workspace-dir", dir.to_str().unwrap()],
    );
    let (code, before) = StoreFixture::run_in(dir.to_str().unwrap(), "inspect-elements", &[]);
    assert_eq!(code, 0);
    let saved = success(
        &dir,
        "save-view",
        &[
            "--name",
            "Plan",
            "--position",
            "0,0,1000",
            "--comment",
            "hello\nworld",
            "--expected-views",
            "none",
        ],
    );
    let view = saved.get("view").unwrap();
    let id = text(view, "id");
    let version = text(&saved, "version");
    assert!(
        std::fs::read_to_string(dir.join("views.usda"))
            .unwrap()
            .contains("def Camera")
    );
    assert_eq!(
        success(&dir, "restore-view", &["--id", &id])
            .get("view")
            .unwrap(),
        view
    );
    let stale = run(
        &dir,
        "update-view",
        &["--id", &id, "--width", "200", "--expected-views", "none"],
    );
    assert_eq!(stale.0, 2);
    assert_eq!(text(&stale.1, "code"), "STALE_WORKSPACE");
    success(
        &dir,
        "update-view",
        &["--id", &id, "--width", "200", "--expected-views", &version],
    );
    assert_eq!(
        success(&dir, "list-views", &[])
            .get("views")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        1
    );
    let (code, after) = StoreFixture::run_in(dir.to_str().unwrap(), "inspect-elements", &[]);
    assert_eq!(code, 0);
    assert_eq!(before, after);
    let (code, _) = StoreFixture::run_in(
        dir.to_str().unwrap(),
        "create-plate",
        &["--width", "20", "--length", "30", "--thickness", "2"],
    );
    assert_eq!(code, 0);
    assert_eq!(
        success(&dir, "restore-view", &["--id", &id])
            .get("model_changed")
            .unwrap(),
        &Json::Bool(true)
    );
    ok(
        &f,
        "publish-workspace",
        &["--workspace-dir", dir.to_str().unwrap()],
    );
    assert!(!f.root.join("store/views.usda").exists());
    success(&dir, "delete-view", &["--id", &id]);
    assert!(
        success(&dir, "list-views", &[])
            .get("views")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(run(&dir, "restore-view", &["--id", &id]).0, 2);
}
#[test]
fn invalid_camera_and_corrupt_documents_are_refused_without_overwriting() {
    let f = StoreFixture::new("view-refusals");
    ok(
        &f,
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    let dir = f.root.join("draft");
    ok(
        &f,
        "create-workspace",
        &["--workspace-dir", dir.to_str().unwrap()],
    );
    for field in [
        ["--width", "NaN"],
        ["--aspect", "0"],
        ["--orientation-xyzw", "0,0,0,0"],
    ] {
        assert_eq!(
            run(&dir, "save-view", &["--name", "bad", field[0], field[1]]).0,
            2
        );
        assert!(!dir.join("views.usda").exists());
    }
    success(&dir, "save-view", &["--name", "good"]);
    let path = dir.join("views.usda");
    std::fs::write(&path, b"#usda 1.0\ninvalid").unwrap();
    let original = std::fs::read(&path).unwrap();
    assert_ne!(run(&dir, "save-view", &["--name", "replacement"]).0, 0);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_ne!(
        run(&f.root.join("store"), "save-view", &["--name", "shared"]).0,
        0
    );
}
