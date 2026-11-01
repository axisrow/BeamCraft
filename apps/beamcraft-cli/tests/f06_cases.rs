// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original driver of the repository's frozen F06 acceptance descriptions.

//! K1 plate projections of F06, read from the JSON fixture rather than copied into tests.
//! This is deliberately not a claim that all F06 outputs exist: the evidence note lists the
//! assertions for each case, missing outputs and the exact remaining case inventory.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_json::{Json, Limits};
use beamcraft_model::command::Placement;
use beamcraft_model::digest::Sha256;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::{BTreeMap, BTreeSet};
use steel::engine::command::{CreatePlateOutcome, MaterialInfo, create_plate, no_materials};
use steel::model::command::CreatePlate;

// Explicit lists: a new or removed fixture case must change the evidence inventory. Members,
// feature/key/query protocols and fault-injection requests need their own adapters, not a
// synthetic successful result here. NEG-08 and NEG-39 are superseded by amendment A-2.
const DRIVEN: &str = "POS-01 POS-02 POS-03 POS-11 POS-48 POS-49 POS-53 POS-54 POS-55 POS-56 \
    NEG-01 NEG-02 NEG-03 NEG-37 NEG-38 NEG-40 NEG-41 NEG-42 NEG-45 NEG-46 NEG-49";
const MEMBERS: &str = "POS-04 POS-05 POS-06 POS-07 POS-08 POS-09 POS-10 POS-12 POS-13 POS-14 \
    POS-15 POS-41 POS-42 POS-46 POS-50 POS-51 POS-52 POS-57 POS-58 \
    NEG-04 NEG-05 NEG-06 NEG-07 NEG-43 NEG-44 NEG-47 NEG-48 NEG-51 NEG-52 NEG-53";
const PROTOCOL: &str = "POS-16 POS-17 POS-18 POS-19 POS-20 POS-21 POS-22 POS-23 POS-24 POS-25 \
    POS-26 POS-27 POS-28 POS-29 POS-30 POS-31 POS-32 POS-33 POS-34 POS-35 POS-36 POS-38 POS-39 \
    POS-40 POS-43 POS-44 POS-45 POS-47 POS-59 NEG-09 NEG-10 NEG-11 NEG-12 NEG-13 NEG-14 NEG-15 \
    NEG-18 NEG-19 NEG-20 NEG-21 NEG-22 NEG-23 NEG-24 NEG-25 NEG-26 NEG-27 NEG-28 NEG-29 NEG-30 \
    NEG-33 NEG-34 NEG-36";
const INJECTION: &str = "POS-37 NEG-16 NEG-17 NEG-31 NEG-32 NEG-35 NEG-50";
const SUPERSEDED: &str = "NEG-08 NEG-39";
const DEVIATIONS: &[&str] = &[];

fn fixture() -> Json {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/contracts/f06-geometry-contract-cases-v0.1.json"
    ))
    .unwrap();
    let file = beamcraft_json::decode(&bytes, &Limits::DEFAULT).unwrap();
    assert_eq!(text(&file, "contract_version"), "0.1.0");
    file
}

fn text<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key).unwrap().string().unwrap()
}

// F06 writes exact references as decimal strings and non-finite typed inputs as sentinels.
// No computed reference, production geometry helper or JSON canonicalizer is used here.
fn number(json: &Json) -> f64 {
    match json {
        Json::String(s) => match s.as_str() {
            "NaN" => f64::NAN,
            "-Infinity" => f64::NEG_INFINITY,
            "Infinity" => f64::INFINITY,
            _ => s.parse().unwrap(),
        },
        Json::Number(_) => json.float().unwrap(),
        _ => panic!("expected a fixture number: {json:?}"),
    }
}

fn vector<const N: usize>(json: &Json) -> [f64; N] {
    json.array()
        .unwrap()
        .iter()
        .map(number)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

fn command(input: &Json) -> CreatePlate {
    let body = input.get("body").unwrap();
    assert_eq!(text(body, "kind"), "plate");
    let placement = input.get("placement").unwrap();
    CreatePlate {
        new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
        name: "F06 plate".into(),
        class: 0,
        contour_points_mm: body
            .get("contour_points_mm")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(vector)
            .collect(),
        thickness_mm: number(body.get("thickness_mm").unwrap()),
        thickness_side: None,
        corners: Vec::new(),
        placement: Placement {
            translation_mm: vector(placement.get("translation_mm").unwrap()),
            orientation_xyzw: vector(placement.get("orientation_xyzw").unwrap()),
        },
        material_digest: None,
        holes_mm: Vec::new(),
    }
}

fn close(actual: f64, expected: f64, coordinate: bool) {
    let tolerance = 1e-6 * expected.abs() + if coordinate { 0.001 } else { 0.0 };
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tolerance,
        "{actual} != {expected} (tolerance {tolerance})"
    );
}

fn check_number(actual: f64, expected: &Json, key: &str) {
    close(actual, number(expected.get(key).unwrap()), false);
}

fn check_count(actual: usize, expected: &Json, key: &str) {
    assert_eq!(
        u64::try_from(actual).unwrap(),
        expected.get(key).unwrap().unsigned().unwrap(),
        "{key}"
    );
}

fn check_bounds(actual: &[[f64; 3]], expected: &Json) {
    for (axis, name) in ["x", "y", "z"].iter().enumerate() {
        let (min, max) = actual
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), p| {
                (min.min(p[axis]), max.max(p[axis]))
            });
        let bounds = if let Ok(bounds) = expected.get(name) {
            vector::<2>(bounds)
        } else {
            [
                vector::<3>(expected.get("min").unwrap())[axis],
                vector::<3>(expected.get("max").unwrap())[axis],
            ]
        };
        close(min, bounds[0], true);
        close(max, bounds[1], true);
    }
}

fn mesh_case(out: &CreatePlateOutcome, expected: &Json) {
    let mesh = out.body.result.as_ref().unwrap().mesh();
    check_count(mesh.vertices().len(), expected, "vertex_count");
    check_count(mesh.triangles().len(), expected, "triangle_count");
    check_count(mesh.face_table().len(), expected, "face_count");
    assert_eq!(
        mesh.vertices().len(),
        expected.get("vertices_mm").unwrap().array().unwrap().len()
    );
    for (p, reference) in mesh
        .vertices()
        .iter()
        .zip(expected.get("vertices_mm").unwrap().array().unwrap())
    {
        for (actual, reference) in [p.x, p.y, p.z].iter().zip(vector::<3>(reference)) {
            close(*actual, reference, true);
        }
    }
    let triangles: Vec<[u32; 3]> = expected
        .get("triangles")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|v| {
            v.array()
                .unwrap()
                .iter()
                .map(|n| u32::try_from(n.unsigned().unwrap()).unwrap())
                .collect::<Vec<_>>()
                .try_into()
                .unwrap()
        })
        .collect();
    assert_eq!(mesh.triangles().as_slice(), triangles);
    let faces: Vec<u16> = expected
        .get("triangle_face_index")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|v| u16::try_from(v.unsigned().unwrap()).unwrap())
        .collect();
    assert_eq!(mesh.triangle_faces().as_slice(), faces);
    let ids: Vec<&str> = expected
        .get("face_local_ids")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|v| v.string().unwrap())
        .collect();
    // The fixture has axis roles ("x0"), while the public mesh table currently returns
    // "plate:f.x0". Assert the roles, but explicitly retain this wire-format deviation.
    let roles: Vec<_> = mesh
        .face_table()
        .iter()
        .map(|id| id.strip_prefix("plate:f.").unwrap())
        .collect();
    assert_eq!(roles, ids);
    assert_ne!(
        mesh.face_table().as_slice(),
        ids,
        "face id deviation changed; update the evidence note"
    );

    // An independent edge census: every directed triangle edge occurs once with its reverse.
    let mut edges = BTreeMap::new();
    let mut physical_edges = BTreeSet::new();
    let mut volume = 0.0;
    for (i, &[a, b, c]) in mesh.triangles().iter().enumerate() {
        for (a, b) in [(a, b), (b, c), (c, a)] {
            edges.entry((a, b)).and_modify(|n| *n += 1).or_insert(1);
        }
        let p = out.body.solid.vertices();
        let (a, b, c) = (p[a as usize], p[b as usize], p[c as usize]);
        let ab = subtract(b, a);
        let ac = subtract(c, a);
        let normal = cross(ab, ac);
        let face = usize::from(mesh.triangle_faces()[i]);
        let signs = expected
            .get("face_outward_normal_direction")
            .unwrap()
            .get(roles[face])
            .unwrap()
            .array()
            .unwrap();
        for (component, sign) in normal.iter().zip(signs) {
            match sign.string().unwrap() {
                "+" => assert!(*component > 0.0),
                "-" => assert!(*component < 0.0),
                "0" => assert_eq!(*component, 0.0),
                _ => panic!("unknown normal direction"),
            }
        }
        volume += dot(a, cross(b, c)) / 6.0;
    }
    for (&(a, b), count) in &edges {
        assert_eq!(*count, 1);
        assert_eq!(edges.get(&(b, a)), Some(&1));
        // A physical edge separates two different semantic faces; cap diagonals do not.
        let incident: BTreeSet<_> = mesh
            .triangles()
            .iter()
            .zip(mesh.triangle_faces())
            .filter(|(t, _)| t.contains(&a) && t.contains(&b))
            .map(|(_, f)| f)
            .collect();
        if incident.len() == 2 {
            physical_edges.insert((a.min(b), a.max(b)));
        }
    }
    check_count(edges.len() / 2, expected, "triangle_edge_count");
    check_count(physical_edges.len(), expected, "topological_edge_count");
    for (key, e, f) in [
        (
            "euler_triangle_mesh",
            edges.len() / 2,
            mesh.triangles().len(),
        ),
        (
            "euler_topological",
            physical_edges.len(),
            mesh.face_table().len(),
        ),
    ] {
        let euler = expected.get(key).unwrap();
        check_count(mesh.vertices().len(), euler, "V");
        check_count(e, euler, "E");
        check_count(f, euler, "F");
        check_count(mesh.vertices().len() + f - e, euler, "V_minus_E_plus_F");
    }
    check_number(volume, expected, "signed_volume_mm3");
    assert_eq!(expected.get("closed_shell").unwrap(), &Json::Bool(true));
    assert_eq!(
        expected
            .get("every_edge_in_two_triangles_opposite_directions")
            .unwrap(),
        &Json::Bool(true)
    );
    // Flood the triangle adjacency graph as well: Euler alone does not prove one shell.
    let mut visited = BTreeSet::from([0]);
    loop {
        let old = visited.len();
        for (i, t) in mesh.triangles().iter().enumerate() {
            if mesh.triangles().iter().enumerate().any(|(j, u)| {
                visited.contains(&j) && t.iter().filter(|v| u.contains(v)).count() >= 2
            }) {
                visited.insert(i);
            }
        }
        if visited.len() == old {
            break;
        }
    }
    assert_eq!(visited.len(), mesh.triangles().len());
    check_count(1, expected, "shell_count");
}

fn subtract(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn check_success(
    id: &str,
    input: &Json,
    command: &CreatePlate,
    out: &CreatePlateOutcome,
    expected: &Json,
) {
    assert_eq!(
        out.authored.to_command(),
        *command,
        "inputs must not be repaired"
    );
    match id {
        "POS-01" => mesh_case(out, expected),
        "POS-02" => {
            let blob = out.body.solid.blob();
            check_count(blob.len(), expected, "blob_byte_length");
            assert_eq!(
                out.body.solid.digest().to_string(),
                text(expected, "mesh_digest_sha256")
            );
            assert_eq!(
                create_plate(command, &|_| false, &no_materials)
                    .unwrap()
                    .body
                    .solid
                    .blob(),
                blob
            );
            assert_eq!(
                expected.get("second_build_bytes_identical").unwrap(),
                &Json::Bool(true)
            );
            let offsets = expected.get("section_offsets").unwrap();
            let lengths = expected.get("section_byte_lengths").unwrap();
            let mut end = 0;
            for (name, bytes) in [
                ("header", 32),
                ("positions", 8 * 3 * 8),
                ("indices", 12 * 3 * 4),
                ("triangle_face", 12 * 2),
            ] {
                assert_eq!(
                    usize::try_from(offsets.get(name).unwrap().unsigned().unwrap()).unwrap(),
                    end
                );
                assert_eq!(
                    usize::try_from(lengths.get(name).unwrap().unsigned().unwrap()).unwrap(),
                    bytes
                );
                assert_eq!(blob.get(end..end + bytes).unwrap().len(), bytes);
                end += bytes;
            }
            assert_eq!(end, blob.len());
        }
        "POS-03" => {
            check_number(out.quantities.volume_mm3, expected, "volume_mm3");
            check_number(out.quantities.volume_m3, expected, "volume_m3");
            assert_eq!(
                out.quantities.volume_status.as_str(),
                text(expected, "volume_status")
            );
            assert_eq!(
                out.quantities.mass_status.as_str(),
                text(expected, "mass_status_without_density")
            );
            assert_eq!(out.quantities.mass_kg, None);
            let mut with_material = command.clone();
            let densities = input.get("density_kg_m3").unwrap().array().unwrap();
            assert_eq!(densities[0], Json::Null);
            let density = number(&densities[1]);
            with_material.material_digest =
                Some(Sha256::digest(b"synthetic F06 material").unwrap());
            let mass = create_plate(&with_material, &|_| false, &|_| {
                Ok(MaterialInfo {
                    density_kg_m3: Some(density),
                })
            })
            .unwrap();
            check_number(
                mass.quantities.mass_kg.unwrap(),
                expected,
                "mass_kg_with_density_1000",
            );
            assert_eq!(
                mass.quantities.mass_status.as_str(),
                text(expected, "mass_status_with_density_1000")
            );
            check_bounds(
                out.body.solid.vertices(),
                expected.get("local_bounds_mm").unwrap(),
            );
            // Area/status are not exposed by this engine API and are NOT claimed here.
        }
        "POS-11" => {
            use beamcraft_engine::query::feature::{
                FeatureGeometry, list_features, lookup_feature,
            };
            use beamcraft_engine::{Limits as ViewLimits, ViewPin, ViewSnapshot};
            use steel::engine::committed::{evaluate_committed, scene_elements};
            let records =
                evaluate_committed(std::slice::from_ref(&out.authored), &no_materials).unwrap();
            let pin = ViewPin::new(Sha256::digest(b"P10 F06 POS-11").unwrap(), 1);
            let snapshot = ViewSnapshot::build(
                pin,
                scene_elements(&records).unwrap(),
                &ViewLimits::default(),
            )
            .unwrap();
            let world = list_features(&snapshot, &out.authored.id)
                .unwrap()
                .into_iter()
                .filter_map(
                    |id| match lookup_feature(&snapshot, &id.to_string()).unwrap().world {
                        FeatureGeometry::Vertex { position } => Some(position),
                        _ => None,
                    },
                )
                .collect::<Vec<_>>();
            let reference = expected.get("world_vertices_mm").unwrap().array().unwrap();
            assert_eq!(world.len(), reference.len());
            for (actual, reference) in world.iter().zip(reference) {
                for (actual, reference) in actual.iter().zip(vector::<3>(reference)) {
                    close(*actual, reference, true);
                }
            }
            check_bounds(&world, expected.get("world_bounds_mm").unwrap());
            check_number(out.body.solid.volume_mm3(), expected, "signed_volume_mm3");
            let mut unplaced = command.clone();
            unplaced.placement = Placement::IDENTITY;
            let unplaced = create_plate(&unplaced, &|_| false, &no_materials).unwrap();
            assert_eq!(out.body.solid.blob(), unplaced.body.solid.blob());
            assert_eq!(out.quantities, unplaced.quantities);
            // F06 section 7 shape/world measurement keys are not exposed by this adapter.
        }
        "POS-48" | "POS-49" => {
            check_bounds(
                out.body.solid.vertices(),
                expected.get("local_bounds_mm").unwrap(),
            );
            check_number(out.body.width_mm, expected, "width_mm");
            check_number(out.body.length_mm, expected, "length_mm");
            check_number(out.quantities.volume_m3, expected, "volume_m3");
            check_count(out.body.solid.vertices().len(), expected, "vertex_count");
            check_count(out.body.solid.triangles().len(), expected, "triangle_count");
            if id == "POS-48" {
                let normals = out.body.result.as_ref().unwrap().mesh().face_normals();
                let expected_frame = expected.get("plate_frame").unwrap();
                for (actual, name) in [
                    (normals[1], "X_p"),
                    (normals[3], "Y_p"),
                    (normals[5], "Z_p"),
                ] {
                    assert_eq!(
                        actual.components(),
                        vector::<3>(expected_frame.get(name).unwrap())
                    );
                }
            }
        }
        "POS-53" | "POS-54" | "POS-55" => {
            // These are admission projections; the tolerance values themselves still have
            // separate geom unit evidence, not an engine output asserted by this adapter.
            let mut origin = command.clone();
            origin.placement = Placement::IDENTITY;
            assert!(create_plate(&origin, &|_| false, &no_materials).is_ok());
        }
        _ => panic!("no success assertions for {id}"),
    }
}

#[test]
fn every_f06_case_has_an_explicit_k1_disposition() {
    let file = fixture();
    let cases = file.get("cases").unwrap().array().unwrap();
    let mut inventory = BTreeSet::new();
    for (label, group) in [
        ("driven plate projection", DRIVEN),
        ("member/mixed K2", MEMBERS),
        ("protocol adapter not run", PROTOCOL),
        ("injection adapter not run", INJECTION),
        ("superseded A-2", SUPERSEDED),
    ] {
        for id in group.split_whitespace() {
            assert!(inventory.insert(format!("F06-{id}")), "duplicate {id}");
            println!("{label}: F06-{id}");
        }
    }
    let actual: BTreeSet<_> = cases
        .iter()
        .map(|c| text(c, "case_id").to_owned())
        .collect();
    assert_eq!(cases.len(), 112);
    assert_eq!(actual.len(), cases.len());
    assert_eq!(
        inventory, actual,
        "update the runner and evidence note for a changed fixture"
    );
}

#[test]
fn k1_plate_projections_match_the_fixture_or_an_explicit_deviation() {
    let file = fixture();
    let cases = file.get("cases").unwrap().array().unwrap();
    let mut driven = BTreeSet::new();
    let mut deviations = BTreeSet::new();
    for case in cases {
        let id = text(case, "case_id").strip_prefix("F06-").unwrap();
        if !DRIVEN.split_whitespace().any(|d| d == id) {
            continue;
        }
        let command = command(case.get("input").unwrap());
        let expected = case.get("expected").unwrap();
        let result = create_plate(&command, &|_| false, &no_materials);
        if DEVIATIONS.contains(&id) {
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::UnsupportedGeometry,
                "{id}"
            );
            assert_ne!(
                expected.get("f04_code").ok().and_then(|v| v.string().ok()),
                Some("UNSUPPORTED_GEOMETRY")
            );
            println!("DEVIATION F06-{id}: actual UNSUPPORTED_GEOMETRY differs from fixture");
            deviations.insert(id);
        } else if let Ok(code) = expected.get("f04_code") {
            if id == "POS-56" {
                use steel::engine::plate_draft::{PlateDraft, create_plate_draft};
                let PlateDraft::Unsupported(plate) =
                    create_plate_draft(&command, &|_| false, &no_materials).unwrap()
                else {
                    panic!("valid near-planar input must remain authored-only");
                };
                assert_eq!(plate.authored.to_command(), command);
                assert_eq!(plate.reason, "contour_not_exactly_planar");
            }
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::parse(code.string().unwrap()).unwrap(),
                "{id}"
            );
            println!("PARTIAL F06-{id}: F04 code checked; geometry diagnostic/status not exposed");
        } else {
            assert_eq!(text(expected, "status"), "ok", "F06-{id}");
            let out = result.unwrap_or_else(|e| panic!("F06-{id}: {e}"));
            check_success(id, case.get("input").unwrap(), &command, &out, expected);
            println!(
                "{} F06-{id}: see evidence note for asserted fields",
                if id == "POS-02" { "PASSED" } else { "PARTIAL" }
            );
        }
        assert!(driven.insert(id));
    }
    assert_eq!(driven.len(), DRIVEN.split_whitespace().count());
    assert_eq!(deviations, DEVIATIONS.iter().copied().collect());
}
