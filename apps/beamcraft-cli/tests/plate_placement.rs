// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P10 plate slice: rigid placement leaves the body local and reaches snapshot, pick and IFC.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use support::StoreFixture;
fn ok(f: &StoreFixture, cmd: &str, args: &[&str]) -> Json {
    let (status, reply) = f.run(cmd, args);
    assert_eq!(status, 0, "{cmd}: {reply:?}");
    reply
}
#[test]
fn plate_rotation_is_a_pose_and_survives_reopen_pick_and_ifc_export() {
    let f = StoreFixture::new("p10-plate-placement");
    let made = ok(
        &f,
        "create-plate",
        &[
            "--width",
            "200",
            "--length",
            "100",
            "--thickness",
            "10",
            "--translation",
            "1000,2000,3000",
        ],
    );
    let before = made.get("plate").unwrap();
    let id = before.get("element_id").unwrap().string().unwrap();
    let turned = ok(
        &f,
        "update-element",
        &[
            "--id",
            id,
            "--orientation-xyzw",
            "0.7071067811865476,0,0,0.7071067811865476",
        ],
    );
    let after = turned.get("element").unwrap();
    assert_eq!(
        before.get("mesh_digest").unwrap(),
        after.get("mesh_digest").unwrap()
    );
    assert_eq!(
        before.get("quantities").unwrap(),
        after.get("quantities").unwrap()
    );
    let inspected = ok(&f, "inspect-elements", &[]);
    let element = &inspected
        .get("snapshot")
        .unwrap()
        .get("elements")
        .unwrap()
        .array()
        .unwrap()[0];
    for (key, expected) in [
        ("world_min_mm", [1000., 1990., 3000.]),
        ("world_max_mm", [1200., 2000., 3100.]),
    ] {
        for (v, e) in element
            .get(key)
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .zip(expected)
        {
            assert!((v.float().unwrap() - e).abs() < 1e-8);
        }
    }
    let pick = ok(
        &f,
        "pick",
        &[
            "--origin",
            "1100,1900,3050",
            "--direction",
            "0,1,0",
            "--targets",
            "face",
            "--max-hits",
            "1",
        ],
    );
    assert_eq!(pick.get("hits").unwrap().array().unwrap().len(), 1);
    let out = f.root.join("rotated.ifc");
    ok(&f, "export-ifc", &["--out", out.to_str().unwrap()]);
    let ifc = std::fs::read_to_string(out).unwrap();
    assert!(ifc.contains("IFCPLATE(") && ifc.contains("IFCAXIS2PLACEMENT3D("));
}
