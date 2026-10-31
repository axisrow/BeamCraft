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
        drafting: Vec::new(),
        views: Vec::new(),
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

#[test]
fn resolution_refuses_a_mismatched_snapshot_before_trying_to_find_geometry() {
    use beamcraft_engine::drawing::projection::{
        self, BodyProjection, DimensionProjection, UnresolvedReason,
    };
    use beamcraft_engine::{Limits, ViewPin, ViewSnapshot};
    let drawing = Drawing::admit(spec()).unwrap();
    let pin = drawing.spec().source;
    let snapshot = ViewSnapshot::build(
        ViewPin::new(pin.manifest, pin.generation),
        vec![],
        &Limits::default(),
    )
    .unwrap();
    let missing = projection::resolve(&drawing, &snapshot).unwrap();
    assert_eq!(
        missing.body,
        BodyProjection::Unresolved(UnresolvedReason::MissingOwner)
    );
    assert_eq!(
        missing.dimension,
        DimensionProjection::Unresolved(UnresolvedReason::MissingOwner)
    );
    for other_pin in [
        ViewPin::new(ContentDigest::from_bytes([9; 32]), pin.generation),
        ViewPin::new(pin.manifest, pin.generation + 1),
    ] {
        let other = ViewSnapshot::build(other_pin, vec![], &Limits::default()).unwrap();
        assert_eq!(
            projection::resolve(&drawing, &other).unwrap_err().code,
            ErrorCode::StaleWorkspace
        );
    }
}

#[test]
fn drawn_objects_are_bounded_distinct_and_never_document_object_ids() {
    let entity = |text: &str, b: [f64; 2]| DraftingSpec {
        id: DraftingEntityId::parse(text).unwrap(),
        geometry: DraftingGeometry::Line { a: [0.0, 0.0], b },
    };
    const D: &str = "01900000-0000-7000-8000-000000000005";
    let admitted = Drawing::admit(DrawingSpec {
        drafting: vec![entity(D, [10.0, 0.0])],
        ..spec()
    })
    .unwrap();
    assert_eq!(admitted.drafting().len(), 1);
    assert_eq!(admitted.drafting()[0].id.to_string(), D);
    let emptied = admitted.clone().with_drafting(Vec::new()).unwrap();
    assert!(emptied.drafting().is_empty());

    // Identity collisions with the document objects and between entities refuse.
    let clash = Drawing::admit(DrawingSpec {
        drafting: vec![entity(C, [10.0, 0.0])],
        ..spec()
    })
    .unwrap_err();
    assert_eq!(clash.code, ErrorCode::DuplicateId);
    assert!(
        admitted
            .clone()
            .with_drafting(vec![entity(D, [4.0, 0.0]), entity(D, [5.0, 0.0])])
            .is_err()
    );

    // More than the entity cap refuses without partial state.
    let crowd = (0..=MAX_DRAFTING_ENTITIES as u64)
        .map(|index| entity(&format!("01990000-0000-7000-8000-{index:012}"), [10.0, 0.0]))
        .collect();
    assert_eq!(
        admitted.with_drafting(crowd).unwrap_err().code,
        ErrorCode::LimitExceeded
    );

    // Geometry bounds still run through the same admission.
    let degenerate = Drawing::admit(DrawingSpec {
        drafting: vec![entity(D, [0.0, 0.0])],
        ..spec()
    })
    .unwrap_err();
    assert_eq!(degenerate.code, ErrorCode::InvalidArgument);
}

#[test]
fn placed_views_are_bounded_distinct_and_never_document_or_drawn_object_ids() {
    let view = |text: &str, scale: f64, origin: [f64; 2]| ViewSpec {
        id: DrawingViewId::parse(text).unwrap(),
        plane: Plane::Top,
        scale_denominator: scale,
        paper_origin_mm: origin,
    };
    const TOP: &str = "01900000-0000-7000-8000-000000000006";
    const OTHER: &str = "01900000-0000-7000-8000-000000000007";
    const D: &str = "01900000-0000-7000-8000-000000000005";
    let entity = |text: &str| DraftingSpec {
        id: DraftingEntityId::parse(text).unwrap(),
        geometry: DraftingGeometry::Line {
            a: [0.0, 0.0],
            b: [10.0, 0.0],
        },
    };
    let with = |views: Vec<ViewSpec>| Drawing::admit(DrawingSpec { views, ..spec() });
    let admitted = with(vec![view(TOP, 50.0, [10.0, -20.0])]).unwrap();
    assert_eq!(admitted.views().len(), 1);
    assert_eq!(admitted.views()[0].id.to_string(), TOP);

    // Paper bounds apply to every placed view exactly as to the primary view.
    for bad in [f64::NAN, f64::INFINITY, 0.0, 0.999, 10_000.01] {
        assert!(with(vec![view(TOP, bad, [0.0, 0.0])]).is_err());
    }
    for bad_origin in [[10_000.01, 0.0], [0.0, -10_000.01]] {
        assert!(with(vec![view(TOP, 50.0, bad_origin)]).is_err());
    }

    // Identity collisions refuse: with the document objects, with drawn objects, pairwise.
    assert_eq!(
        with(vec![view(B, 50.0, [0.0, 0.0])]).unwrap_err().code,
        ErrorCode::DuplicateId
    );
    assert_eq!(
        Drawing::admit(DrawingSpec {
            views: vec![view(D, 50.0, [0.0, 0.0])],
            drafting: vec![entity(D)],
            ..spec()
        })
        .unwrap_err()
        .code,
        ErrorCode::DuplicateId
    );
    assert!(
        with(vec![
            view(TOP, 50.0, [0.0, 0.0]),
            view(TOP, 60.0, [1.0, 0.0])
        ])
        .is_err()
    );

    // The cap refuses without partial state.
    let crowd = (0..=MAX_VIEWS as u64)
        .map(|index| {
            view(
                &format!("01990000-0000-7000-8000-{index:012}"),
                50.0,
                [0.0, 0.0],
            )
        })
        .collect();
    assert_eq!(with(crowd).unwrap_err().code, ErrorCode::LimitExceeded);

    // Placed views and drawn objects share the one closed intent page: sixteen views leave
    // room for exactly forty-three drawn objects beside the fixed prims and their scopes.
    let both = |views: usize, drafting: usize| {
        let mut s = spec();
        s.views = (0..views as u64)
            .map(|index| {
                view(
                    &format!("01990000-0000-7000-8000-{index:012}"),
                    50.0,
                    [0.0, 0.0],
                )
            })
            .collect();
        s.drafting = (0..drafting as u64)
            .map(|index| entity(&format!("019a0000-0000-7000-8000-{index:012}")))
            .collect();
        Drawing::admit(s)
    };
    assert!(both(16, 43).is_ok());
    assert_eq!(both(16, 44).unwrap_err().code, ErrorCode::LimitExceeded);

    // Replacement keeps every other object and its identity, cross-checked both ways.
    let mut s = spec();
    s.drafting = vec![entity(D)];
    let editing = Drawing::admit(s).unwrap();
    assert_eq!(
        editing
            .clone()
            .with_views(vec![view(D, 50.0, [0.0, 0.0])])
            .unwrap_err()
            .code,
        ErrorCode::DuplicateId
    );
    let replaced = editing
        .with_views(vec![
            view(TOP, 50.0, [10.0, 20.0]),
            view(OTHER, 100.0, [0.0, 0.0]),
        ])
        .unwrap();
    assert_eq!(replaced.views().len(), 2);
    assert_eq!(replaced.spec().source, spec().source);
    assert_eq!(replaced.spec().dimension, spec().dimension);
    assert_eq!(replaced.drafting().len(), 1);
    assert_eq!(
        replaced.with_drafting(vec![entity(TOP)]).unwrap_err().code,
        ErrorCode::DuplicateId
    );
}

#[test]
fn placed_views_resolve_from_the_document_pin_and_foreign_views_refuse() {
    use beamcraft_engine::drawing::projection::{self, BodyProjection, UnresolvedReason};
    use beamcraft_engine::{Limits, ViewPin, ViewSnapshot};
    let view = |text: &str, plane: Plane| ViewSpec {
        id: DrawingViewId::parse(text).unwrap(),
        plane,
        scale_denominator: 50.0,
        paper_origin_mm: [10.0, 20.0],
    };
    const TOP: &str = "01900000-0000-7000-8000-000000000006";
    let mut s = spec();
    s.views = vec![view(TOP, Plane::Top)];
    let drawing = Drawing::admit(s).unwrap();
    let pin = drawing.spec().source;
    let snapshot = ViewSnapshot::build(
        ViewPin::new(pin.manifest, pin.generation),
        vec![],
        &Limits::default(),
    )
    .unwrap();
    // A placed view resolves exactly like the primary view; an absent owner is explicitly
    // unresolved, never an error.
    for placed in drawing.views() {
        assert_eq!(
            projection::resolve_view(&drawing, placed, &snapshot).unwrap(),
            BodyProjection::Unresolved(UnresolvedReason::MissingOwner)
        );
    }
    // A view that is not part of the document refuses before any geometry work.
    let foreign = view(TOP, Plane::Front);
    assert_eq!(
        projection::resolve_view(&drawing, &foreign, &snapshot)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    // A changed source pin marks the placed view stale together with the document.
    let moved = ViewSnapshot::build(
        ViewPin::new(pin.manifest, pin.generation + 1),
        vec![],
        &Limits::default(),
    )
    .unwrap();
    for placed in drawing.views() {
        assert_eq!(
            projection::resolve_view(&drawing, placed, &moved)
                .unwrap_err()
                .code,
            ErrorCode::StaleWorkspace
        );
    }
}
