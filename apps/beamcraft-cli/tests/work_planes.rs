// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V08 synthetic frames through the real CLI. Expected world positions are elementary axes.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::{Json, Limits};
use std::process::Command;
use support::{StoreFixture, text};
fn ok(f: &StoreFixture, cmd: &str, args: &[&str]) -> Json {
    let (status, reply) = f.run(cmd, args);
    assert_eq!(status, 0, "{cmd}: {reply:?}");
    reply
}
fn construct(args: &[&str]) -> (i32, Json) {
    let result = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg("work-plane")
        .args(args)
        .output()
        .unwrap();
    (
        result.status.code().unwrap(),
        beamcraft_json::decode(&result.stdout, &Limits::DEFAULT).unwrap(),
    )
}
fn values(j: &Json) -> Vec<f64> {
    j.array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}
#[test]
fn construction_modes_are_read_only_and_generate_cli_plane_expressions() {
    for (args, normal) in [
        (vec!["--parallel", "xy", "--offset", "300"], [0., 0., 1.]),
        (vec!["--parallel", "xz", "--offset", "200"], [0., -1., 0.]),
        (vec!["--parallel", "yz", "--offset", "100"], [1., 0., 0.]),
        (
            vec!["--points", "10,20,30,11,20,30,10,20,31"],
            [0., -1., 0.],
        ),
        (vec!["--vertical", "10,20,30,11,20,90"], [0., -1., 0.]),
    ] {
        let (code, j) = construct(&args);
        assert_eq!(code, 0, "{j:?}");
        assert_eq!(values(j.get("normal").unwrap()), normal);
        let expression = text(&j, "plane");
        let (code, reread) = construct(&["--plane", &expression]);
        assert_eq!(code, 0);
        assert_eq!(j, reread);
    }
    for args in [
        vec!["--parallel", "xy", "--offset", "NaN"],
        vec!["--points", "0,0,0,1,0,0,2,0,0"],
        vec!["--vertical", "0,0,0,0,0,1"],
        vec!["--plane", "0,0,0,1,0,0,1,1,0"],
        vec!["--parallel", "xy", "--points", "0,0,0,1,0,0,0,1,0"],
    ] {
        assert_eq!(construct(&args).0, 2);
    }
}
#[test]
fn plane_local_plate_and_member_reopen_at_the_expected_world_positions() {
    let f = StoreFixture::new("v08-placement");
    let expression = "10,200,30,1,0,0,0,0,1";
    let plate = ok(
        &f,
        "create-plate",
        &[
            "--width",
            "200",
            "--length",
            "100",
            "--thickness",
            "10",
            "--plane",
            expression,
        ],
    );
    let record = plate.get("plate").unwrap();
    assert_eq!(
        values(
            record
                .get("placement")
                .unwrap()
                .get("translation_mm")
                .unwrap()
        ),
        [10., 200., 30.]
    );
    let inspected = ok(&f, "inspect-elements", &[]);
    let entry = &inspected
        .get("snapshot")
        .unwrap()
        .get("elements")
        .unwrap()
        .array()
        .unwrap()[0];
    for (key, expected) in [
        ("world_min_mm", [10., 190., 30.]),
        ("world_max_mm", [210., 200., 130.]),
    ] {
        for (v, e) in values(entry.get(key).unwrap()).iter().zip(expected) {
            assert!((v - e).abs() < 1e-9);
        }
    }
    let before = ok(&f, "open-revision", &[]);
    let face = ok(
        &f,
        "work-plane",
        &["--origin", "110,0,80", "--direction", "0,1,0"],
    );
    for (a, b) in values(face.get("normal").unwrap())
        .into_iter()
        .zip([0., -1., 0.])
    {
        assert!((a - b).abs() < 1e-12);
    }
    assert_eq!(before, ok(&f, "open-revision", &[]));
    let missing = ok(
        &f,
        "work-plane",
        &["--origin", "-100,0,80", "--direction", "0,1,0"],
    );
    assert_eq!(text(&missing, "outcome"), "no_face");
    let profile = ok(&f, "define-profile", &["--width", "20", "--height", "30"]);
    let digest = text(profile.get("definition").unwrap(), "digest");
    let beam = ok(
        &f,
        "create-member",
        &[
            "--profile-digest",
            &digest,
            "--start",
            "0,0",
            "--end",
            "1000,200",
            "--plane",
            expression,
        ],
    );
    let axis = beam
        .get("member")
        .unwrap()
        .get("axis_points_mm")
        .unwrap()
        .array()
        .unwrap();
    assert_eq!(values(&axis[0]), [10., 200., 30.]);
    assert_eq!(values(&axis[1]), [1010., 200., 230.]);
    let before = ok(&f, "inspect-elements", &[]);
    for extra in [
        ["--translation", "0,0,0"],
        ["--orientation-xyzw", "0,0,0,1"],
        ["--z", "100"],
    ] {
        let mut args = vec![
            "--width",
            "200",
            "--length",
            "100",
            "--thickness",
            "10",
            "--plane",
            expression,
        ];
        args.extend(extra);
        assert_eq!(f.run("create-plate", &args).0, 2);
    }
    assert_eq!(before, ok(&f, "inspect-elements", &[]));
}

#[test]
fn a_named_hole_facet_can_define_its_actual_planar_work_plane() {
    let f = StoreFixture::new("v08-nonplanar-face");
    ok(
        &f,
        "create-plate",
        &[
            "--width",
            "200",
            "--length",
            "100",
            "--thickness",
            "10",
            "--hole-round",
            "100,50,40",
        ],
    );
    let before = ok(&f, "open-revision", &[]);
    let (code, result) = f.run(
        "work-plane",
        &["--origin", "100,50,5", "--direction", "1,0,0"],
    );
    assert_eq!(code, 0, "{result:?}");
    let normal = values(result.get("normal").unwrap());
    assert!(normal[0] < -0.99 && normal[1].abs() > 0.09 && normal[1].abs() < 0.11);
    assert_eq!(normal[2], 0.0);
    assert_eq!(values(result.get("origin_mm").unwrap()), [120., 50., 5.]);
    assert_eq!(before, ok(&f, "open-revision", &[]));
}
