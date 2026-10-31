// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Contributor-original construction snap acceptance through the real binary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;
use beamcraft_json::Json;
use support::{StoreFixture, at, text};

fn ok(f: &StoreFixture, command: &str, args: &[&str]) -> Json {
    let (code, result) = f.run(command, args);
    assert_eq!(code, 0, "{command}: {result:?}");
    result
}
fn list<'a>(value: &'a Json, key: &str) -> &'a [Json] {
    at(value, &[key]).array().unwrap()
}
fn point(value: &Json) -> Vec<f64> {
    list(value, "point_world_mm")
        .iter()
        .map(|n| n.float().unwrap())
        .collect()
}

#[test]
fn grid_midpoints_include_each_unequal_bay_with_nearest_disabled() {
    let f = StoreFixture::new("grid-bay-midpoints");
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "Bays",
            "--u-expression",
            "0 1000 2000",
            "--v-expression",
            "0 800 1200",
            "--u-span",
            "-100,2100",
            "--v-span",
            "-100,3100",
        ],
    );
    let before = ok(&f, "open-revision", &[]);
    for (query, expected) in [
        ("501,1,300", [500., 0., 300.]),
        ("2001,1,300", [2000., 0., 300.]),
        ("1,401,300", [0., 400., 300.]),
        ("1001,1401,300", [1000., 1400., 300.]),
    ] {
        let hit = ok(
            &f,
            "snap",
            &[
                "--point",
                query,
                "--radius",
                "3",
                "--kinds",
                "midpoint",
                "--nearest",
                "false",
            ],
        );
        assert!(
            list(&hit, "candidates")
                .iter()
                .any(|h| point(h) == expected),
            "{hit:?}"
        );
        for gate in ["--reference-snaps", "--grid-snaps"] {
            let off = ok(
                &f,
                "snap",
                &[
                    "--point", query, "--radius", "3", "--kinds", "midpoint", gate, "false",
                ],
            );
            assert!(list(&off, "candidates").is_empty(), "{off:?}");
        }
    }
    // The span midpoint follows the same inclined-plane lift as every grid target.
    let hit = ok(
        &f,
        "snap",
        &[
            "--point",
            "1,401,401",
            "--radius",
            "3",
            "--kinds",
            "midpoint",
            "--nearest",
            "false",
            "--plane",
            "0,0,0,1,0,0,0,0.7071067811865476,0.7071067811865476",
        ],
    );
    assert!(
        list(&hit, "candidates").iter().any(|h| point(h)
            .iter()
            .zip([0., 400., 400.])
            .all(|(a, b)| (a - b).abs() < 1e-8)),
        "{hit:?}"
    );
    assert_eq!(before, ok(&f, "open-revision", &[]));
}

#[test]
fn selection_edges_follow_the_evaluated_triangle_instead_of_its_box() {
    let f = StoreFixture::new("triangle-outline");
    let plate = ok(
        &f,
        "create-plate",
        &[
            "--contour",
            "0,0;0,800;1000,0",
            "--normalize-winding",
            "true",
            "--thickness",
            "10",
            "--translation",
            "100,200,300",
        ],
    );
    let id = text(at(&plate, &["plate"]), "element_id");
    let before = ok(&f, "open-revision", &[]);
    let result = ok(&f, "element-edges", &["--id", &id]);
    let edges = list(&result, "edges_world_mm");
    assert_eq!(edges.len(), 9, "{result:?}");
    for edge in edges {
        for p in edge.array().unwrap() {
            let p = p
                .array()
                .unwrap()
                .iter()
                .map(|v| v.float().unwrap())
                .collect::<Vec<_>>();
            assert!(
                [
                    [100., 200., 300.],
                    [1100., 200., 300.],
                    [100., 1000., 300.],
                    [100., 200., 310.],
                    [1100., 200., 310.],
                    [100., 1000., 310.]
                ]
                .iter()
                .any(|v| p == *v),
                "invented box corner: {p:?}"
            );
        }
    }
    assert_eq!(before, ok(&f, "open-revision", &[]));
}

#[test]
fn source_and_point_kind_switches_are_independent_for_grids_and_body_midpoints() {
    let f = StoreFixture::new("snap-switch-matrix");
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "G",
            "--u-expression",
            "0 1000",
            "--v-expression",
            "0 1000",
            "--u-span",
            "0,1000",
            "--v-span",
            "0,1000",
        ],
    );
    ok(
        &f,
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "800",
            "--thickness",
            "10",
            "--translation",
            "3000,0,0",
        ],
    );
    let before = ok(&f, "open-revision", &[]);
    for reference in [false, true] {
        for geometry in [false, true] {
            for grids in [false, true] {
                for nearest in [false, true] {
                    let boolean = |value| if value { "true" } else { "false" };
                    let result = ok(
                        &f,
                        "snap",
                        &[
                            "--point",
                            "1,137,0",
                            "--radius",
                            "2",
                            "--kinds",
                            "grid_line",
                            "--reference-snaps",
                            boolean(reference),
                            "--geometry-snaps",
                            boolean(geometry),
                            "--grid-snaps",
                            boolean(grids),
                            "--nearest",
                            boolean(nearest),
                        ],
                    );
                    let hits = list(&result, "candidates");
                    assert_eq!(
                        hits.len(),
                        usize::from(reference && grids && nearest),
                        "reference={reference} geometry={geometry} grids={grids} nearest={nearest}: {result:?}"
                    );
                    if !hits.is_empty() {
                        assert_eq!(point(&hits[0]), [0., 137., 0.]);
                    }
                }
            }
        }
    }
    for (query, kind, expected) in [
        ("1,500,0", "midpoint", vec![0., 500., 0.]),
        ("1,1,0", "grid_intersection", vec![0., 0., 0.]),
        ("1,1,0", "grid_end", vec![0., 0., 0.]),
    ] {
        let result = ok(
            &f,
            "snap",
            &[
                "--point",
                query,
                "--radius",
                "2",
                "--kinds",
                kind,
                "--nearest",
                "false",
            ],
        );
        assert!(!list(&result, "candidates").is_empty(), "{result:?}");
        assert!(
            list(&result, "candidates")
                .iter()
                .all(|hit| point(hit) == expected)
        );
    }
    // Midpoint is one point kind supplied by different sources. Excluded physical geometry
    // must not leak through a generic midpoint request or consume the single result slot.
    for geometry in [false, true] {
        let result = ok(
            &f,
            "snap",
            &[
                "--point",
                "3500,1,0",
                "--radius",
                "2",
                "--kinds",
                "midpoint",
                "--reference-snaps",
                "false",
                "--geometry-snaps",
                if geometry { "true" } else { "false" },
                "--nearest",
                "false",
                "--max-hits",
                "1",
            ],
        );
        assert_eq!(
            list(&result, "candidates").len(),
            usize::from(geometry),
            "{result:?}"
        );
    }
    assert_eq!(before, ok(&f, "open-revision", &[]));
}

#[test]
fn physical_edges_midpoints_and_contour_vertices_are_exact_and_read_only() {
    let f = StoreFixture::new("construction-snaps");
    ok(
        &f,
        "create-plate",
        &[
            "--contour",
            "0,0;1000,0;1000,300;400,300;400,800;0,800",
            "--thickness",
            "10",
        ],
    );
    let before = ok(&f, "open-revision", &[]);
    for (kind, query, expected) in [
        ("midpoint", "502,3,0", vec![500., 0., 0.]),
        ("edge", "601,3,0", vec![601., 0., 0.]),
        ("vertex", "402,302,0", vec![400., 300., 0.]),
    ] {
        let snapped = ok(
            &f,
            "snap",
            &[
                "--point",
                query,
                "--radius",
                "5",
                "--kinds",
                kind,
                "--plane",
                "0,0,0,1,0,0,0,1,0",
            ],
        );
        let hits = list(&snapped, "candidates");
        assert_eq!(hits.len(), 1, "{snapped:?}");
        assert_eq!(text(&hits[0], "kind"), kind);
        assert_eq!(point(&hits[0]), expected);
        assert!(at(&hits[0], &["feature_id"]).string().is_ok());
    }
    assert_eq!(before, ok(&f, "open-revision", &[]));
}

#[test]
fn offset_profile_member_snaps_to_authored_axis_instead_of_physical_bounds() {
    let f = StoreFixture::new("construction-axes");
    let profile = text(
        at(
            &ok(
                &f,
                "define-profile",
                &[
                    "--family", "i_shape", "--h", "200", "--b", "100", "--tw", "6", "--tf", "10",
                    "--r", "0",
                ],
            ),
            &["definition"],
        ),
        "digest",
    );
    ok(
        &f,
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &profile,
            "--section-plane-offset",
            "350",
            "--translation",
            "100,2000,50",
        ],
    );
    let before = ok(&f, "open-revision", &[]);
    for (kind, query, expected) in [
        ("axis_end", "101,2001,50", vec![100., 2000., 50.]),
        ("axis", "731,2001,50", vec![731., 2000., 50.]),
        ("center", "1601,2001,50", vec![1600., 2000., 50.]),
        ("midpoint", "1601,2001,50", vec![1600., 2000., 50.]),
    ] {
        let snapped = ok(
            &f,
            "snap",
            &["--point", query, "--radius", "3", "--kinds", kind],
        );
        let hits = list(&snapped, "candidates");
        assert_eq!(hits.len(), 1, "{snapped:?}");
        assert_eq!(point(&hits[0]), expected);
    }
    let outside = ok(
        &f,
        "snap",
        &[
            "--point",
            "1600,2000,50",
            "--radius",
            "5",
            "--kinds",
            "center",
            "--plane",
            "0,0,0,1,0,0,0,1,0",
        ],
    );
    assert!(list(&outside, "candidates").is_empty());
    assert_eq!(before, ok(&f, "open-revision", &[]));
}

#[test]
fn grid_intersections_meet_inclined_plane_and_snapped_contour_publishes_exact_plate() {
    let f = StoreFixture::new("construction-grid");
    ok(
        &f,
        "create-grid",
        &[
            "--name",
            "G",
            "--u-expression",
            "0 1000",
            "--v-expression",
            "0 2000",
        ],
    );
    let before = ok(&f, "open-revision", &[]);
    let plane = "0,0,-100,1,0,0,0,0.7071067811865476,0.7071067811865476";
    let snapped = ok(
        &f,
        "snap",
        &[
            "--point",
            "1002,2001,1901",
            "--radius",
            "5",
            "--kinds",
            "grid_intersection",
            "--plane",
            plane,
        ],
    );
    let hits = list(&snapped, "candidates");
    assert_eq!(hits.len(), 1, "{snapped:?}");
    assert!(
        point(&hits[0])
            .iter()
            .zip([1000., 2000., 1900.])
            .all(|(a, b)| (a - b).abs() < 1e-8)
    );
    let mut vertices = Vec::new();
    for query in ["1,2,0", "1002,1,0", "1001,2002,0", "2,2001,0"] {
        let result = ok(
            &f,
            "snap",
            &[
                "--point",
                query,
                "--radius",
                "5",
                "--kinds",
                "grid_intersection",
            ],
        );
        let hit = &list(&result, "candidates")[0];
        vertices.push(
            point(hit)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    assert_eq!(before, ok(&f, "open-revision", &[]));
    let plate = ok(
        &f,
        "create-plate",
        &["--contour", &vertices.join(";"), "--thickness", "10"],
    );
    assert_eq!(
        at(&plate, &["plate", "quantities", "volume", "mm3"])
            .float()
            .unwrap(),
        20_000_000.
    );
}
