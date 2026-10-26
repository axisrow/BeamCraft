// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Contributor-original synthetic shapes and independently derived expected coordinates.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_engine::{
    element::{ElementId, ElementKind},
    limits::Limits,
    query::{
        feature::{FeatureGeometry, FeatureId, FeatureKind, list_features, lookup_feature},
        pick::{PickQuery, PointPick, RayPick, SnapKind, SnapQuery, pick, snap},
        snapshot::{ViewPin, ViewSnapshot},
    },
    scene::SceneElement,
    solid::SolidMesh,
};
use beamcraft_geom::{Point3, Translation3};
use beamcraft_kernel::{
    RectangularPlate,
    ops::{Body, HoleCutter, through_hole},
    profile::{Cavity, CurveSpec, Point2, extrude},
    rectangular_plate,
    section::{Representation, SectionMember, section_member},
};
use beamcraft_model::{
    digest::Sha256,
    section::{ParametricSection, SectionFamily, SectionShape},
};

const ID: &str = "01990000-0000-7000-8000-000000000001";

fn snapshot(solid: &SolidMesh, kind: ElementKind, id: &str, at: [f64; 3]) -> ViewSnapshot {
    ViewSnapshot::build(
        ViewPin::new(Sha256::digest(b"nonbox").unwrap(), 1),
        vec![SceneElement::new(
            ElementId::parse(id).unwrap(),
            kind,
            Translation3::new(at[0], at[1], at[2]).unwrap(),
            solid.scene_mesh(),
        )],
        &Limits::default(),
    )
    .unwrap()
}

fn surface_picks(s: &ViewSnapshot, point: [f64; 3], prefix: &str) {
    for query in [
        PickQuery::Point(PointPick::new(point, vec![FeatureKind::Face])),
        PickQuery::Ray(RayPick::new(
            [point[0], point[1], point[2] + 100.0],
            [0.0, 0.0, -1.0],
            vec![FeatureKind::Face],
        )),
    ] {
        let hits = pick(s, &query).unwrap();
        assert!(!hits.hits.is_empty());
        let hit = &hits.hits[0];
        let id = hit.feature_id.as_ref().unwrap();
        assert!(id.to_string().starts_with(prefix), "{id}");
        assert!(
            hit.point_world_mm
                .iter()
                .zip(point)
                .all(|(a, b)| (a - b).abs() < 1e-8)
        );
        assert!(matches!(
            lookup_feature(s, &id.to_string()).unwrap().world,
            FeatureGeometry::Surface { .. }
        ));
    }
}

#[test]
fn contour_roles_resolve_pick_snap_and_survive_bundle_move_and_copy() {
    let extrusion = extrude(
        &[
            Point2::new(0.0, 0.0),
            Point2::new(200.0, 0.0),
            Point2::new(0.0, 100.0),
        ],
        &[],
        10.0,
    )
    .unwrap();
    let solid = SolidMesh::from_extrusion(&extrusion).unwrap();
    let original = snapshot(&solid, ElementKind::Plate, ID, [0.0; 3]);
    surface_picks(
        &original,
        [20.0, 20.0, 10.0],
        &format!("fid1:{ID}:extrusion:"),
    );
    let edge = pick(
        &original,
        &PickQuery::Point(PointPick::new([100.0, 0.0, 10.0], vec![FeatureKind::Edge])),
    )
    .unwrap();
    assert_eq!(
        edge.hits[0].feature_id.as_ref().unwrap().local_id(),
        "extrusion:e.z1.o0"
    );
    let vertex = format!("fid1:{ID}:extrusion:v.z1.o1");
    assert_eq!(
        lookup_feature(&original, &vertex).unwrap().world,
        FeatureGeometry::Vertex {
            position: [200.0, 0.0, 10.0]
        }
    );
    let snap = snap(
        &original,
        &SnapQuery {
            point_mm: [200.0, 0.0, 10.0],
            radius_mm: 0.01,
            kinds: vec![SnapKind::Vertex],
            max_hits: 16,
        },
    )
    .unwrap();
    assert_eq!(
        snap.candidates[0].feature_id.as_ref().unwrap().to_string(),
        vertex
    );
    let moved = snapshot(&solid, ElementKind::Plate, ID, [30.0, 40.0, 50.0]);
    assert_eq!(
        lookup_feature(&moved, &vertex).unwrap().world,
        FeatureGeometry::Vertex {
            position: [230.0, 40.0, 60.0]
        }
    );
    let copy = snapshot(
        &solid,
        ElementKind::Plate,
        "01990000-0000-7000-8000-000000000002",
        [0.0; 3],
    );
    let old = list_features(&original, &ElementId::parse(ID).unwrap()).unwrap();
    let new = list_features(&copy, &copy.elements()[0].id).unwrap();
    assert_eq!(old.len(), 20); // triangular prism: 5 faces, 9 edges, 6 vertices
    assert!(
        old.iter()
            .zip(&new)
            .all(|(a, b)| a.local_id() == b.local_id() && a != b)
    );
    let roundtrip =
        ViewSnapshot::from_bundle(&original.to_bundle().unwrap(), &Limits::default()).unwrap();
    assert_eq!(
        lookup_feature(&roundtrip, &vertex),
        lookup_feature(&original, &vertex)
    );
    let outside = pick(
        &original,
        &PickQuery::Point(PointPick::new([190.0, 90.0, 10.0], vec![FeatureKind::Face])),
    )
    .unwrap();
    assert!(outside.hits.is_empty());
}

#[test]
fn polygon_and_round_holes_expose_surviving_faces_without_filling_the_hole() {
    let plate = rectangular_plate(RectangularPlate {
        width_mm: 200.0,
        length_mm: 100.0,
        thickness_mm: 10.0,
    })
    .unwrap();
    for cavity in [
        Cavity::polygon(vec![
            Point2::new(80.0, 30.0),
            Point2::new(80.0, 70.0),
            Point2::new(120.0, 70.0),
            Point2::new(120.0, 30.0),
        ]),
        Cavity::circle(Point2::new(100.0, 50.0), 20.0, CurveSpec::DEFAULT).unwrap(),
    ] {
        let op = through_hole(
            &Body::from_plate(&plate).unwrap(),
            &HoleCutter::new(cavity, -10.0, 20.0, 7),
        )
        .unwrap();
        let s = snapshot(
            &SolidMesh::from_operation(&op),
            ElementKind::Plate,
            ID,
            [0.0; 3],
        );
        surface_picks(&s, [20.0, 20.0, 10.0], &format!("fid1:{ID}:plate:"));
        let query = PickQuery::Ray(RayPick::new(
            [100.0, 50.0, 100.0],
            [0.0, 0.0, -1.0],
            vec![FeatureKind::Face],
        ));
        assert!(pick(&s, &query).unwrap().hits.is_empty());
        let query = PickQuery::Point(PointPick::new([100.0, 50.0, 10.0], vec![FeatureKind::Face]));
        assert!(pick(&s, &query).unwrap().hits.is_empty());
        let wall = PickQuery::Ray(RayPick::new(
            [100.0, 50.0, 5.0],
            [1.0, 0.0, 0.0],
            vec![FeatureKind::Face],
        ));
        let hits = pick(&s, &wall).unwrap();
        let id = hits.hits[0].feature_id.as_ref().unwrap();
        assert!(id.local_id().starts_with("cut:f.h7"));
        assert!((hits.hits[0].point_world_mm[0] - 120.0).abs() < 1e-8);
        assert!(lookup_feature(&s, &id.to_string()).is_ok());
        assert_eq!(
            lookup_feature(&s, &format!("fid1:{ID}:plate:v.x0.y0.z1"))
                .unwrap()
                .world,
            FeatureGeometry::Vertex {
                position: [0.0, 0.0, 10.0]
            }
        );
    }
}

#[test]
fn i_section_uses_versioned_profile_roles() {
    let section = section_member(&SectionMember {
        start: Point3::ORIGIN,
        end: Point3::new(0.0, 0.0, 1000.0),
        shape: SectionShape::Parametric(
            ParametricSection::new(SectionFamily::IShape, vec![300.0, 150.0, 7.0, 10.0, 0.0])
                .unwrap(),
        ),
        representation: Representation::Detailed,
        roll_rad: None,
    })
    .unwrap();
    let s = snapshot(
        &SolidMesh::from_section(&section),
        ElementKind::Beam,
        ID,
        [0.0; 3],
    );
    assert!(
        section
            .body()
            .face_table()
            .iter()
            .all(|id| id.starts_with("profile:"))
    );
    surface_picks(&s, [50.0, 145.0, 1000.0], &format!("fid2:{ID}:profile:"));
    let vertex = format!("fid2:{ID}:profile:v.z0.o0");
    assert_eq!(
        lookup_feature(&s, &vertex).unwrap().world,
        FeatureGeometry::Vertex {
            position: [-75.0, -150.0, 0.0]
        }
    );
    assert!(lookup_feature(&s, &format!("fid1:{ID}:member:f.z1")).is_err());
    let moved = snapshot(
        &SolidMesh::from_section(&section),
        ElementKind::Beam,
        ID,
        [10.0, 20.0, 30.0],
    );
    assert_eq!(
        lookup_feature(&moved, &vertex).unwrap().world,
        FeatureGeometry::Vertex {
            position: [-65.0, -130.0, 30.0]
        }
    );
    let half = core::f64::consts::FRAC_1_SQRT_2;
    let rotated = ViewSnapshot::build(
        s.pin(),
        vec![
            SceneElement::new(
                ElementId::parse(ID).unwrap(),
                ElementKind::Beam,
                Translation3::new(0.0, 0.0, 0.0).unwrap(),
                SolidMesh::from_section(&section).scene_mesh(),
            )
            .with_orientation([0.0, 0.0, half, half]),
        ],
        &Limits::default(),
    )
    .unwrap();
    let FeatureGeometry::Vertex { position } = lookup_feature(&rotated, &vertex).unwrap().world
    else {
        panic!("vertex");
    };
    assert!(
        position
            .iter()
            .zip([150.0, -75.0, 0.0])
            .all(|(a, b)| (a - b).abs() < 1e-8)
    );
}

#[test]
fn token_families_accept_only_canonical_bounded_roles() {
    for local in [
        "extrusion:f.z0",
        "extrusion:e.z0.o2",
        "extrusion:v.z1.c2.7",
        "cut:f.h7.s3",
        "cut:f.p8",
    ] {
        let text = format!("fid1:{ID}:{local}");
        assert_eq!(FeatureId::parse(&text).unwrap().to_string(), text);
    }
    for text in [
        format!("fid1:{ID}:profile:f.o0"),
        format!("fid2:{ID}:extrusion:f.o0"),
        format!("fid2:{ID}:profile:f.o01"),
        format!("fid1:{ID}:extrusion:v.z2.o0"),
        format!("fid1:{ID}:cut:e.h7.s0"),
        format!("fid1:{ID}:cut:f.h1000"),
        format!("fid1:{ID}:extrusion:v.z0.c255.1023"),
    ] {
        assert!(FeatureId::parse(&text).is_err(), "{text}");
    }
}

#[test]
fn plane_cuts_keep_surviving_names_and_retire_clipped_edges() {
    use beamcraft_kernel::ops::{CutPlane, Keep, plane_cut};
    let plate = rectangular_plate(RectangularPlate {
        width_mm: 200.0,
        length_mm: 100.0,
        thickness_mm: 10.0,
    })
    .unwrap();
    let op = plane_cut(
        &Body::from_plate(&plate).unwrap(),
        &CutPlane::new([100.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        Keep::Negative,
        8,
    )
    .unwrap();
    let s = snapshot(
        &SolidMesh::from_operation(&op),
        ElementKind::Plate,
        ID,
        [0.0; 3],
    );
    assert!(lookup_feature(&s, &format!("fid1:{ID}:plate:v.x0.y0.z1")).is_ok());
    assert!(lookup_feature(&s, &format!("fid1:{ID}:plate:v.x1.y0.z1")).is_err());
    assert!(lookup_feature(&s, &format!("fid1:{ID}:plate:e.y0.z1")).is_err());
    let hits = pick(
        &s,
        &PickQuery::Ray(RayPick::new(
            [150.0, 50.0, 5.0],
            [-1.0, 0.0, 0.0],
            vec![FeatureKind::Face],
        )),
    )
    .unwrap();
    assert_eq!(hits.hits[0].point_world_mm, [100.0, 50.0, 5.0]);
    assert_eq!(
        hits.hits[0].feature_id.as_ref().unwrap().local_id(),
        "cut:f.p8"
    );
    let id = hits.hits[0].feature_id.as_ref().unwrap().to_string();
    let back = ViewSnapshot::from_bundle(&s.to_bundle().unwrap(), &Limits::default()).unwrap();
    assert_eq!(lookup_feature(&s, &id), lookup_feature(&back, &id));
}
