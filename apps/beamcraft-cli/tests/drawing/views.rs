// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D04 slice 1: placed drawing views. Nothing is placed automatically, every placed view
//! resolves exactly its own basis of the document's pinned source, a model change marks the
//! views stale together, and unrelated edits retain the placement.

use super::*;
use beamcraft_cli::drawing::archive::SourceArchive;
use beamcraft_engine::drawing::projection::{self, BodyProjection, ProjectedEdge};
use beamcraft_engine::drawing::{
    DEFAULT_SCALE_DENOMINATOR, DimensionSpec, Drawing, DrawingSpec, Plane, SourceModelPin, ViewSpec,
};
use beamcraft_engine::query::feature::{FeatureGeometry, FeatureId, lookup_feature};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{DimensionId, DrawingId, DrawingViewId, ElementId};

const DRAWING: &str = "01900000-0000-7000-8000-000000000301";
const VIEW: &str = "01900000-0000-7000-8000-000000000302";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000303";
const ACTOR: &str = "01900000-0000-7000-8000-000000000304";
const OP_CREATE: &str = "01900000-0000-7000-8000-000000000305";
const REV_CREATE: &str = "01900000-0000-7000-8000-000000000306";
const OP_VIEWS: &str = "01900000-0000-7000-8000-000000000307";
const REV_VIEWS: &str = "01900000-0000-7000-8000-000000000308";
const OP_DRAFT: &str = "01900000-0000-7000-8000-000000000309";
const REV_DRAFT: &str = "01900000-0000-7000-8000-000000000310";
const OP_REPLACE: &str = "01900000-0000-7000-8000-000000000311";
const REV_REPLACE: &str = "01900000-0000-7000-8000-000000000312";
const TOP: &str = "01900000-0000-7000-8000-000000000313";
const RIGHT: &str = "01900000-0000-7000-8000-000000000314";
const EXTRA: &str = "01900000-0000-7000-8000-000000000315";
const LINE: &str = "01900000-0000-7000-8000-000000000316";

fn create_args(document_dir: &str) -> Vec<String> {
    [
        "--document-dir",
        document_dir,
        "--drawing-id",
        DRAWING,
        "--view-id",
        VIEW,
        "--dimension-id",
        DIMENSION,
        "--id",
        MEMBER,
        "--anchor-a",
        &format!("fid1:{MEMBER}:member:v.x0.y0.z0"),
        "--anchor-b",
        &format!("fid1:{MEMBER}:member:v.x0.y0.z1"),
        "--op-id",
        OP_CREATE,
        "--revision-id",
        REV_CREATE,
        "--actor-id",
        ACTOR,
    ]
    .iter()
    .map(|value| (*value).to_owned())
    .collect()
}

fn argv(options: &[String]) -> Vec<&str> {
    options.iter().map(String::as_str).collect()
}

/// Runs `edit-views` with no `--store` argument: a placement is reachable from the document
/// alone, so the harness that injects the model store is deliberately bypassed.
fn edit_views(
    document_dir: &str,
    head: &str,
    op: &str,
    revision: &str,
    views: &[String],
) -> (i32, Json) {
    let mut args = vec![
        "edit-views".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--expected-head".to_owned(),
        head.to_owned(),
        "--op-id".to_owned(),
        op.to_owned(),
        "--revision-id".to_owned(),
        revision.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    args.extend(
        views
            .iter()
            .flat_map(|value| ["--placed-view".to_owned(), value.clone()]),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(&args)
        .output()
        .unwrap();
    let body = beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    (output.status.code().unwrap(), body)
}

fn placed(json: &Json) -> Vec<(String, String)> {
    list(json, "placed_views")
        .iter()
        .map(|view| (text(view, "id"), text(view, "plane")))
        .collect()
}

#[test]
fn views_are_placed_only_by_explicit_commands_and_survive_other_edits() {
    let f = Fixture::new();
    f.model();
    let document_dir = f
        .shared
        .root
        .join("drawing-doc")
        .to_str()
        .unwrap()
        .to_owned();
    // A freshly created document places nothing; every view needs its own command.
    let created = f.ok("create-drawing", &argv(&create_args(&document_dir)));
    assert!(placed(&created).is_empty());
    let head = text(&created, "revision_id");

    let top = format!("{TOP}:top:100:0,0");
    let right = format!("{RIGHT}:right:50:100,50");
    let (code, placed_both) = edit_views(
        &document_dir,
        &head,
        OP_VIEWS,
        REV_VIEWS,
        &[top.clone(), right.clone()],
    );
    assert_eq!(code, 0, "{placed_both:?}");
    assert_eq!(
        placed(&placed_both),
        vec![
            (TOP.to_owned(), "top".to_owned()),
            (RIGHT.to_owned(), "right".to_owned())
        ]
    );
    let views_head = text(&placed_both, "revision_id");

    // The exact replay returns the same revision; a different placement under the same
    // operation id is a different request and refuses.
    let (code, replayed) = edit_views(&document_dir, &head, OP_VIEWS, REV_VIEWS, &[top, right]);
    assert_eq!(code, 0, "{replayed:?}");
    assert_eq!(text(&replayed, "revision_id"), views_head);
    let (code, refusal) = edit_views(
        &document_dir,
        &head,
        OP_VIEWS,
        REV_VIEWS,
        &[format!("{TOP}:top:75:0,0")],
    );
    assert_eq!(code, 2);
    assert_eq!(text(&refusal, "code"), "OP_ID_REUSE");

    // A drawn-object edit is a different collection: the placement carries over unchanged.
    let drafting = f.ok(
        "edit-drafting",
        &[
            "--document-dir",
            &document_dir,
            "--expected-head",
            &views_head,
            "--op-id",
            OP_DRAFT,
            "--revision-id",
            REV_DRAFT,
            "--actor-id",
            ACTOR,
            "--drafting",
            &format!("{LINE}:line:0,0,100,0"),
        ],
    );
    assert_eq!(list(&drafting, "drafting").len(), 1);
    assert_eq!(placed(&drafting).len(), 2);

    // Placement replacement is one whole-document operation.
    let (code, replaced) = edit_views(
        &document_dir,
        &text(&drafting, "revision_id"),
        OP_REPLACE,
        REV_REPLACE,
        &[format!("{TOP}:top:75:5,5")],
    );
    assert_eq!(code, 0, "{replaced:?}");
    assert_eq!(placed(&replaced), vec![(TOP.to_owned(), "top".to_owned())]);

    // Identity collisions refuse without committing.
    let head_now = text(&replaced, "revision_id");
    let (code, refusal) = edit_views(
        &document_dir,
        &head_now,
        OP_DRAFT,
        REV_DRAFT,
        &[format!("{TOP}:front:10:0,0"), format!("{TOP}:top:10:0,0")],
    );
    assert_eq!(code, 2);
    assert_eq!(text(&refusal, "code"), "DUPLICATE_ID");

    // The document inspect carries the placement with its paper settings.
    let inspected = f.ok(
        "inspect-drawing",
        &[
            "--document-dir",
            &document_dir,
            "--model-store",
            f.shared.root.join("store").to_str().unwrap(),
        ],
    );
    let only = &list(&inspected, "placed_views")[0];
    assert_eq!(text(only, "id"), TOP);
    assert_eq!(text(only, "plane"), "top");
    assert_eq!(list(only, "paper_origin_mm").len(), 2);
}

#[test]
fn a_placed_view_shows_exactly_its_own_basis_of_the_pinned_source() {
    let f = Fixture::new();
    f.model();
    let document_dir = f
        .shared
        .root
        .join("drawing-doc")
        .to_str()
        .unwrap()
        .to_owned();
    let created = f.ok("create-drawing", &argv(&create_args(&document_dir)));
    let head = text(&created, "revision_id");
    let (code, placed_both) = edit_views(
        &document_dir,
        &head,
        OP_VIEWS,
        REV_VIEWS,
        &[
            format!("{TOP}:top:100:0,0"),
            format!("{RIGHT}:right:50:100,50"),
        ],
    );
    assert_eq!(code, 0, "{placed_both:?}");

    // The same archived source the document was committed against.
    let source = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    let snapshot = source.inspect().unwrap().snapshot;
    let pin = source.pin();
    let spec_of = |views: Vec<ViewSpec>| DrawingSpec {
        id: DrawingId::parse(DRAWING).unwrap(),
        name: "Member drawing".into(),
        source: SourceModelPin {
            project: pin.project,
            revision: pin.revision,
            manifest: pin.manifest,
            schema: pin.schema,
            generation: pin.generation,
        },
        member: ElementId::parse(MEMBER).unwrap(),
        view: ViewSpec {
            id: DrawingViewId::parse(VIEW).unwrap(),
            plane: Plane::Front,
            scale_denominator: DEFAULT_SCALE_DENOMINATOR,
            paper_origin_mm: [0.0, 0.0],
        },
        dimension: DimensionSpec {
            id: DimensionId::parse(DIMENSION).unwrap(),
            anchors: ["v.x0.y0.z0", "v.x0.y0.z1"]
                .map(|role| FeatureId::parse(&format!("fid1:{MEMBER}:member:{role}")).unwrap()),
            paper_offset_mm: 10.0,
            decimal_places: 0,
        },
        drafting: Vec::new(),
        views,
    };
    let drawing = Drawing::admit(spec_of(Vec::new()))
        .unwrap()
        .with_views(vec![
            ViewSpec {
                id: DrawingViewId::parse(TOP).unwrap(),
                plane: Plane::Top,
                scale_denominator: 100.0,
                paper_origin_mm: [0.0, 0.0],
            },
            ViewSpec {
                id: DrawingViewId::parse(RIGHT).unwrap(),
                plane: Plane::Right,
                scale_denominator: 50.0,
                paper_origin_mm: [100.0, 50.0],
            },
        ])
        .unwrap();
    let resolve = |view: &ViewSpec| match projection::resolve_view(&drawing, view, &snapshot) {
        Ok(BodyProjection::Resolved(edges)) => edges,
        other => panic!("expected resolved wireframe: {other:?}"),
    };
    let top = resolve(&drawing.views()[0]);
    let right = resolve(&drawing.views()[1]);
    let paper = |edges: &[ProjectedEdge]| {
        edges
            .iter()
            .map(|edge| (edge.feature.to_string(), edge.paper_mm))
            .collect::<std::collections::BTreeMap<String, [[f64; 2]; 2]>>()
    };
    let top_map = paper(&top);
    let right_map = paper(&right);
    // The canonical box wireframe has twelve named edges, and the same semantic addresses
    // appear in every basis.
    assert_eq!(top.len(), 12);
    let top_keys: std::collections::BTreeSet<&str> = top_map.keys().map(String::as_str).collect();
    let right_keys: std::collections::BTreeSet<&str> =
        right_map.keys().map(String::as_str).collect();
    assert_eq!(top_keys, right_keys);
    // Each view's paper millimetres are exactly that view's projection of the same world
    // coordinates: top at 1:100 with origin 0 gives [x, y]; right at 1:50 with origin
    // [100, 50] gives [y/50+100, z/50+50]. The profile is centred on the member axis.
    for (feature, top_ends) in &top_map {
        let FeatureGeometry::Edge { ends: [a, b], .. } =
            lookup_feature(&snapshot, feature).unwrap().world
        else {
            panic!("{feature} is not an edge in the archived source");
        };
        let right_ends = right_map[feature.as_str()];
        for (index, world) in [a, b].iter().enumerate() {
            let [x, y, z] = *world;
            assert_eq!(top_ends[index], [x / 100.0, y / 100.0]);
            assert_eq!(right_ends[index], [y / 50.0 + 100.0, z / 50.0 + 50.0]);
        }
    }

    // A model change marks every placed view stale: the new head refuses before geometry.
    f.ok(
        "create-plate",
        &[
            "--width",
            "10",
            "--length",
            "10",
            "--thickness",
            "1",
            "--translation",
            "9000,0,0",
            "--new-id",
            EXTRA,
        ],
    );
    let moved = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    assert_ne!(moved.pin(), pin);
    let moved_snapshot = moved.inspect().unwrap().snapshot;
    for view in drawing.views() {
        assert_eq!(
            projection::resolve_view(&drawing, view, &moved_snapshot)
                .unwrap_err()
                .code,
            ErrorCode::StaleWorkspace
        );
    }
    // An explicitly refreshed document resolves again against the new pin.
    let refreshed = drawing.refreshed(moved.pin());
    for view in refreshed.views() {
        match projection::resolve_view(&refreshed, view, &moved_snapshot).unwrap() {
            BodyProjection::Resolved(edges) => assert_eq!(edges.len(), 12),
            other => panic!("expected resolved wireframe after refresh: {other:?}"),
        }
    }
}
