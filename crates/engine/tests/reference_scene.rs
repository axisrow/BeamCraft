// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic reference geometry, MPL-2.0.

//! Headless R01 attach -> reference representation -> pinned view scene, without a USD/UI edge.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_engine::query::pick::{PickQuery, PointPick, pick};
use beamcraft_engine::reference::{ReferenceRecord, attach, correct_units, move_reference};
use beamcraft_engine::scene::{ReferenceGeometry, reference_element};
use beamcraft_engine::{ElementId, ElementKind, Limits, ViewPin, ViewSettings, ViewSnapshot, diff};
use beamcraft_kernel::trimesh::{MAX_TRIANGLES, MAX_VERTICES, TriMesh};
use beamcraft_model::command::Placement;
use beamcraft_model::digest::Sha256;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::reference::{
    AttachReference, CorrectReferenceUnits, MoveReference, SourceFormat, SourcePin,
    UnitsInterpretation, UpAxis,
};

fn attached() -> ReferenceRecord {
    let bytes = b"#usda 1.0\n(defaultPrim = \"Site\")\ndef Xform \"Site\" {}\n";
    attach(
        &AttachReference {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000137").unwrap(),
            name: "Original synthetic source".into(),
            placement: Placement {
                translation_mm: [10.0, 20.0, 30.0],
                ..Placement::IDENTITY
            },
            source: SourcePin {
                digest: Sha256::digest(bytes).unwrap(),
                bytes: bytes.len() as u64,
                format: SourceFormat::Usda,
                default_prim: "Site".into(),
                version: 1,
                label: "site.usda".into(),
                declared_meters_per_unit: Some(1.0),
                declared_up_axis: Some(UpAxis::Y),
            },
        },
        &|_| false,
    )
    .unwrap()
    .record
}

fn snapshot(
    record: &ReferenceRecord,
    geometry: &[ReferenceGeometry],
    generation: u64,
) -> ViewSnapshot {
    let element = reference_element(record, geometry).unwrap().unwrap();
    ViewSnapshot::build(
        ViewPin::new(Sha256::digest(b"revision").unwrap(), generation),
        vec![element],
        &Limits::default(),
    )
    .unwrap()
}

#[test]
fn an_attached_wrapper_is_one_reference_element_with_all_its_surfaces() {
    let record = attached();
    let geometry = [
        ReferenceGeometry::cube(2.0, [0.0; 3]).unwrap(),
        ReferenceGeometry::cube(2.0, [4.0, 0.0, 0.0]).unwrap(),
    ];
    let scene = snapshot(&record, &geometry, 1);
    assert_eq!(scene.len(), 1);
    let entry = &scene.elements()[0];
    assert_eq!(entry.id, record.id);
    assert_eq!(entry.kind, ElementKind::Reference);
    assert_eq!(entry.placement_mm, [10.0, 20.0, 30.0]);
    // Declared metres and Y-up alone must not silently reinterpret the foreign coordinates.
    assert_eq!(entry.full.world_bounds.min, [9.0, 19.0, 29.0]);
    assert_eq!(entry.full.world_bounds.max, [15.0, 21.0, 31.0]);
    assert_eq!(entry.full, entry.simplified);
    assert_eq!(scene.meshes()[0].vertex_count(), 16);
    assert_eq!(scene.meshes()[0].triangle_count(), 24);
    assert_eq!(
        ViewSnapshot::from_bundle(&scene.to_bundle().unwrap(), &Limits::default()).unwrap(),
        scene
    );
    let hidden = ViewSettings::default();
    let hidden = ViewSettings {
        hidden_kinds: hidden.hidden_kinds.hiding(ElementKind::Reference),
        ..hidden
    };
    assert!(scene.project(&hidden).unwrap().is_empty());
    // Displaying a foreign surface must not accidentally give it our box engineering features.
    let result = pick(
        &scene,
        &PickQuery::Point(PointPick::new(
            [10.0, 20.0, 30.0],
            vec![beamcraft_engine::query::feature::FeatureKind::Face],
        )),
    )
    .unwrap();
    assert!(result.hits.is_empty());
    assert_eq!(result.skipped, [record.id]);

    let moved = move_reference(
        &record,
        &MoveReference {
            target_id: record.id,
            placement: Placement {
                translation_mm: [100.0, 200.0, 300.0],
                ..Placement::IDENTITY
            },
        },
    )
    .unwrap()
    .record;
    let next = snapshot(&moved, &geometry, 2);
    let change = diff(&scene, &next);
    assert_ne!(scene.token(), next.token());
    assert_eq!(next.meshes(), scene.meshes());
    assert!(change.new_meshes.is_empty());
    assert!(change.changes[0].fields.placement);
    assert!(!change.changes[0].fields.shape);
    assert_eq!(
        next.elements()[0].full.world_bounds.min,
        [99.0, 199.0, 299.0]
    );
    assert_eq!(moved.source, record.source);
}

#[test]
fn explicit_r01_units_correction_maps_points_before_the_wrapper_translation() {
    let record = attached();
    let corrected = correct_units(
        &record,
        &CorrectReferenceUnits {
            target_id: record.id,
            interpretation: Some(UnitsInterpretation {
                meters_per_unit: 1.0,
                up_axis: UpAxis::Y,
            }),
        },
    )
    .unwrap()
    .record;
    let geometry = ReferenceGeometry::cube(2.0, [3.0, 4.0, 5.0]).unwrap();
    let scene = snapshot(&corrected, &[geometry], 2);
    let bounds = scene.engineering_bounds().unwrap();
    assert_eq!(bounds.min, [2010.0, -5980.0, 3030.0]);
    assert_eq!(bounds.max, [4010.0, -3980.0, 5030.0]);
    assert_eq!(corrected.source, record.source);
    assert_eq!(record.units, None);
}

#[test]
fn triangle_winding_is_preserved_or_reversed_only_when_requested() {
    let mut mesh = ReferenceGeometry {
        points: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
        face_vertex_counts: vec![3],
        face_vertex_indices: vec![0, 1, 2],
        translation: [0.0; 3],
        left_handed: false,
    };
    let full = snapshot(&attached(), &[mesh.clone()], 1);
    assert_eq!(
        TriMesh::from_blob(full.meshes()[0].bytes())
            .unwrap()
            .triangles(),
        [[0, 1, 2]]
    );
    mesh.left_handed = true;
    let reversed = snapshot(&attached(), &[mesh], 1);
    assert_eq!(
        TriMesh::from_blob(reversed.meshes()[0].bytes())
            .unwrap()
            .triangles(),
        [[2, 1, 0]]
    );
}

#[test]
fn malformed_foreign_geometry_never_becomes_a_partial_mesh() {
    let record = attached();
    let cube = ReferenceGeometry::cube(2.0, [0.0; 3]).unwrap();
    let mut cases = Vec::new();
    let mut bad = cube.clone();
    bad.face_vertex_indices[0] = u32::MAX;
    cases.push((bad, ErrorCode::InvalidArgument));
    let mut bad = cube.clone();
    bad.face_vertex_indices.pop();
    cases.push((bad, ErrorCode::InvalidArgument));
    let mut bad = cube.clone();
    bad.face_vertex_indices.push(0);
    cases.push((bad, ErrorCode::InvalidArgument));
    let mut bad = cube.clone();
    bad.points.resize(MAX_VERTICES + 1, [0.0; 3]);
    cases.push((bad, ErrorCode::LimitExceeded));
    let mut bad = cube.clone();
    bad.face_vertex_counts = vec![4; MAX_TRIANGLES / 2 + 1];
    cases.push((bad, ErrorCode::LimitExceeded));
    for (bad, expected) in cases {
        assert_eq!(
            reference_element(&record, &[cube.clone(), bad])
                .unwrap_err()
                .code,
            expected
        );
    }
    let mut many_faces = cube.clone();
    many_faces.face_vertex_counts = vec![0; MAX_TRIANGLES / 2 + 1];
    assert_eq!(
        reference_element(&record, &[many_faces.clone(), many_faces])
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded,
        "the aggregate face budget is checked before scanning faces"
    );
    assert!(reference_element(&record, &[]).unwrap().is_none());
}

#[test]
fn a_display_approximates_or_leaves_out_what_the_measurement_would_refuse() {
    let record = attached();
    let cube = ReferenceGeometry::cube(2.0, [0.0; 3]).unwrap();
    let triangles = |geometry: &[ReferenceGeometry]| {
        let scene = snapshot(&record, geometry, 1);
        TriMesh::from_blob(scene.meshes()[0].bytes())
            .unwrap()
            .triangles()
            .len()
    };
    let alone = triangles(std::slice::from_ref(&cube));
    // A pentagon is fan-triangulated; a concave or non-planar quad is too.
    let pentagon = ReferenceGeometry {
        points: vec![
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [1.0, 3.0, 0.0],
            [-1.0, 1.0, 0.0],
        ],
        face_vertex_counts: vec![5],
        face_vertex_indices: vec![0, 1, 2, 3, 4],
        translation: [0.0; 3],
        left_handed: false,
    };
    assert_eq!(triangles(&[pentagon]), 3);
    let mut concave = concave_quad();
    assert_eq!(triangles(&[concave.clone()]), 2);
    concave.points[0][2] += 0.5;
    assert_eq!(triangles(&[concave]), 2);
    // A surface that cannot be placed is left out, the rest is drawn.
    let mut nan = cube.clone();
    nan.points[0][0] = f64::NAN;
    let mut far = cube.clone();
    far.points[0][0] = f64::MAX;
    assert_eq!(triangles(&[cube.clone(), nan, far]), alone);
    // The wrapper rotation is baked into the vertices instead of being refused: a half turn about
    // Z sends the cube centred at x = 4 to x = -4, so its box spans -5..-3 before the translation.
    let mut rotated = record.clone();
    rotated.placement.orientation_xyzw = [0.0, 0.0, 1.0, 0.0];
    let scene = snapshot(
        &rotated,
        &[ReferenceGeometry::cube(2.0, [4.0, 0.0, 0.0]).unwrap()],
        1,
    );
    let bounds = scene.engineering_bounds().unwrap();
    assert!(
        (bounds.min[0] - (10.0 - 5.0)).abs() < 1e-6,
        "{:?}",
        bounds.min
    );
    assert!(
        (bounds.max[0] - (10.0 - 3.0)).abs() < 1e-6,
        "{:?}",
        bounds.max
    );
}

fn concave_quad() -> ReferenceGeometry {
    ReferenceGeometry {
        points: vec![
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.5, 0.5, 0.0],
            [0.0, 2.0, 0.0],
        ],
        face_vertex_counts: vec![4],
        face_vertex_indices: vec![0, 1, 2, 3],
        translation: [0.0; 3],
        left_handed: false,
    }
}
