// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Atomic T03 operations through fresh real CLI processes.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn ok(f: &StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c} {a:?}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
fn plate(f: &StoreFixture, x: &str) -> String {
    text(
        ok(
            f,
            "create-plate",
            &[
                "--width",
                "100",
                "--length",
                "50",
                "--thickness",
                "10",
                "--translation",
                x,
            ],
        )
        .get("plate")
        .unwrap(),
        "element_id",
    )
}
fn plates(f: &StoreFixture) -> Vec<Json> {
    ok(f, "inspect-elements", &[])
        .get("plates")
        .unwrap()
        .array()
        .unwrap()
        .to_vec()
}
fn numbers(j: &Json) -> Vec<f64> {
    j.array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}
const OP: &str = "019a7bf0-0000-7000-8000-000000800001";
#[test]
fn move_and_rotate_keep_local_bodies_and_retry_uses_recorded_arguments() {
    let f = StoreFixture::new("group-rigid");
    let a = plate(&f, "10,0,0");
    let b = plate(&f, "30,0,0");
    let ids = format!("{a},{b}");
    let before = plates(&f);
    let args = ["--ids", &ids, "--vector", "5,0,0", "--op-id", OP];
    ok(&f, "move-elements", &args);
    let after = plates(&f);
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(old.get("mesh_digest"), new.get("mesh_digest"));
        assert_eq!(old.get("quantities"), new.get("quantities"));
    }
    ok(
        &f,
        "rotate-elements",
        &[
            "--ids",
            &ids,
            "--center",
            "0,0,0",
            "--axis",
            "0,0,1",
            "--angle",
            "1.5707963267948966",
        ],
    );
    let rotated = plates(&f);
    assert!(
        (numbers(
            rotated[0]
                .get("placement")
                .unwrap()
                .get("translation_mm")
                .unwrap()
        )[1] - 15.)
            .abs()
            < 1e-9
    );
    let head = ok(&f, "open-revision", &[]);
    assert_eq!(
        text(&ok(&f, "move-elements", &args), "outcome"),
        "transform_replayed"
    );
    assert_eq!(ok(&f, "open-revision", &[]), head);
    let changed = ["--ids", &ids, "--vector", "6,0,0", "--op-id", OP];
    assert_eq!(f.run("move-elements", &changed).0, 2);
    assert_eq!(ok(&f, "open-revision", &[]), head);
}
#[test]
fn mixed_invalid_group_and_overflow_publish_nothing() {
    let f = StoreFixture::new("group-refusal");
    let a = plate(&f, "10,0,0");
    let head = ok(&f, "open-revision", &[]);
    for args in [
        vec![
            "--ids".to_owned(),
            format!("{a},019a7bf0-0000-7000-8000-000000ffffff"),
            "--vector".into(),
            "1,0,0".into(),
        ],
        vec![
            "--ids".into(),
            format!("{a},{a}"),
            "--vector".into(),
            "1,0,0".into(),
        ],
        vec![
            "--ids".into(),
            a.clone(),
            "--vector".into(),
            "1000000000,0,0".into(),
        ],
    ] {
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(f.run("move-elements", &args).0, 2);
        assert_eq!(ok(&f, "open-revision", &[]), head);
    }
}
#[test]
fn independent_and_linked_copy_groups_are_one_undo_unit() {
    let published = StoreFixture::new("group-copy");
    let a = plate(&published, "0,0,0");
    let b = plate(&published, "100,0,0");
    let ids = format!("{a},{b}");
    let workspace = published.root.join("draft");
    ok(
        &published,
        "create-workspace",
        &["--workspace-dir", workspace.to_str().unwrap()],
    );
    let run = |c: &str, args: &[&str]| {
        let (code, j) = StoreFixture::run_in(workspace.to_str().unwrap(), c, args);
        assert_eq!(code, 0, "{c}: {j:?}");
        j
    };
    let j = run(
        "copy-elements",
        &[
            "--ids",
            &ids,
            "--vector",
            "0,100,0",
            "--instance",
            "true",
            "--op-id",
            OP,
        ],
    );
    assert_eq!(j.get("elements").unwrap().array().unwrap().len(), 2);
    assert_eq!(
        run("inspect-elements", &[])
            .get("plates")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        4
    );
    let head = run("open-revision", &[]);
    run(
        "copy-elements",
        &[
            "--ids",
            &ids,
            "--vector",
            "0,100,0",
            "--instance",
            "true",
            "--op-id",
            OP,
        ],
    );
    assert_eq!(run("open-revision", &[]), head);
    let undo = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args([
            "undo-workspace",
            "--workspace-dir",
            workspace.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stdout)
    );
    assert_eq!(
        run("inspect-elements", &[])
            .get("plates")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        2
    );
    run(
        "copy-elements",
        &["--ids", &ids, "--from", "0,0,0", "--to", "100,100,0"],
    );
    assert_eq!(
        run("inspect-elements", &[])
            .get("plates")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        4
    );
}
