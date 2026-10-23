// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D01 contract admission and exact source-pin semantics, independent of geometry/persistence.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_engine::drawing::*;
use beamcraft_engine::query::feature::FeatureId;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::*;

const A: &str = "01900000-0000-7000-8000-000000000001";
const B: &str = "01900000-0000-7000-8000-000000000002";
const C: &str = "01900000-0000-7000-8000-000000000003";
const MEMBER: &str = "01900000-0000-7000-8000-000000000004";

fn spec() -> DrawingSpec {
    DrawingSpec {
        id: DrawingId::parse(A).unwrap(),
        name: "Member projection".into(),
        source: SourceModelPin {
            project: ProjectId::parse(A).unwrap(),
            revision: RevisionId::parse(B).unwrap(),
            manifest: ContentDigest::from_bytes([1; 32]),
            schema: ContentDigest::from_bytes([2; 32]),
            generation: 42,
        },
        member: ElementId::parse(MEMBER).unwrap(),
        view: ViewSpec {
            id: DrawingViewId::parse(B).unwrap(),
            plane: Plane::Front,
            scale_denominator: DEFAULT_SCALE_DENOMINATOR,
            paper_origin_mm: [25.0, 40.0],
        },
        dimension: DimensionSpec {
            id: DimensionId::parse(C).unwrap(),
            anchors: ["v.x0.y0.z0", "v.x0.y0.z1"]
                .map(|role| FeatureId::parse(&format!("fid1:{MEMBER}:member:{role}")).unwrap()),
            paper_offset_mm: -10.0,
            decimal_places: 1,
        },
    }
}

#[test]
fn typed_document_ids_are_full_uuidv7_and_distinct_objects_cannot_share_one_uuid() {
    assert_eq!(DrawingId::parse(A).unwrap().to_string(), A);
    assert_eq!(DrawingViewId::parse(B).unwrap().to_string(), B);
    assert_eq!(DimensionId::parse(C).unwrap().to_string(), C);
    assert_eq!(
        DocumentVersionId::parse(MEMBER).unwrap().to_string(),
        MEMBER
    );
    for bad in [
        "1",
        "01900000-0000-4000-8000-000000000001",
        "01900000-0000-7000-0000-000000000001",
        "01900000-0000-7000-8000-00000000000A",
    ] {
        assert!(DrawingId::parse(bad).is_err());
        assert!(DrawingViewId::parse(bad).is_err());
        assert!(DimensionId::parse(bad).is_err());
        assert!(DocumentVersionId::parse(bad).is_err());
    }
    for pair in 0..3 {
        let mut s = spec();
        match pair {
            0 => s.view.id = DrawingViewId::from_uuid(s.id.uuid()),
            1 => s.dimension.id = DimensionId::from_uuid(s.id.uuid()),
            _ => s.dimension.id = DimensionId::from_uuid(s.view.id.uuid()),
        }
        assert_eq!(Drawing::admit(s).unwrap_err().code, ErrorCode::DuplicateId);
    }
}

#[test]
fn admission_accepts_only_member_vertex_references_of_the_selected_owner() {
    let admitted = Drawing::admit(spec()).unwrap();
    assert_eq!(admitted.spec(), &spec());
    for text in [
        format!("fid1:{MEMBER}:plate:v.x0.y0.z0"),
        format!("fid1:{MEMBER}:member:f.x0"),
        format!("fid1:{MEMBER}:member:e.x0.y0"),
        format!("fid1:{A}:member:v.x0.y0.z0"),
    ] {
        let mut s = spec();
        s.dimension.anchors[0] = FeatureId::parse(&text).unwrap();
        assert_eq!(
            Drawing::admit(s).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
    let mut s = spec();
    s.dimension.anchors[1] = s.dimension.anchors[0].clone();
    assert_eq!(Drawing::admit(s).unwrap_err().field, "anchors");
    for bad in [
        "0",
        "triangle:0",
        "vertex:1",
        "fid1:short:member:v.x0.y0.z0",
    ] {
        assert!(FeatureId::parse(bad).is_err());
    }
}

#[test]
fn paper_settings_and_labels_are_bounded_before_resolution() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.0,
        -1.0,
        0.999,
        10_000.001,
    ] {
        let mut s = spec();
        s.view.scale_denominator = value;
        let code = Drawing::admit(s).unwrap_err().code;
        assert_eq!(
            code,
            if value.is_finite() {
                ErrorCode::InvalidArgument
            } else {
                ErrorCode::NonFiniteNumber
            }
        );
    }
    for scale in [1.0, 100.0, MAX_SCALE_DENOMINATOR] {
        let mut s = spec();
        s.view.scale_denominator = scale;
        s.view.paper_origin_mm = [-MAX_PAPER_MM, MAX_PAPER_MM];
        s.dimension.paper_offset_mm = MAX_PAPER_MM;
        s.dimension.decimal_places = 3;
        assert!(Drawing::admit(s).is_ok());
    }
    for bad in [f64::NAN, f64::INFINITY, -10_000.001, 10_000.001] {
        for axis in 0..2 {
            let mut s = spec();
            s.view.paper_origin_mm[axis] = bad;
            assert!(Drawing::admit(s).is_err());
        }
        let mut s = spec();
        s.dimension.paper_offset_mm = bad;
        assert!(Drawing::admit(s).is_err());
    }
    for bad in [String::new(), "bad\nname".into(), "x".repeat(257)] {
        let mut s = spec();
        s.name = bad;
        assert!(Drawing::admit(s).is_err());
    }
    let mut s = spec();
    s.dimension.decimal_places = 4;
    assert_eq!(Drawing::admit(s).unwrap_err().field, "decimal_places");
    for plane in [Plane::Front, Plane::Top, Plane::Right] {
        assert_eq!(Plane::parse(plane.as_str()), Some(plane));
    }
    assert_eq!(Plane::parse("camera"), None);
}

#[test]
fn freshness_compares_every_source_coordinate_and_missing_context_is_never_current() {
    let source = spec().source;
    assert_eq!(source.freshness(LiveSource::Unknown), Freshness::Unknown);
    assert_eq!(
        source.freshness(LiveSource::Unavailable),
        Freshness::Unavailable
    );
    assert_eq!(
        source.freshness(LiveSource::Available(source)),
        Freshness::Current
    );
    for field in 0..5 {
        let mut changed = source;
        match field {
            0 => changed.project = ProjectId::parse(C).unwrap(),
            1 => changed.revision = RevisionId::parse(C).unwrap(),
            2 => changed.manifest = ContentDigest::from_bytes([3; 32]),
            3 => changed.schema = ContentDigest::from_bytes([3; 32]),
            _ => changed.generation += 1,
        }
        assert_eq!(
            source.freshness(LiveSource::Available(changed)),
            Freshness::Stale
        );
    }
}

#[test]
fn refresh_is_explicit_and_preserves_object_identity_and_references() {
    let old = Drawing::admit(spec()).unwrap();
    let mut next_pin = old.spec().source;
    next_pin.revision = RevisionId::parse(C).unwrap();
    next_pin.manifest = ContentDigest::from_bytes([9; 32]);
    next_pin.generation += 1;
    let next = old.refreshed(next_pin);
    assert_eq!(old.spec(), &spec());
    assert_eq!(next.spec().source, next_pin);
    assert_eq!(next.spec().id, old.spec().id);
    assert_eq!(next.spec().name, old.spec().name);
    assert_eq!(next.spec().member, old.spec().member);
    assert_eq!(next.spec().view, old.spec().view);
    assert_eq!(next.spec().dimension, old.spec().dimension);
}
