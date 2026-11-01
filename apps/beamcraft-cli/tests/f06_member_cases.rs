// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original adapter of the frozen F06 fixture; no copied third-party material.

//! K2 member projections. A projection passing does not mean the entire contract case passes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_json::{Json, Limits};
use beamcraft_model::command::Placement;
use beamcraft_model::digest::Sha256;
use beamcraft_model::id::ElementId;
use std::collections::{BTreeMap, BTreeSet};
use steel::engine::command::{MaterialInfo, create_plate, no_materials};
use steel::engine::member::{CreateMemberOutcome, ProfileInfo, create_straight_member};
use steel::model::command::{CreatePlate, CreateStraightMember, MemberKind};

const CASES: &str = "POS-04 POS-05 POS-06 POS-07 POS-08 POS-09 POS-10 POS-12 POS-13 POS-14 \
    POS-15 POS-41 POS-42 POS-46 POS-50 POS-51 POS-52 POS-57 POS-58 \
    NEG-04 NEG-05 NEG-06 NEG-07 NEG-43 NEG-44 NEG-47 NEG-48 NEG-51 NEG-52 NEG-53";

fn number(value: &Json) -> f64 {
    match value {
        Json::Number(_) => value.float().unwrap(),
        Json::String(s) => match s.as_str() {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            _ => s.split_once('/').map_or_else(
                || s.parse().unwrap(),
                |(n, d)| n.parse::<f64>().unwrap() / d.parse::<f64>().unwrap(),
            ),
        },
        _ => panic!("not a fixture number: {value:?}"),
    }
}

fn vector(value: &Json) -> [f64; 3] {
    value
        .array()
        .unwrap()
        .iter()
        .map(number)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

fn close(actual: f64, expected: f64, absolute: f64) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= absolute + 1e-6 * expected.abs(),
        "actual {actual}, expected {expected}"
    );
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
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}

fn member(
    input: &Json,
    body: &Json,
    density: Option<f64>,
) -> Result<CreateMemberOutcome, beamcraft_model::command::CommandError> {
    let profile = body.get("profile").unwrap();
    let shape = ProfileInfo::Rectangle {
        width_mm: number(profile.get("width_mm").unwrap()),
        height_mm: number(profile.get("height_mm").unwrap()),
        declared_area_mm2: None,
    };
    let mut placement = Placement::IDENTITY;
    if let Ok(p) = input.get("placement") {
        placement.translation_mm = vector(p.get("translation_mm").unwrap());
        placement.orientation_xyzw = p
            .get("orientation_xyzw")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(number)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
    }
    let command = CreateStraightMember {
        new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000001102").unwrap(),
        kind: MemberKind::Beam,
        name: "F06 K2 member".into(),
        class: 0,
        axis_points_mm: body
            .get("axis_points_mm")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(vector)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap(),
        profile_digest: Sha256::digest(b"F06 synthetic profile").unwrap(),
        placement,
        roll_rad: body.get("roll_rad").ok().map(number),
        section_placement: None,
        material_digest: density.map(|_| Sha256::digest(b"F06 synthetic material").unwrap()),
    };
    let out = create_straight_member(&command, &|_| false, &|_| Ok(shape.clone()), &|_| {
        Ok(MaterialInfo {
            density_kg_m3: density,
        })
    })?;
    assert_eq!(
        out.authored.to_command(),
        Some(command),
        "authored inputs changed"
    );
    Ok(out)
}

fn success(out: &CreateMemberOutcome, expected: &Json) {
    let mesh = &out.body.solid;
    let vertices = mesh.vertices();
    let triangles = mesh.triangles();
    for (key, value) in [
        ("axis_length_mm", out.body.axis_length_mm),
        ("volume_mm3", out.quantities.volume_mm3),
        ("volume_m3", out.quantities.volume_m3),
    ] {
        if let Ok(reference) = expected.get(key) {
            close(value, number(reference), 0.0);
        }
    }
    for (key, count) in [
        ("vertex_count", vertices.len()),
        ("triangle_count", triangles.len()),
    ] {
        if let Ok(reference) = expected.get(key) {
            assert_eq!(count as u64, reference.unsigned().unwrap());
        }
    }
    if let Ok(reference) = expected.get("vertices_mm") {
        assert_eq!(vertices.len(), reference.array().unwrap().len());
        for (actual, reference) in vertices.iter().zip(reference.array().unwrap()) {
            for (actual, expected) in actual.iter().zip(vector(reference)) {
                close(*actual, expected, 0.001);
            }
        }
    }
    if let Ok(bounds) = expected.get("local_bounds_mm") {
        for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
            let (low, high) = if let Ok(range) = bounds.get(name) {
                let a = range.array().unwrap();
                (number(&a[0]), number(&a[1]))
            } else {
                (
                    vector(bounds.get("min").unwrap())[axis],
                    vector(bounds.get("max").unwrap())[axis],
                )
            };
            close(
                vertices
                    .iter()
                    .map(|v| v[axis])
                    .fold(f64::INFINITY, f64::min),
                low,
                0.001,
            );
            close(
                vertices
                    .iter()
                    .map(|v| v[axis])
                    .fold(f64::NEG_INFINITY, f64::max),
                high,
                0.001,
            );
        }
    }
    // Recover the actual basis from the emitted mesh; expected bases come only from the fixture.
    let scales = [
        out.body.width_mm,
        out.body.height_mm,
        out.body.axis_length_mm,
    ];
    for ((name, vertex), scale) in [("X_m", 1), ("Y_m", 2), ("Z_m", 4)].into_iter().zip(scales) {
        let actual = subtract(vertices[vertex], vertices[0]).map(|v| v / scale);
        let reference = expected
            .get("member_frame")
            .ok()
            .and_then(|f| f.get(name).ok())
            .or_else(|| expected.get(name).ok())
            .or_else(|| expected.get(&format!("{name}_approx")).ok());
        if let Some(reference) = reference {
            for (actual, expected) in actual.into_iter().zip(vector(reference)) {
                close(actual, expected, if expected == 0.0 { 1e-6 } else { 0.0 });
            }
        }
    }
    let mut edges: BTreeMap<(u32, u32), (usize, i32)> = BTreeMap::new();
    let mut volume = 0.0;
    let mut area = 0.0;
    for &[a, b, c] in triangles {
        let [a_v, b_v, c_v] = [a, b, c].map(|i| vertices[i as usize]);
        let normal = cross(subtract(b_v, a_v), subtract(c_v, a_v));
        area += dot(normal, normal).sqrt() / 2.0;
        volume += dot(a_v, cross(b_v, c_v)) / 6.0;
        for (a, b) in [(a, b), (b, c), (c, a)] {
            let edge = edges.entry((a.min(b), a.max(b))).or_default();
            edge.0 += 1;
            edge.1 += if a < b { 1 } else { -1 };
        }
    }
    assert!(
        edges
            .values()
            .all(|&(count, directions)| count == 2 && directions == 0)
    );
    assert_eq!(vertices.len() + triangles.len(), edges.len() + 2);
    close(volume, out.quantities.volume_mm3, 0.0);
    for (key, actual) in [
        ("signed_volume_mm3", volume),
        ("surface_area_mm2", area),
        ("surface_area_m2", area / 1e6),
    ] {
        if let Ok(reference) = expected.get(key) {
            close(actual, number(reference), 0.0);
        }
    }
    if let Ok(reference) = expected.get("mass_kg") {
        close(out.quantities.mass_kg.unwrap(), number(reference), 0.0);
    }
}

#[test]
fn the_thirty_f06_member_and_mixed_cases_have_explicit_evidence() {
    let file = beamcraft_json::decode(
        include_bytes!("../../../tests/fixtures/contracts/f06-geometry-contract-cases-v0.1.json"),
        &Limits::DEFAULT,
    )
    .unwrap();
    assert_eq!(
        file.get("contract_version").unwrap().string().unwrap(),
        "0.1.0"
    );
    let mut seen = BTreeSet::new();
    let mut member_and_mixed = 0usize;
    for case in file.get("cases").unwrap().array().unwrap() {
        let id = case
            .get("case_id")
            .unwrap()
            .string()
            .unwrap()
            .strip_prefix("F06-")
            .unwrap();
        let input = case.get("input").unwrap();
        let member_case = match input.get("body") {
            Ok(body) => body.get("kind").unwrap().string().unwrap() == "member",
            Err(_) => input.get("bodies").ok().is_some_and(|bodies| {
                bodies
                    .array()
                    .unwrap()
                    .iter()
                    .any(|b| b.get("kind").unwrap().string().unwrap() == "member")
            }),
        };
        if member_case {
            member_and_mixed += 1;
        }
        if !CASES.split_whitespace().any(|wanted| wanted == id) {
            continue;
        }
        assert!(
            member_case,
            "listed case F06-{id} is no longer a member/mixed fixture case"
        );
        assert!(seen.insert(id.to_owned()));
        let expected = case.get("expected").unwrap();
        if id == "POS-14" {
            let bodies = input.get("bodies").unwrap().array().unwrap();
            let plate = &bodies[0];
            let out = create_plate(
                &CreatePlate {
                    new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000001101").unwrap(),
                    name: "F06 thin plate".into(),
                    class: 0,
                    contour_points_mm: plate
                        .get("contour_points_mm")
                        .unwrap()
                        .array()
                        .unwrap()
                        .iter()
                        .map(vector)
                        .collect(),
                    thickness_mm: number(plate.get("thickness_mm").unwrap()),
                    thickness_side: None,
                    corners: Vec::new(),
                    placement: Placement::IDENTITY,
                    material_digest: None,
                    holes_mm: Vec::new(),
                },
                &|_| false,
                &no_materials,
            )
            .unwrap();
            close(
                out.quantities.volume_mm3,
                number(expected.get("plate_volume_mm3").unwrap()),
                0.0,
            );
            close(
                out.quantities.volume_m3,
                number(expected.get("plate_volume_m3").unwrap()),
                0.0,
            );
            let out = member(input, &bodies[1], None).unwrap();
            close(
                out.body.axis_length_mm,
                number(expected.get("member_axis_length_mm").unwrap()),
                0.0,
            );
            close(
                out.quantities.volume_mm3,
                number(expected.get("member_volume_mm3").unwrap()),
                0.0,
            );
        } else {
            let density = input.get("density_kg_m3").ok().and_then(|v| v.float().ok());
            let result = member(input, input.get("body").unwrap(), density);
            if id.starts_with("NEG-") {
                assert_eq!(
                    result.unwrap_err().code.as_str(),
                    expected.get("f04_code").unwrap().string().unwrap(),
                    "F06-{id}"
                );
            } else {
                let out = result.unwrap_or_else(|e| panic!("F06-{id}: {e:?}"));
                success(&out, expected);
                if id == "POS-13" {
                    assert_eq!(out.quantities.mass_kg, None);
                    assert_eq!(
                        out.quantities.mass_status.as_str(),
                        expected
                            .get("mass_status_without_density")
                            .unwrap()
                            .string()
                            .unwrap()
                    );
                    let density = number(&input.get("density_kg_m3").unwrap().array().unwrap()[1]);
                    let with_density =
                        member(input, input.get("body").unwrap(), Some(density)).unwrap();
                    close(
                        with_density.quantities.mass_kg.unwrap(),
                        number(expected.get("mass_kg_with_density_2000").unwrap()),
                        0.0,
                    );
                    assert_eq!(
                        with_density.quantities.mass_status.as_str(),
                        expected
                            .get("mass_status_with_density_2000")
                            .unwrap()
                            .string()
                            .unwrap()
                    );
                }
            }
        }
        println!(
            "PARTIAL F06-{id}: engine/mesh projection only; no protocol or persisted-state claim"
        );
    }
    assert_eq!(seen, CASES.split_whitespace().map(str::to_owned).collect());
    assert_eq!(
        member_and_mixed, 33,
        "member/mixed fixture composition changed; update the K2 inventory and this driver"
    );
    assert_eq!(seen.len(), 30);
}
