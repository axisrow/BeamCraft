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
use beamcraft_model::sheet::Sheet;
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
        views: Vec::new(),
        sections: Vec::new(),
        details: Vec::new(),
        sheet: None,
        layers: Vec::new(),
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
    const LAYER: &str = "01900000-0000-7000-8000-000000000014";
    let entity = |suffix: u64, geometry| DraftingSpec {
        id: DraftingEntityId::parse(&format!("01900000-0000-7000-8000-{suffix:012}")).unwrap(),
        geometry,
        layer: DraftingEntityId::parse(LAYER).unwrap(),
        line_type: LineType::Dashed,
    };
    let layer = LayerSpec {
        id: DraftingEntityId::parse(LAYER).unwrap(),
        name: "Выноски \\ \"A\"".into(),
        visible: true,
    };
    let intent = drawing()
        .with_drafting(
            vec![layer],
            vec![
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
                entity(
                    10,
                    DraftingGeometry::Text {
                        anchor: [12.0, 3.0],
                        text: "Note \u{00ab}B\u{00bb}".to_owned(),
                        height: 3.5,
                        rotation: -0.25,
                    },
                ),
                entity(
                    11,
                    DraftingGeometry::Hatch {
                        outline: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 3.0]],
                        pattern: HatchPattern::Crosshatch,
                    },
                ),
                entity(
                    12,
                    DraftingGeometry::Symbol {
                        anchor: [40.0, 40.0],
                        symbol: PaperSymbol::Section,
                        scale: 2.0,
                    },
                ),
                entity(
                    13,
                    DraftingGeometry::FreeDimension {
                        from: [0.0, 0.0],
                        to: [30.0, 0.0],
                        offset: 6.5,
                        decimal_places: 1,
                    },
                ),
            ],
        )
        .unwrap();
    let asset = codec::encode(&intent).unwrap();
    assert!(asset.bytes.len() < codec::MAX_INTENT_BYTES);
    assert_eq!(asset.bytes, codec::encode(&intent).unwrap().bytes);
    let decoded = codec::decode(asset.clone(), identity(), &control()).unwrap();
    assert_eq!(decoded, intent);
    assert_eq!(decoded.layers().len(), 1);
    assert_eq!(decoded.layers()[0].name, "Выноски \\ \"A\"");
    assert_eq!(decoded.drafting()[0].line_type, LineType::Dashed);

    // Ordinals are canonical: renaming a child prim leaves the closed schema.
    let renamed = changed(&asset, "\"E1\"", "\"E9\"");
    assert!(codec::decode(renamed, identity(), &control()).is_err());
    // The same holds for the layer scope's children.
    let renamed_layer = changed(&asset, "\"L1\"", "\"L4\"");
    assert!(codec::decode(renamed_layer, identity(), &control()).is_err());
    // A foreign prim inside the layer scope is not canonical bytes.
    let foreign = changed(&asset, "def Scope \"L1\"", "def Scope \"L1x\"");
    assert!(codec::decode(foreign, identity(), &control()).is_err());
    // A drawn object naming a layer the document does not define refuses at admission.
    let stranger = changed(
        &asset,
        "custom string drawing:layer = \"01900000-0000-7000-8000-000000000014\"",
        "custom string drawing:layer = \"01900000-0000-7000-8000-0000000000ff\"",
    );
    assert!(codec::decode(stranger, identity(), &control()).is_err());
    // A foreign attribute is ignored by the semantic reader but is not canonical bytes.
    let extra = changed(
        &asset,
        "custom token drawing:kind = \"line\"",
        "custom token drawing:kind = \"line\"\n            custom string drawing:extra = \"x\"",
    );
    assert!(codec::decode(extra, identity(), &control()).is_err());
    // A text entity without its required payload disagrees with its kind.
    let stripped = changed(
        &asset,
        "            custom string drawing:text = \"Note «B»\"\n",
        "",
    );
    assert!(codec::decode(stripped, identity(), &control()).is_err());
    // A rewritten coordinate is a well-formed different document, not a decoding failure: its
    // detection is the manifest digest's job, verified at the document store layer.
}

#[test]
fn the_densest_admitted_page_encodes_within_the_intent_byte_cap() {
    const LAYER: &str = "01900000-0000-7000-8000-000000000015";
    // The longest bounded canonical coordinates: twenty-four bytes each, the admission edge,
    // as literals so every shortest round-trip form is exactly the decimal written here.
    const EDGE: [f64; 16] = [
        1e-22, 2e-22, 3e-22, 4e-22, 5e-22, 6e-22, 7e-22, 8e-22, 9e-22, 1.0e-21, 1.1e-21, 1.2e-21,
        1.3e-21, 1.4e-21, 1.5e-21, 1.6e-21,
    ];
    let layer = LayerSpec {
        id: DraftingEntityId::parse(LAYER).unwrap(),
        name: "Dense".into(),
        visible: true,
    };
    let entities = (0..58u64)
        .map(|index| DraftingSpec {
            id: DraftingEntityId::parse(&format!("019a0000-0000-7000-8000-{index:012}")).unwrap(),
            geometry: DraftingGeometry::Hatch {
                outline: EDGE.iter().map(|value| [*value, *value]).collect(),
                pattern: HatchPattern::Crosshatch,
            },
            layer: DraftingEntityId::parse(LAYER).unwrap(),
            line_type: LineType::Dotted,
        })
        .collect();
    // One layer and fifty-eight sixteen-point hatches is exactly the one-page ceiling.
    let intent = drawing().with_drafting(vec![layer], entities).unwrap();
    let asset = codec::encode(&intent).unwrap();
    assert!(
        asset.bytes.len() <= codec::MAX_INTENT_BYTES,
        "{}",
        asset.bytes.len()
    );
    assert_eq!(
        codec::decode(asset, identity(), &control()).unwrap(),
        intent
    );
}

#[test]
fn sections_and_details_round_trip_exactly_and_foreign_tokens_refuse() {
    let section = |suffix: u64, cut: f64, side: CutSide| SectionSpec {
        id: DrawingViewId::parse(&format!("01900000-0000-7000-8000-{suffix:012}")).unwrap(),
        plane: Plane::Right,
        cut_mm: cut,
        side,
        scale_denominator: 50.0,
        paper_origin_mm: [10.0, -10.0],
    };
    let detail = |suffix: u64, parent: &DrawingViewId| DetailSpec {
        id: DrawingViewId::parse(&format!("01900000-0000-7000-8000-{suffix:012}")).unwrap(),
        parent: *parent,
        center_paper_mm: [15.0, 0.0],
        radius_mm: 5.5,
        scale_denominator: 20.0,
        paper_origin_mm: [100.0, 100.0],
    };
    let intent = drawing()
        .with_sections(vec![
            section(6, 1500.0, CutSide::Positive),
            section(7, 2500.0, CutSide::Negative),
        ])
        .unwrap()
        .with_details(vec![detail(8, &DrawingViewId::parse(VIEW).unwrap())])
        .unwrap();
    let asset = codec::encode(&intent).unwrap();
    assert!(asset.bytes.len() < codec::MAX_INTENT_BYTES);
    assert_eq!(asset.bytes, codec::encode(&intent).unwrap().bytes);
    let text = String::from_utf8(asset.bytes.clone()).unwrap();
    assert!(text.contains("def Scope \"Sections\"") && text.contains("\"S1\""));
    assert!(text.contains("drawing:side = \"negative\""));
    assert!(text.contains("def Scope \"Details\"") && text.contains("drawing:parent"));
    let decoded = codec::decode(asset.clone(), identity(), &control()).unwrap();
    assert_eq!(decoded, intent);
    // Empty placements encode without their optional scopes.
    let emptied = intent
        .clone()
        .with_sections(Vec::new())
        .unwrap()
        .with_details(Vec::new())
        .unwrap();
    let bare = String::from_utf8(codec::encode(&emptied).unwrap().bytes).unwrap();
    assert!(!bare.contains("\"Sections\"") && !bare.contains("\"Details\""));
    // Ordinals are canonical and the side token is closed.
    let renamed = changed(&asset, "\"S1\"", "\"S9\"");
    assert!(codec::decode(renamed, identity(), &control()).is_err());
    let sideways = changed(
        &asset,
        "drawing:side = \"positive\"",
        "drawing:side = \"sideways\"",
    );
    assert!(codec::decode(sideways, identity(), &control()).is_err());
}

#[test]
fn placed_views_round_trip_exactly_and_keep_their_own_paper_settings() {
    let view = |suffix: u64, plane: Plane, scale: f64, origin: [f64; 2]| ViewSpec {
        id: DrawingViewId::parse(&format!("01900000-0000-7000-8000-{suffix:012}")).unwrap(),
        plane,
        scale_denominator: scale,
        paper_origin_mm: origin,
    };
    let intent = drawing()
        .with_views(vec![
            view(6, Plane::Front, 50.0, [10.0, -20.0]),
            view(
                7,
                Plane::Right,
                MAX_SCALE_DENOMINATOR,
                [-MAX_PAPER_MM, MAX_PAPER_MM],
            ),
        ])
        .unwrap();
    let asset = codec::encode(&intent).unwrap();
    assert!(asset.bytes.len() < codec::MAX_INTENT_BYTES);
    assert_eq!(asset.bytes, codec::encode(&intent).unwrap().bytes);
    let decoded = codec::decode(asset.clone(), identity(), &control()).unwrap();
    assert_eq!(decoded, intent);
    // An empty placement encodes without the optional scope and still round-trips.
    let emptied = intent.clone().with_views(Vec::new()).unwrap();
    assert!(
        !String::from_utf8(codec::encode(&emptied).unwrap().bytes)
            .unwrap()
            .contains("\"Views\"")
    );
    // Ordinals are canonical: renaming a child prim leaves the closed schema.
    let renamed = changed(&asset, "\"V1\"", "\"V9\"");
    assert!(codec::decode(renamed, identity(), &control()).is_err());
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

#[test]
fn the_sheet_round_trips_exactly_and_non_canonical_sheets_refuse() {
    let mut spec = drawing().spec().clone();
    spec.sheet = Some(SheetSpec {
        size: Sheet::A3,
        frame: SheetFrame { inset_mm: 12.5 },
        title: TitleBlock {
            project: "Bridge".into(),
            author: "Drafter".into(),
        },
    });
    let intent = Drawing::admit(spec).unwrap();
    let asset = codec::encode(&intent).unwrap();
    let text = String::from_utf8(asset.bytes.clone()).unwrap();
    assert!(text.contains("def Scope \"Sheet\""));
    assert!(text.contains("drawing:size = \"a3\""));
    assert_eq!(
        codec::decode(asset.clone(), identity(), &control()).unwrap(),
        intent
    );
    // Anything outside the canonical sheet schema refuses: a renamed scope prim, and an
    // unsupported size token. (A re-formatted number that stays canonical is a different,
    // valid document, not a tamper.)
    for (before, after) in [
        ("def Scope \"Sheet\"", "def Scope \"SheetX\""),
        ("drawing:size = \"a3\"", "drawing:size = \"b3\""),
    ] {
        assert!(codec::decode(changed(&asset, before, after), identity(), &control()).is_err());
    }
}
