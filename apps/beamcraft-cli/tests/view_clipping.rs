// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V15 exact transport, conservative projection and engineering isolation.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn personal(dir: &std::path::Path, c: &str, a: &[&str]) -> Json {
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
fn command(dir: &std::path::Path, c: &str, a: &[&str]) -> Json {
    let (code, j) = StoreFixture::run_in(dir.to_str().unwrap(), c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn export(dir: &std::path::Path, id: Option<&str>) -> Vec<u8> {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"));
    c.arg("export-view")
        .arg("--store")
        .arg(dir)
        .args(["--out", "-"]);
    if let Some(id) = id {
        c.args(["--view", id]);
    }
    let o = c.output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    o.stdout
}
fn sections(bytes: &[u8]) -> (Json, &[u8]) {
    assert_eq!(&bytes[..8], b"BCVXFRM3");
    let mut offset = 8;
    let mut state = Json::Null;
    for i in 0..4 {
        let n = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if i == 3 {
            state = beamcraft_json::decode(
                &bytes[offset..offset + n],
                &beamcraft_json::Limits::DEFAULT,
            )
            .unwrap();
        }
        offset += n;
    }
    (state, &bytes[offset..])
}
#[test]
fn clipping_changes_only_presentation_and_the_legacy_bundle_is_identical() {
    let f = StoreFixture::new("view-clipping");
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
    let before = command(&dir, "inspect-elements", &[]);
    let raw = export(&dir, None);
    let saved = personal(
        &dir,
        "create-view",
        &["--name", "Plan", "--kind", "plane", "--depth", "100"],
    );
    let id = saved
        .get("view")
        .unwrap()
        .get("id")
        .unwrap()
        .string()
        .unwrap();
    personal(
        &dir,
        "set-view-clipping",
        &[
            "--id",
            id,
            "--planes",
            "200,0,0,1,0,0",
            "--work-area",
            "-500,-500,-500,500,500,500",
        ],
    );
    let framed = export(&dir, Some(id));
    let (state, bundle) = sections(&framed);
    assert_eq!(bundle, raw);
    assert!(
        state
            .get("visible_elements")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(state.get("clip_planes").unwrap().array().unwrap().len(), 9);
    personal(&dir, "clear-view-clipping", &["--id", id]);
    let framed = export(&dir, Some(id));
    let (state, bundle) = sections(&framed);
    assert_eq!(bundle, raw);
    assert_eq!(
        state
            .get("visible_elements")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        state
            .get("clip_planes")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(command(&dir, "inspect-elements", &[]), before);
    let path = f.root.join("presented.bin");
    let result = command(
        &dir,
        "export-view",
        &["--view", id, "--out", path.to_str().unwrap()],
    );
    assert_ne!(result.get("presentation").unwrap(), &Json::Null);
    assert_eq!(sections(&std::fs::read(path).unwrap()).1, raw);
    let csv = f.root.join("bom.csv");
    command(&dir, "export-bom", &["--out", csv.to_str().unwrap()]);
    assert_eq!(std::fs::read_to_string(csv).unwrap().lines().count(), 2);
}
