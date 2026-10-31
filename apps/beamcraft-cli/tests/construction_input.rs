// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Contributor-original 3D numeric direction acceptance.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::{Json, Limits};
use support::{StoreFixture, at};

fn resolve(args: &[&str]) -> (i32, Json) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg("resolve-construction-point")
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap(),
    )
}
fn point(result: &Json) -> Vec<f64> {
    at(result, &["point_world_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|n| n.float().unwrap())
        .collect()
}
#[test]
fn typed_length_and_cursor_ortho_produce_exact_spatial_points_without_a_store() {
    let (code, p) = resolve(&[
        "--reference",
        "10,20,30",
        "--toward",
        "13,24,42",
        "--distance",
        "500",
    ]);
    assert_eq!(code, 0, "{p:?}");
    assert!(
        point(&p)
            .iter()
            .zip([10. + 1500. / 13., 20. + 2000. / 13., 30. + 6000. / 13.])
            .all(|(a, b)| (a - b).abs() < 1e-9)
    );
    let (code, p) = resolve(&[
        "--reference",
        "10,20,30",
        "--toward",
        "13,24,42",
        "--distance",
        "500",
        "--ortho",
        "true",
    ]);
    assert_eq!(code, 0, "{p:?}");
    assert_eq!(point(&p), [10., 20., 530.]);
    let (code, p) = resolve(&[
        "--reference",
        "0,0,0",
        "--origin",
        "1000,-1000,1000",
        "--direction",
        "-1000,1000,-400",
        "--distance",
        "500",
        "--ortho",
        "true",
    ]);
    assert_eq!(code, 0, "{p:?}");
    assert!(
        point(&p)
            .iter()
            .zip([0., 0., 500.])
            .all(|(a, b)| (a - b).abs() < 1e-9)
    );
    for value in ["NaN", "0", "-500", "inf", "1e99"] {
        assert_eq!(
            resolve(&[
                "--reference",
                "0,0,0",
                "--toward",
                "3,4,12",
                "--distance",
                value
            ])
            .0,
            2
        );
    }
}
#[test]
fn construction_contours_accept_either_winding_without_repairing_crossings() {
    let f = StoreFixture::new("construction-winding");
    for plane in [None, Some("100,200,300,0,1,0,0,0,1")] {
        for contour in [
            "0,0;1000,0;1000,200;200,200;200,800;0,800",
            "0,0;0,800;200,800;200,200;1000,200;1000,0",
        ] {
            let mut args = vec![
                "--contour",
                contour,
                "--thickness",
                "10",
                "--normalize-winding",
                "true",
            ];
            if let Some(plane) = plane {
                args.extend(["--plane", plane]);
            }
            let (code, result) = f.run("create-plate", &args);
            assert_eq!(code, 0, "{result:?}");
            let (_, inspected) = f.run("inspect-elements", &[]);
            for plate in at(&inspected, &["plates"]).array().unwrap() {
                assert_eq!(
                    at(plate, &["quantities", "volume", "mm3"]).float().unwrap(),
                    3_200_000.
                );
            }
        }
    }
    let before = f.run("open-revision", &[]);
    for contour in ["0,0;1000,1000;0,1000;1000,0", "0,0;0,0;1000,0"] {
        let (code, result) = f.run(
            "create-plate",
            &[
                "--contour",
                contour,
                "--thickness",
                "10",
                "--normalize-winding",
                "true",
            ],
        );
        assert_eq!(code, 2, "{result:?}");
        assert_eq!(before, f.run("open-revision", &[]));
    }
}
