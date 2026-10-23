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
