// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Bounded linear/polar groups and canonical-link remapping through the real CLI.
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
fn plate(f: &StoreFixture) -> String {
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
                "10,0,0",
            ],
        )
        .get("plate")
        .unwrap(),
        "element_id",
    )
}
fn records(f: &StoreFixture) -> Vec<Json> {
    ok(f, "inspect-elements", &[])
        .get("plates")
        .unwrap()
        .array()
        .unwrap()
        .to_vec()
}
const OP: &str = "019a7bf0-0000-7000-8000-000000a00001";
#[test]
fn linear_count_excludes_source_and_replays_without_writing() {
    let f = StoreFixture::new("array-linear");
    let id = plate(&f);
    let args = [
        "--ids", &id, "--step", "10,0,0", "--count", "3", "--op-id", OP,
    ];
    let result = ok(&f, "array-linear", &args);
    assert_eq!(result.get("elements").unwrap().array().unwrap().len(), 3);
    let mut xs = records(&f)
        .iter()
        .map(|p| {
            p.get("placement")
                .unwrap()
                .get("translation_mm")
                .unwrap()
                .array()
                .unwrap()[0]
                .float()
                .unwrap()
        })
        .collect::<Vec<_>>();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs, vec![10., 20., 30., 40.]);
    let head = ok(&f, "open-revision", &[]);
    assert_eq!(
        text(&ok(&f, "array-linear", &args), "outcome"),
        "transform_replayed"
    );
    assert_eq!(ok(&f, "open-revision", &[]), head);
}
#[test]
fn polar_rotates_each_copy_from_the_original_and_all_limits_are_atomic() {
    let f = StoreFixture::new("array-polar");
    let id = plate(&f);
    ok(
        &f,
        "array-polar",
        &[
            "--ids",
            &id,
            "--center",
            "0,0,0",
            "--axis",
            "0,0,1",
            "--angle",
            "1.5707963267948966",
            "--count",
            "3",
            "--instance",
            "true",
        ],
    );
    let points = records(&f)
        .iter()
        .map(|p| {
            p.get("placement")
                .unwrap()
                .get("translation_mm")
                .unwrap()
                .array()
                .unwrap()
                .iter()
                .map(|v| v.float().unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for [x, y] in [[10., 0.], [0., 10.], [-10., 0.], [0., -10.]] {
        assert!(
            points
                .iter()
                .any(|p| (p[0] - x).abs() < 1e-8 && (p[1] - y).abs() < 1e-8)
        );
    }
    let head = ok(&f, "open-revision", &[]);
    for count in ["0", "129", "-1"] {
        assert_eq!(
            f.run(
                "array-linear",
                &["--ids", &id, "--step", "10,0,0", "--count", count]
            )
            .0,
            2
        );
        assert_eq!(ok(&f, "open-revision", &[]), head);
    }
    assert_eq!(
        f.run(
            "array-linear",
            &["--ids", &id, "--step", "1000000000,0,0", "--count", "2"]
        )
        .0,
        2
    );
    assert_eq!(ok(&f, "open-revision", &[]), head);
}
#[test]
fn independent_array_remaps_internal_instance_links_per_slot() {
    let f = StoreFixture::new("array-links");
    let owner = plate(&f);
    ok(
        &f,
        "add-hole",
        &["--id", &owner, "--hole-rect", "10,10,10,10"],
    );
    ok(
        &f,
        "cut-by-plane",
        &[
            "--id",
            &owner,
            "--point",
            "80,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
            "--operation-id",
            "019a7bf0-0000-7000-8000-000000a00002",
        ],
    );
    let copy = ok(
        &f,
        "copy-instance",
        &["--id", &owner, "--offset", "0,100,0"],
    );
    let instance = text(copy.get("element").unwrap(), "element_id");
    let ids = format!("{owner},{instance}");
    let j = ok(
        &f,
        "array-linear",
        &["--ids", &ids, "--step", "500,0,0", "--count", "2"],
    );
    let entries = j.get("elements").unwrap().array().unwrap();
    assert_eq!(entries.len(), 4);
    let owner_copy = text(
        entries
            .iter()
            .find(|j| text(j, "source_id") == owner)
            .unwrap(),
        "element_id",
    );
    ok(
        &f,
        "update-element",
        &["--id", &owner_copy, "--thickness", "20"],
    );
    let all = records(&f);
    let thickness = |id: &str| {
        all.iter()
            .find(|j| text(j, "element_id") == id)
            .unwrap()
            .get("dimensions_mm")
            .unwrap()
            .get("thickness")
            .unwrap()
            .float()
            .unwrap()
    };
    assert_eq!(thickness(&owner), 10.);
    assert_eq!(thickness(&instance), 10.);
    assert_eq!(
        all.iter()
            .filter(|j| j
                .get("dimensions_mm")
                .unwrap()
                .get("thickness")
                .unwrap()
                .float()
                .unwrap()
                == 20.)
            .count(),
        2
    );
}
