// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V16 real CLI modes and explicit selection leave engineering bytes unchanged.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn export(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg("export-view")
        .arg("--store")
        .arg(root)
        .args(["--out", "-"])
        .args(args)
        .output()
        .unwrap()
}
fn frame(bytes: &[u8]) -> (Json, &[u8]) {
    assert_eq!(&bytes[..8], b"BCVXFRM3");
    let mut offset = 8;
    for _ in 0..3 {
        let n = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4 + n;
    }
    let n = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4;
    (
        beamcraft_json::decode(&bytes[offset..offset + n], &beamcraft_cli::display::LIMITS)
            .unwrap(),
        &bytes[offset + n..],
    )
}
#[test]
fn modes_selection_and_refusals_preserve_the_full_bundle() {
    let f = StoreFixture::new("render-modes");
    let (code, p) = f.run(
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    assert_eq!(code, 0, "{p:?}");
    let id = p
        .get("plate")
        .unwrap()
        .get("element_id")
        .unwrap()
        .string()
        .unwrap();
    let root = f.root.join("store");
    let raw = export(&root, &[]);
    assert!(raw.status.success());
    for mode in [
        "wireframe",
        "transparent",
        "grey",
        "shaded",
        "selected-only",
    ] {
        let o = export(&root, &["--render-mode", mode, "--select", id]);
        assert!(o.status.success(), "{:?}", o.stdout);
        let (j, b) = frame(&o.stdout);
        assert_eq!(b, raw.stdout);
        let r = j.get("render").unwrap();
        assert_eq!(r.get("mode").unwrap().string().unwrap(), mode);
        assert_eq!(j.get("visible_elements").unwrap().array().unwrap().len(), 1);
        assert_eq!(
            r.get("surfaces").unwrap().boolean().unwrap(),
            mode != "wireframe"
        );
        assert_eq!(
            r.get("lit").unwrap().boolean().unwrap(),
            !matches!(mode, "wireframe" | "grey")
        );
        assert_eq!(
            r.get("transparency").unwrap().float().unwrap(),
            if mode == "transparent" { 0.65 } else { 0.0 }
        );
    }
    let o = export(&root, &["--render-mode", "selected-only"]);
    let (j, b) = frame(&o.stdout);
    assert_eq!(b, raw.stdout);
    assert!(
        j.get("visible_elements")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
    let path = f.root.join("selection.json");
    std::fs::write(&path, format!("[\"{id}\",\"{id}\"]")).unwrap();
    let o = export(
        &root,
        &[
            "--render-mode",
            "selected-only",
            "--select-file",
            path.to_str().unwrap(),
        ],
    );
    assert!(o.status.success());
    assert_eq!(
        frame(&o.stdout)
            .0
            .get("selected_elements")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        1
    );
    for args in [
        vec!["--render-mode", "solid"],
        vec!["--select", "019a7bf0-0000-7000-8000-000000eeeeee"],
        vec!["--select", id, "--select-file", path.to_str().unwrap()],
    ] {
        assert!(!export(&root, &args).status.success());
    }
    std::fs::write(&path, "[false]").unwrap();
    assert!(
        !export(&root, &["--select-file", path.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(export(&root, &[]).stdout, raw.stdout);
    assert!(!root.join("views.usda").exists());
}
