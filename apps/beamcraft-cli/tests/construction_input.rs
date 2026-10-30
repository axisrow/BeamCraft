// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Contributor-original 3D numeric direction and cursor snap acceptance.
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

#[test]
fn orthographic_cursor_aperture_keeps_the_same_targets_at_different_depths() {
    let f = StoreFixture::new("orthographic-cursor");
    assert_eq!(
        f.run(
            "create-plate",
            &[
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--translation",
                "0,0,1000"
            ]
        )
        .0,
        0
    );
    let before = f.run("open-revision", &[]);
    let mut expected = None;
    for origin in ["1,0,2000", "1,0,1000000"] {
        let (code, result) = f.run(
            "snap",
            &[
                "--point",
                "0,0,0",
                "--radius",
                "2",
                "--kinds",
                "vertex",
                "--origin",
                origin,
                "--direction",
                "0,0,-1",
                "--cursor-radius",
                "2",
            ],
        );
        assert_eq!(code, 0, "{result:?}");
        let points = at(&result, &["candidates"])
            .array()
            .unwrap()
            .iter()
            .map(point)
            .collect::<Vec<_>>();
        assert!(points.contains(&vec![0., 0., 1000.]));
        if let Some(expected) = &expected {
            assert_eq!(&points, expected);
        } else {
            expected = Some(points);
        }
    }
    for radius in ["0", "-1", "NaN", "1000001"] {
        assert_eq!(
            f.run(
                "snap",
                &[
                    "--point",
                    "0,0,0",
                    "--radius",
                    "2",
                    "--kinds",
                    "vertex",
                    "--origin",
                    "1,0,2000",
                    "--direction",
                    "0,0,-1",
                    "--cursor-radius",
                    radius
                ]
            )
            .0,
            2
        );
    }
    assert_eq!(before, f.run("open-revision", &[]));
}

#[test]
fn cursor_snap_finds_elevated_geometry_independently_of_ground_point() {
    let f = StoreFixture::new("spatial-cursor");
    let (code, p) = f.run(
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--translation",
            "0,0,1000",
        ],
    );
    assert_eq!(code, 0, "{p:?}");
    let before = f.run("open-revision", &[]);
    let (code, result) = f.run(
        "snap",
        &[
            "--point",
            "0,0,0",
            "--radius",
            "2",
            "--kinds",
            "vertex",
            "--origin",
            "0,-500,1500",
            "--direction",
            "0,500,-500",
            "--aperture",
            "0.0001",
        ],
    );
    assert_eq!(code, 0, "{result:?}");
    let hits = at(&result, &["candidates"]).array().unwrap();
    assert_eq!(hits.len(), 1, "{result:?}");
    assert_eq!(point(&hits[0]), [0., 0., 1000.]);
    assert_eq!(before, f.run("open-revision", &[]));
}
