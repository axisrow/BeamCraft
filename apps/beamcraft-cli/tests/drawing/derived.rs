// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D02 ordinary USD curves from real CLI-authored models and strict archived source snapshots.

use super::*;
use beamcraft_cli::drawing::{archive::SourceArchive, codec, derived};
use beamcraft_engine::drawing::*;
use beamcraft_engine::query::feature::FeatureId;
use beamcraft_layer_engine::{
    Asset, BACKEND, Budgets, Cancellation, Control, Identity, OpenRequest, Profile, Reader, Value,
};
use beamcraft_model::digest::Sha256;
use beamcraft_model::id::{DimensionId, DrawingId, DrawingViewId, ElementId};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

fn intent(pin: SourceModelPin) -> Drawing {
    Drawing::admit(DrawingSpec {
        id: DrawingId::parse(PLATE).unwrap(),
        name: "Original drawing".into(),
        source: pin,
        member: ElementId::parse(MEMBER).unwrap(),
        view: ViewSpec {
            id: DrawingViewId::parse(COPY).unwrap(),
            plane: Plane::Front,
            scale_denominator: 100.0,
            paper_origin_mm: [0.0, 0.0],
        },
        dimension: DimensionSpec {
            id: DimensionId::parse(HOLED).unwrap(),
            anchors: ["v.x0.y0.z0", "v.x0.y0.z1"]
                .map(|role| FeatureId::parse(&format!("fid1:{MEMBER}:member:{role}")).unwrap()),
            paper_offset_mm: 10.0,
            decimal_places: 1,
        },
        drafting: Vec::new(),
        layers: Vec::new(),
        views: Vec::new(),
        sections: Vec::new(),
        details: Vec::new(),
        sheet: None,
    })
    .unwrap()
}

#[test]
fn standard_curves_and_exact_double_values_compose_in_a_fresh_usd_reader() {
    let f = Fixture::new();
    f.model();
    let head = f.head();
    let source = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    let drawing = intent(source.pin());
    let snapshot = source.inspect().unwrap().snapshot;
    let geometry = derived::encode(&drawing, &snapshot).unwrap();
    derived::verify(&geometry, &drawing, &snapshot).unwrap();
    assert_eq!(
        geometry.bytes,
        derived::encode(&drawing, &snapshot).unwrap().bytes
    );
    let assets = vec![
        derived::root().unwrap(),
        codec::encode(&drawing).unwrap(),
        geometry.clone(),
    ];
    let mut request = OpenRequest {
        identity: Identity {
            project: source.pin().project.to_string(),
            workspace: drawing.spec().id.to_string(),
            revision: geometry.digest,
            generation: 1,
        },
        root: derived::ROOT_LAYER.into(),
        assets: assets.clone(),
        owned_layers: BTreeSet::from([
            derived::ROOT_LAYER.into(),
            codec::INTENT_LAYER.into(),
            derived::DERIVED_LAYER.into(),
        ]),
        manifest: geometry.digest,
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets::FIRST_SLICE,
    };
    request.manifest = request.manifest_digest().unwrap();
    let control = Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(30),
    };
    let mut reader = Reader::default();
    let opened = reader.open(request, &control).unwrap();
    assert_eq!(opened.prim_count, 5);
    assert_eq!(opened.meters_per_unit, 0.001);
    assert_eq!(opened.up_axis, "Z");
    let page = reader.read(&opened.context, None, &control).unwrap();
    assert!(page.next.is_none());
    let get = |path: &str, field: &str| {
        page.items
            .iter()
            .find(|p| p.path == path)
            .unwrap()
            .properties
            .iter()
            .find(|p| p.name == field)
            .unwrap()
            .value
            .as_ref()
            .unwrap()
    };
    assert_eq!(
        get("/Drawing/View/Geometry", "type"),
        &Value::Text("linear".into())
    );
    assert_eq!(
        get("/Drawing/View/Geometry", "curveVertexCounts"),
        &Value::Array(vec![Value::Int(2); 12])
    );
    assert_eq!(
        get("/Drawing/Dimension", "drawing:projectedLengthMm"),
        &Value::Number(3000.0)
    );
    assert_eq!(
        get("/Drawing/Dimension", "drawing:labelMm"),
        &Value::Text("3000.0".into())
    );
    let Value::Array(points) = get("/Drawing/View/Geometry", "drawing:exactPointsMm") else {
        panic!("missing exact points")
    };
    assert_eq!(points.len(), 24);
    assert_eq!(
        points[0],
        Value::Array(vec![
            Value::Number(0.0),
            Value::Number(-0.25),
            Value::Number(0.0)
        ])
    );
    assert_eq!(
        points[1],
        Value::Array(vec![
            Value::Number(30.0),
            Value::Number(-0.25),
            Value::Number(0.0)
        ])
    );
    let Value::Array(strokes) = get("/Drawing/Dimension/Geometry", "drawing:exactPointsMm") else {
        panic!("missing dimension strokes")
    };
    assert_eq!(strokes.len(), 6);
    assert_eq!(
        strokes[1],
        Value::Array(vec![
            Value::Number(0.0),
            Value::Number(9.75),
            Value::Number(0.0)
        ])
    );
    assert_eq!(f.head(), head);
    // Optional original test output for an external SDK/oracle; never required by normal CI.
    if let Some(output) = std::env::var_os("BEAMCRAFT_DRAWING_DERIVED_OUTPUT") {
        let output = PathBuf::from(output);
        fs::create_dir_all(&output).unwrap();
        for asset in assets {
            fs::write(output.join(asset.name), asset.bytes).unwrap();
        }
    }
}

#[test]
fn rehashed_fabricated_or_stale_geometry_refuses_and_missing_features_store_no_value() {
    let f = Fixture::new();
    f.model();
    let source = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    let drawing = intent(source.pin());
    let geometry = derived::encode(&drawing, &source.inspect().unwrap().snapshot).unwrap();
    let text = String::from_utf8(geometry.bytes.clone()).unwrap();
    let bytes = text.replace("3000", "4000").into_bytes();
    let fabricated = Asset {
        name: geometry.name.clone(),
        digest: Sha256::digest(&bytes).unwrap(),
        bytes,
    };
    assert!(derived::verify(&fabricated, &drawing, &source.inspect().unwrap().snapshot).is_err());
    f.ok(
        "update-element",
        &["--id", MEMBER, "--translation", "100,2000,0"],
    );
    let moved = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    let snapshot = moved.inspect().unwrap().snapshot;
    assert!(derived::verify(&geometry, &drawing, &snapshot).is_err());
    assert!(derived::verify(&geometry, &drawing.refreshed(moved.pin()), &snapshot).is_err());
    f.ok("delete-element", &["--id", MEMBER]);
    let deleted = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    let refreshed = drawing.refreshed(deleted.pin());
    let unresolved = derived::encode(&refreshed, &deleted.inspect().unwrap().snapshot).unwrap();
    let text = String::from_utf8(unresolved.bytes.clone()).unwrap();
    assert!(text.contains("missing_owner"));
    assert!(!text.contains("projectedLengthMm"));
    assert!(!text.contains("labelMm"));
    assert!(!text.contains("BasisCurves"));
    derived::verify(
        &unresolved,
        &refreshed,
        &deleted.inspect().unwrap().snapshot,
    )
    .unwrap();
}
