// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Closed drawing intent: exact USD round trip, semantic admission and hostile-source refusals.

#![allow(clippy::unwrap_used)]

use beamcraft_cli::drawing::codec;
use beamcraft_engine::drawing::*;
use beamcraft_engine::query::feature::FeatureId;
use beamcraft_layer_engine::{Asset, Cancellation, Control, Identity};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::*;
use std::time::{Duration, Instant};

const DRAWING: &str = "01900000-0000-7000-8000-000000000001";
const VIEW: &str = "01900000-0000-7000-8000-000000000002";
const DIM: &str = "01900000-0000-7000-8000-000000000003";
const MEMBER: &str = "01900000-0000-7000-8000-000000000004";

fn drawing() -> Drawing {
    Drawing::admit(DrawingSpec {
        id: DrawingId::parse(DRAWING).unwrap(),
        name: "Балка \"A\" \\ original".into(),
        source: SourceModelPin {
            project: ProjectId::parse(VIEW).unwrap(),
            revision: RevisionId::parse(DIM).unwrap(),
            manifest: ContentDigest::from_bytes([1; 32]),
            schema: ContentDigest::from_bytes([2; 32]),
            generation: u64::MAX,
        },
        member: ElementId::parse(MEMBER).unwrap(),
        view: ViewSpec {
            id: DrawingViewId::parse(VIEW).unwrap(),
            plane: Plane::Top,
            scale_denominator: 125.0,
            paper_origin_mm: [-12.5, 17.25],
        },
        dimension: DimensionSpec {
            id: DimensionId::parse(DIM).unwrap(),
            anchors: ["v.x0.y0.z0", "v.x1.y1.z1"]
                .map(|role| FeatureId::parse(&format!("fid1:{MEMBER}:member:{role}")).unwrap()),
            paper_offset_mm: -7.5,
            decimal_places: 3,
        },
        drafting: Vec::new(),
    })
    .unwrap()
}

fn identity() -> Identity {
    Identity {
        project: VIEW.into(),
        workspace: DRAWING.into(),
        revision: ContentDigest::from_bytes([9; 32]),
        generation: 17,
    }
}
fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(30),
    }
}

fn changed(asset: &Asset, before: &str, after: &str) -> Asset {
    let text = String::from_utf8(asset.bytes.clone()).unwrap();
    assert!(text.contains(before), "{before}");
    let bytes = text.replacen(before, after, 1).into_bytes();
    Asset {
        name: asset.name.clone(),
        digest: Sha256::digest(&bytes).unwrap(),
        bytes,
    }
}

#[test]
fn ordinary_usd_round_trips_exact_ids_pins_units_settings_and_qualified_references() {
    let intent = drawing();
    let asset = codec::encode(&intent).unwrap();
    assert!(asset.bytes.len() < codec::MAX_INTENT_BYTES);
    assert_eq!(asset.digest, Sha256::digest(&asset.bytes).unwrap());
    assert_eq!(asset.bytes, codec::encode(&intent).unwrap().bytes);
    let decoded = codec::decode(asset.clone(), identity(), &control()).unwrap();
    assert_eq!(decoded, intent);
    assert_eq!(codec::encode(&decoded).unwrap().bytes, asset.bytes);
    assert_eq!(decoded.spec().source.generation, u64::MAX);
    assert_ne!(
        codec::schema_identity().unwrap(),
        decoded.spec().source.schema
    );
    assert_ne!(decoded.spec().source.manifest, identity().revision);
    for plane in [Plane::Front, Plane::Top, Plane::Right] {
        let mut spec = intent.spec().clone();
        spec.view.plane = plane;
        spec.view.paper_origin_mm = [-0.0, 0.0];
        spec.dimension.paper_offset_mm = 0.0;
        let admitted = Drawing::admit(spec).unwrap();
        assert_eq!(
            codec::decode(codec::encode(&admitted).unwrap(), identity(), &control()).unwrap(),
            admitted
        );
    }
}

#[test]
fn drawn_objects_round_trip_exactly_and_foreign_topology_refuses() {
    let entity = |suffix: u64, geometry| DraftingSpec {
        id: DraftingEntityId::parse(&format!("01900000-0000-7000-8000-{suffix:012}")).unwrap(),
        geometry,
    };
    let intent = drawing()
        .with_drafting(vec![
            entity(
                5,
                DraftingGeometry::Line {
                    a: [1.5, -2.0],
                    b: [30.25, 4.0],
                },
            ),
            entity(
                6,
                DraftingGeometry::Circle {
                    center: [10.0, 10.0],
                    radius: 2.5,
                },
            ),
            entity(
                7,
                DraftingGeometry::Arc {
                    center: [0.0, 0.0],
                    radius: 5.0,
                    start: 0.0,
                    end: 1.25,
                },
            ),
            entity(
                8,
                DraftingGeometry::Rectangle {
                    a: [0.0, 0.0],
                    b: [8.0, 3.0],
                },
            ),
            entity(
                9,
                DraftingGeometry::Polyline {
                    points: vec![[0.0, 0.0], [2.0, 1.0], [5.0, -1.0]],
                },
            ),
        ])
        .unwrap();
    let asset = codec::encode(&intent).unwrap();
    assert!(asset.bytes.len() < codec::MAX_INTENT_BYTES);
    assert_eq!(asset.bytes, codec::encode(&intent).unwrap().bytes);
    let decoded = codec::decode(asset.clone(), identity(), &control()).unwrap();
    assert_eq!(decoded, intent);

    // Ordinals are canonical: renaming a child prim leaves the closed schema.
    let renamed = changed(&asset, "\"E1\"", "\"E9\"");
    assert!(codec::decode(renamed, identity(), &control()).is_err());
    // A foreign attribute is ignored by the semantic reader but is not canonical bytes.
    let extra = changed(
        &asset,
        "custom token drawing:kind = \"line\"",
        "custom token drawing:kind = \"line\"\n            custom string drawing:extra = \"x\"",
    );
    assert!(codec::decode(extra, identity(), &control()).is_err());
    // A rewritten coordinate is a well-formed different document, not a decoding failure: its
    // detection is the manifest digest's job, verified at the document store layer.
}

#[test]
fn corruption_wrong_identity_schema_types_and_unknown_content_refuse() {
    let asset = codec::encode(&drawing()).unwrap();
    let mut wrong_hash = asset.clone();
    wrong_hash.digest = ContentDigest::from_bytes([0; 32]);
    assert_eq!(
        codec::decode(wrong_hash, identity(), &control())
            .unwrap_err()
            .code,
        ErrorCode::IntegrityError
    );
    for (before, after) in [
        ("beamcraft-drawing/1", "beamcraft-drawing/2"),
        (
            &codec::schema_identity().unwrap().to_string(),
            &ContentDigest::from_bytes([0; 32]).to_string(),
        ),
    ] {
        assert_eq!(
            codec::decode(changed(&asset, before, after), identity(), &control())
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedVersion
        );
    }
    for field in 0..2 {
        let mut wrong = identity();
        if field == 0 {
            wrong.project = DIM.into();
        } else {
            wrong.workspace = DIM.into();
        }
        assert_eq!(
            codec::decode(asset.clone(), wrong, &control())
                .unwrap_err()
                .code,
            ErrorCode::IntegrityError
        );
    }
    for (before, after) in [
        ("custom double drawing:scale", "custom string drawing:scale"),
        (
            "custom double drawing:scale = 125",
            "custom double drawing:scale = 0",
        ),
        ("member:v.x1.y1.z1", "member:f.x1"),
        ("member:v.x1.y1.z1", "member:0"),
        ("drawing:decimals = 3", "drawing:decimals = 4"),
        ("def Scope \"View\"", "def Scope \"Other\""),
        ("upAxis = \"Z\"", "upAxis = \"Y\""),
        ("metersPerUnit = 0.001", "metersPerUnit = 1"),
        (
            "custom string drawing:contract",
            "custom string unknown:contract",
        ),
        (
            "def Xform \"Drawing\"\n{",
            "def Xform \"Drawing\"\n{\n    custom string extra:field = \"inert\"",
        ),
        ("#usda 1.0", "#usda 1.0\n# noncanonical content"),
    ] {
        assert!(
            codec::decode(changed(&asset, before, after), identity(), &control()).is_err(),
            "{before} -> {after}"
        );
    }
}

#[test]
fn oversized_assets_arrays_and_cancelled_or_expired_requests_never_return_intent() {
    let asset = codec::encode(&drawing()).unwrap();
    let mut huge = asset.clone();
    huge.bytes.resize(codec::MAX_INTENT_BYTES + 1, b' ');
    assert_eq!(
        codec::decode(huge, identity(), &control())
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    let extra = changed(
        &asset,
        "drawing:anchors = [",
        "drawing:anchors = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\", \"g\", \"h\", \"i\", ",
    );
    assert!(codec::decode(extra, identity(), &control()).is_err());
    let cancelled = control();
    cancelled.cancellation.cancel();
    assert_eq!(
        codec::decode(asset.clone(), identity(), &cancelled)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    let expired = Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now(),
    };
    assert_eq!(
        codec::decode(asset, identity(), &expired).unwrap_err().code,
        ErrorCode::Cancelled
    );
}
