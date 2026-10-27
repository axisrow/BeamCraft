// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V17 ordered appearance sets, original palette, overlays and private CAS.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn model(dir: &std::path::Path, c: &str, a: &[&str]) -> Json {
    let (code, j) = StoreFixture::run_in(dir.to_str().unwrap(), c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn personal(dir: &std::path::Path, c: &str, a: &[&str]) -> (i32, Json) {
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
fn ok(dir: &std::path::Path, c: &str, a: &[&str]) -> Json {
    let (code, j) = personal(dir, c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
fn file(f: &StoreFixture, s: &str) -> String {
    let p = f.root.join("rule.json");
    std::fs::write(&p, s).unwrap();
    p.to_str().unwrap().to_owned()
}
const FILTER: &str = "019a7bf0-0000-7000-8000-000000f00001";
const SET: &str = "019a7bf0-0000-7000-8000-000000f00002";
fn export(dir: &std::path::Path, args: &[&str]) -> (Json, Vec<u8>) {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg("export-view")
        .arg("--store")
        .arg(dir)
        .args(["--out", "-"])
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{:?}", o.stdout);
    let b = &o.stdout;
    assert_eq!(&b[..8], b"BCVXFRM3");
    let mut offset = 8;
    for _ in 0..3 {
        let n = u32::from_le_bytes(b[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4 + n;
    }
    let n = u32::from_le_bytes(b[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4;
    (
        beamcraft_json::decode(&b[offset..offset + n], &beamcraft_cli::display::LIMITS).unwrap(),
        b[offset + n..].to_vec(),
    )
}
#[test]
fn rule_order_palette_overlay_and_dependency_guards_are_read_only() {
    let f = StoreFixture::new("representation-sets");
    let (code, p) = f.run(
        "create-plate",
        &[
            "--width",
            "100",
            "--length",
            "50",
            "--thickness",
            "10",
            "--class",
            "1",
        ],
    );
    assert_eq!(code, 0, "{p:?}");
    let a = text(p.get("plate").unwrap(), "element_id");
    let root = f.root.join("store");
    let dir = f.root.join("draft");
    model(
        &root,
        "create-workspace",
        &["--workspace-dir", dir.to_str().unwrap()],
    );
    let before = model(&dir, "inspect-elements", &[]);
    let path = file(&f, r#"{"all":[]}"#);
    ok(
        &dir,
        "save-filter",
        &["--id", FILTER, "--name", "All", "--rule", &path],
    );
    let rules = format!(
        r#"{{"by_class":true,"rules":[{{"filter":"{FILTER}","color":[1,0,0],"transparency":0.2}},{{"filter":"{FILTER}","color":[0,1,0],"transparency":0.8}}]}}"#
    );
    let path = file(&f, &rules);
    ok(
        &dir,
        "save-representation-set",
        &["--id", SET, "--name", "Ordered", "--rules", &path],
    );
    let view = ok(&dir, "create-view", &["--kind", "3d", "--name", "Styled"]);
    let id = text(view.get("view").unwrap(), "id");
    ok(&dir, "update-view", &["--id", &id, "--representation", SET]);
    let personal_before = std::fs::read(dir.join("views.usda")).unwrap();
    let (j, bundle) = export(&dir, &["--view", &id]);
    let styles = j.get("appearances").unwrap().array().unwrap();
    assert_eq!(styles.len(), 1);
    assert_eq!(
        styles[0].get("color").unwrap().array().unwrap()[0]
            .float()
            .unwrap(),
        1.0
    );
    assert_eq!(styles[0].get("transparency").unwrap().float().unwrap(), 0.2);
    assert_ne!(personal(&dir, "delete-filter", &["--id", FILTER]).0, 0);
    assert_ne!(
        personal(&dir, "delete-representation-set", &["--id", SET]).0,
        0
    );
    let overlay = file(
        &f,
        &format!(r#"[{{"id":"{a}","color":[0,0,1],"transparency":0.9}}]"#),
    );
    let (over, b) = export(
        &dir,
        &[
            "--view",
            &id,
            "--overlay",
            &overlay,
            "--render-mode",
            "transparent",
        ],
    );
    assert_eq!(b, bundle);
    let style = &over.get("appearances").unwrap().array().unwrap()[0];
    assert_eq!(style.get("transparency").unwrap().float().unwrap(), 0.9);
    assert_eq!(
        style.get("color").unwrap().array().unwrap()[2]
            .float()
            .unwrap(),
        1.0
    );
    assert_eq!(
        std::fs::read(dir.join("views.usda")).unwrap(),
        personal_before
    );
    assert_eq!(model(&dir, "inspect-elements", &[]), before);
    let (again, _) = export(&dir, &["--view", &id]);
    assert_eq!(again, j);
    let palette = file(&f, r#"{"by_class":true,"rules":[]}"#);
    ok(
        &dir,
        "save-representation-set",
        &["--id", SET, "--name", "Palette", "--rules", &palette],
    );
    let (p, _) = export(&dir, &["--representation", SET]);
    let color = p.get("appearances").unwrap().array().unwrap()[0]
        .get("color")
        .unwrap()
        .array()
        .unwrap();
    assert_eq!(
        color
            .iter()
            .map(|c| (c.float().unwrap() * 255.0).round() as u8)
            .collect::<Vec<_>>(),
        beamcraft_engine::class_palette::DEFAULT_CLASS_PALETTE[1]
    );
    ok(&dir, "inspect-representation-set", &["--id", SET]);
    ok(&dir, "list-representation-sets", &[]);
    ok(
        &dir,
        "update-view",
        &["--id", &id, "--representation", "none"],
    );
    ok(&dir, "delete-representation-set", &["--id", SET]);
    ok(&dir, "delete-filter", &["--id", FILTER]);
}
#[test]
fn invalid_channels_unknown_rules_and_stale_private_versions_never_write() {
    let f = StoreFixture::new("representation-refusals");
    let (code, p) = f.run(
        "create-plate",
        &["--width", "10", "--length", "20", "--thickness", "3"],
    );
    assert_eq!(code, 0, "{p:?}");
    let root = f.root.join("store");
    let dir = f.root.join("draft");
    model(
        &root,
        "create-workspace",
        &["--workspace-dir", dir.to_str().unwrap()],
    );
    let path = file(&f, r#"{"all":[]}"#);
    ok(
        &dir,
        "save-filter",
        &["--id", FILTER, "--name", "All", "--rule", &path],
    );
    let before = std::fs::read(dir.join("views.usda")).unwrap();
    for body in [
        format!(
            r#"{{"by_class":false,"rules":[{{"filter":"{FILTER}","color":[2,0,0],"transparency":0}}]}}"#
        ),
        format!(
            r#"{{"by_class":false,"rules":[{{"filter":"{SET}","color":[1,0,0],"transparency":0}}]}}"#
        ),
        r#"{"by_class":false,"rules":[],"extra":0}"#.into(),
    ] {
        let path = file(&f, &body);
        assert_ne!(
            personal(
                &dir,
                "save-representation-set",
                &["--id", SET, "--name", "Bad", "--rules", &path]
            )
            .0,
            0
        );
        assert_eq!(std::fs::read(dir.join("views.usda")).unwrap(), before);
    }
    let path = file(&f, r#"{"by_class":false,"rules":[]}"#);
    assert_ne!(
        personal(
            &dir,
            "save-representation-set",
            &[
                "--id",
                SET,
                "--name",
                "Stale",
                "--rules",
                &path,
                "--expected-views",
                "none"
            ]
        )
        .0,
        0
    );
    let overlay = file(
        &f,
        r#"[{"id":"019a7bf0-0000-7000-8000-000000ffffff","color":[1,0,0],"transparency":0}]"#,
    );
    let out = f.root.join("refused.bin");
    let (code, _) = StoreFixture::run_in(
        dir.to_str().unwrap(),
        "export-view",
        &["--overlay", &overlay, "--out", out.to_str().unwrap()],
    );
    assert_ne!(code, 0);
    assert!(!out.exists());
    assert_eq!(std::fs::read(dir.join("views.usda")).unwrap(), before);
}
