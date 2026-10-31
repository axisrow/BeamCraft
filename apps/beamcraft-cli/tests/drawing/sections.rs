// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D04 slice 2: section and detail views. A section is placed by hand through `edit-views`,
//! shows exactly the visible half of the member wireframe plus the exact cut contour, and a
//! detail magnifies exactly one parent view's circular region; a model change marks every
//! derived view stale together, and nothing is placed automatically.

use super::*;
use beamcraft_cli::drawing::archive::SourceArchive;
use beamcraft_engine::drawing::projection::{
    self, DetailProjection, ProjectedEdge, SectionGeometry, SectionProjection,
};
use beamcraft_engine::drawing::{
    CutSide, DEFAULT_SCALE_DENOMINATOR, DetailSpec, DimensionSpec, Drawing, DrawingSpec, Plane,
    SectionSpec, SourceModelPin, ViewSpec,
};
use beamcraft_engine::query::feature::FeatureId;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{DimensionId, DrawingId, DrawingViewId, ElementId};

const DRAWING: &str = "01900000-0000-7000-8000-000000000401";
const VIEW: &str = "01900000-0000-7000-8000-000000000402";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000403";
const ACTOR: &str = "01900000-0000-7000-8000-000000000404";
const OP_CREATE: &str = "01900000-0000-7000-8000-000000000405";
const REV_CREATE: &str = "01900000-0000-7000-8000-000000000406";
const OP_EDIT: &str = "01900000-0000-7000-8000-000000000407";
const REV_EDIT: &str = "01900000-0000-7000-8000-000000000408";
const OP_CLEAR: &str = "01900000-0000-7000-8000-000000000409";
const REV_CLEAR: &str = "01900000-0000-7000-8000-000000000410";
const OP_DRAFT: &str = "01900000-0000-7000-8000-000000000414";
const REV_DRAFT: &str = "01900000-0000-7000-8000-000000000415";
const SECTION: &str = "01900000-0000-7000-8000-000000000411";
const DETAIL: &str = "01900000-0000-7000-8000-000000000412";
const FOREIGN: &str = "01900000-0000-7000-8000-000000000413";
const LINE: &str = "01900000-0000-7000-8000-000000000416";
const LINE_LAYER: &str = "01900000-0000-7000-8000-00000000041b";
const PLATE: &str = "01900000-0000-7000-8000-000000000417";
const SECTION_FAR: &str = "01900000-0000-7000-8000-000000000418";
const SECTION_ALL: &str = "01900000-0000-7000-8000-000000000419";
const DETAIL_CLIP: &str = "01900000-0000-7000-8000-00000000041a";

/// The document's member is the fixture beam: world x 0..3000, y 1950..2050,
/// z -25..25 (a 100 x 50 profile centred on the axis y = 2000, z = 0).
const CORNERS_YZ: [[f64; 2]; 4] = [
    [1950.0, -25.0],
    [2050.0, -25.0],
    [1950.0, 25.0],
    [2050.0, 25.0],
];

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

/// Runs `edit-views` with raw option pairs, so one harness drives views, sections and
/// details alike.
fn edit_views(
    document_dir: &str,
    head: &str,
    op: &str,
    revision: &str,
    pairs: &[String],
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
    args.extend(pairs.to_owned());
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(&args)
        .output()
        .unwrap();
    let body = beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    (output.status.code().unwrap(), body)
}

fn pairs(values: &[(&str, &str)]) -> Vec<String> {
    values
        .iter()
        .flat_map(|(key, value)| [(*key).to_owned(), (*value).to_owned()])
        .collect()
}

/// Rebuilds the placed document in-process from the archived pin, as the CLI stored it.
fn document(
    pin: beamcraft_engine::drawing::SourceModelPin,
    sections: Vec<SectionSpec>,
    details: Vec<DetailSpec>,
) -> Drawing {
    Drawing::admit(DrawingSpec {
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
        views: Vec::new(),
        sections,
        details,
        sheet: None,
        layers: Vec::new(),
    })
    .unwrap()
}

/// The section placed by `edit_views` below: right basis, cut through the member's middle
/// at x = 1500, positive side visible, 1:1 paper at the origin.
fn mid_section(id: &str, side: CutSide, cut: f64) -> SectionSpec {
    SectionSpec {
        id: DrawingViewId::parse(id).unwrap(),
        plane: Plane::Right,
        cut_mm: cut,
        side,
        scale_denominator: 1.0,
        paper_origin_mm: [0.0, 0.0],
    }
}

fn detail(id: &str, radius: f64) -> DetailSpec {
    DetailSpec {
        id: DrawingViewId::parse(id).unwrap(),
        parent: DrawingViewId::parse(VIEW).unwrap(),
        center_paper_mm: [15.0, 0.0],
        radius_mm: radius,
        scale_denominator: 20.0,
        paper_origin_mm: [100.0, 100.0],
    }
}

fn resolved_section(projection: SectionProjection) -> SectionGeometry {
    match projection {
        SectionProjection::Resolved(geometry) => Some(geometry),
        SectionProjection::Unresolved(reason) => {
            eprintln!("section unexpectedly unresolved: {reason:?}");
            None
        }
    }
    .expect("a resolved section")
}

fn resolved_detail(
    projection: DetailProjection,
) -> beamcraft_engine::drawing::projection::DetailGeometry {
    match projection {
        DetailProjection::Resolved(geometry) => Some(geometry),
        DetailProjection::Unresolved(reason) => {
            eprintln!("detail unexpectedly unresolved: {reason:?}");
            None
        }
    }
    .expect("a resolved detail")
}

/// Endpoint pairs of resolved edges, order-free.
fn endpoints(edges: &[ProjectedEdge]) -> Vec<[[f64; 2]; 2]> {
    let mut all: Vec<[[f64; 2]; 2]> = edges.iter().map(|edge| edge.paper_mm).collect();
    all.sort_by(|p, q| {
        p[0][0]
            .total_cmp(&q[0][0])
            .then(p[0][1].total_cmp(&q[0][1]))
            .then(p[1][0].total_cmp(&q[1][0]))
            .then(p[1][1].total_cmp(&q[1][1]))
    });
    all
}

#[test]
fn sections_and_details_are_placed_only_by_edit_views_and_reported_by_inspect() {
    let f = Fixture::new();
    f.model();
    let document_dir = f
        .shared
        .root
        .join("drawing-doc")
        .to_str()
        .unwrap()
        .to_owned();
    // A created document places nothing, even when the command carries the placement options:
    // create/refresh accept them but ignore them, exactly as for --placed-view.
    let created = f.ok(
        "create-drawing",
        &argv(
            &[
                create_args(&document_dir),
                pairs(&[(
                    "--placed-section",
                    &format!("{SECTION}:right:1500:positive:1:0,0"),
                )]),
            ]
            .concat(),
        ),
    );
    assert!(list(&created, "placed_sections").is_empty());
    assert!(list(&created, "placed_details").is_empty());
    let head = text(&created, "revision_id");

    let (code, placed) = edit_views(
        &document_dir,
        &head,
        OP_EDIT,
        REV_EDIT,
        &pairs(&[
            (
                "--placed-section",
                &format!("{SECTION}:right:1500:positive:1:0,0"),
            ),
            (
                "--placed-detail",
                &format!("{DETAIL}:{VIEW}:15,20:20:20:100,100"),
            ),
        ]),
    );
    assert_eq!(code, 0, "{placed:?}");
    let sections = list(&placed, "placed_sections");
    assert_eq!(sections.len(), 1);
    assert_eq!(text(&sections[0], "id"), SECTION);
    assert_eq!(text(&sections[0], "plane"), "right");
    assert_eq!(number(at(&sections[0], &["cut_mm"])), 1500.0);
    assert_eq!(text(&sections[0], "side"), "positive");
    let details = list(&placed, "placed_details");
    assert_eq!(details.len(), 1);
    assert_eq!(text(&details[0], "id"), DETAIL);
    assert_eq!(text(&details[0], "parent"), VIEW);
    assert_eq!(number(at(&details[0], &["radius_mm"])), 20.0);
    let edit_head = text(&placed, "revision_id");

    // The exact replay returns the same revision; a changed placement under the same
    // operation id is a different request and refuses.
    let (code, replayed) = edit_views(
        &document_dir,
        &head,
        OP_EDIT,
        REV_EDIT,
        &pairs(&[
            (
                "--placed-section",
                &format!("{SECTION}:right:1500:positive:1:0,0"),
            ),
            (
                "--placed-detail",
                &format!("{DETAIL}:{VIEW}:15,20:20:20:100,100"),
            ),
        ]),
    );
    assert_eq!(code, 0, "{replayed:?}");
    assert_eq!(text(&replayed, "revision_id"), edit_head);
    let (code, refusal) = edit_views(
        &document_dir,
        &head,
        OP_EDIT,
        REV_EDIT,
        &pairs(&[
            (
                "--placed-section",
                &format!("{SECTION}:right:1500:positive:1:0,0"),
            ),
            (
                "--placed-detail",
                &format!("{DETAIL}:{VIEW}:15,20:21:20:100,100"),
            ),
        ]),
    );
    assert_eq!(code, 2);
    assert_eq!(text(&refusal, "code"), "OP_ID_REUSE");

    // Identity and parent rules refuse without committing: a section cannot take the
    // primary view's UUID (identity), and a detail needs an existing view as its parent
    // (admission), while an unknown side token is a CLI parse refusal.
    let bad: [(&str, Vec<(&str, String)>); 3] = [
        (
            "DUPLICATE_ID",
            vec![(
                "--placed-section",
                format!("{VIEW}:right:1500:positive:1:0,0"),
            )],
        ),
        (
            "INVALID_ARGUMENT",
            vec![(
                "--placed-detail",
                format!("{DETAIL}:{FOREIGN}:0,0:5:20:0,0"),
            )],
        ),
        (
            "INVALID_ARGUMENT",
            vec![(
                "--placed-section",
                format!("{FOREIGN}:right:1500:sideways:1:0,0"),
            )],
        ),
    ];
    for (expected_code, refusal_pairs) in bad {
        let flat: Vec<String> = refusal_pairs
            .iter()
            .flat_map(|(key, value)| [(*key).to_owned(), value.clone()])
            .collect();
        let (code, refusal) = edit_views(&document_dir, &edit_head, OP_CLEAR, REV_CLEAR, &flat);
        assert_eq!(code, 2, "{refusal:?}");
        assert_eq!(text(&refusal, "code"), expected_code, "{refusal:?}");
    }

    // A drawn-object edit is a different collection: the derived views carry over.
    let drafted = f.ok(
        "edit-drafting",
        &[
            "--document-dir",
            &document_dir,
            "--expected-head",
            &edit_head,
            "--op-id",
            OP_DRAFT,
            "--revision-id",
            REV_DRAFT,
            "--actor-id",
            ACTOR,
            "--layer",
            &format!("{LINE_LAYER}:Paper:on"),
            "--drafting",
            &format!("{LINE}:line:0,0,100,0:{LINE_LAYER}:continuous"),
        ],
    );
    assert_eq!(list(&drafted, "placed_sections").len(), 1);
    assert_eq!(list(&drafted, "placed_details").len(), 1);

    // Replacement is one whole-document operation: an edit without placement options
    // clears every derived view, and inspect reports the cleared state.
    let (code, cleared) = edit_views(
        &document_dir,
        &text(&drafted, "revision_id"),
        OP_CLEAR,
        REV_CLEAR,
        &[],
    );
    assert_eq!(code, 0, "{cleared:?}");
    assert!(list(&cleared, "placed_sections").is_empty());
    assert!(list(&cleared, "placed_details").is_empty());
    let inspected = f.ok(
        "inspect-drawing",
        &[
            "--document-dir",
            &document_dir,
            "--model-store",
            f.shared.root.join("store").to_str().unwrap(),
        ],
    );
    assert!(list(&inspected, "placed_sections").is_empty());
    assert!(list(&inspected, "placed_details").is_empty());
}

#[test]
fn a_section_shows_the_visible_half_and_its_exact_cut_contour() {
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
    let (code, placed) = edit_views(
        &document_dir,
        &head,
        OP_EDIT,
        REV_EDIT,
        &pairs(&[
            (
                "--placed-section",
                &format!("{SECTION}:right:1500:positive:1:0,0"),
            ),
            (
                "--placed-detail",
                &format!("{DETAIL}:{VIEW}:15,20:20:20:100,100"),
            ),
        ]),
    );
    assert_eq!(code, 0, "{placed:?}");

    let source = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    let snapshot = source.inspect().unwrap().snapshot;
    let pin = source.pin();
    let drawing = document(
        pin,
        vec![
            mid_section(SECTION, CutSide::Positive, 1500.0),
            mid_section(SECTION_FAR, CutSide::Positive, 5000.0),
            mid_section(SECTION_ALL, CutSide::Negative, 5000.0),
        ],
        vec![detail(DETAIL, 20.0), detail(DETAIL_CLIP, 5.0)],
    );
    let section = &drawing.sections()[0];
    let geometry =
        resolved_section(projection::resolve_section(&drawing, section, &snapshot).unwrap());

    // Eight wireframe edges stay visible: the four depth-parallel edges end exactly on the
    // cut, and the four in-plane edges of the far half project as the profile's three free
    // sides (the fourth side is the cut contour itself).
    let edges = endpoints(&geometry.edges);
    assert_eq!(edges.len(), 8);
    let straight: Vec<[[f64; 2]; 2]> = edges
        .iter()
        .filter(|segment| segment[0] != segment[1])
        .copied()
        .collect();
    assert_eq!(
        straight,
        vec![
            [[1950.0, -25.0], [1950.0, 25.0]],
            [[1950.0, -25.0], [2050.0, -25.0]],
            [[1950.0, 25.0], [2050.0, 25.0]],
            [[2050.0, -25.0], [2050.0, 25.0]],
        ]
    );
    // The cut contour is the exact 100 x 50 profile rectangle in boundary order.
    let mut contour = geometry.cut_paper_mm.clone();
    contour.sort_by(|p, q| p[0].total_cmp(&q[0]).then(p[1].total_cmp(&q[1])));
    let mut want = CORNERS_YZ;
    want.sort_by(|p, q| p[0].total_cmp(&q[0]).then(p[1].total_cmp(&q[1])));
    assert_eq!(contour, want);

    // A cut beyond the member hides everything on one side and shows the whole wireframe
    // on the other; neither carries a contour.
    let miss = resolved_section(
        projection::resolve_section(&drawing, &drawing.sections()[1], &snapshot).unwrap(),
    );
    assert!(miss.edges.is_empty() && miss.cut_paper_mm.is_empty());
    let all = resolved_section(
        projection::resolve_section(&drawing, &drawing.sections()[2], &snapshot).unwrap(),
    );
    assert_eq!(all.edges.len(), 12);
    assert!(all.cut_paper_mm.is_empty());

    // The detail magnifies exactly the parent's region: the whole part fits radius 20, so
    // every edge maps by the 5x magnification around (15, 20) onto (100, 100).
    let detail_geometry = resolved_detail(
        projection::resolve_detail(&drawing, &drawing.details()[0], &snapshot).unwrap(),
    );
    assert_eq!(detail_geometry.edges.len(), 12);
    assert_eq!(detail_geometry.region_center_mm, [100.0, 100.0]);
    assert_eq!(detail_geometry.region_radius_mm, 100.0);
    assert!(
        endpoints(&detail_geometry.edges)
            .iter()
            .any(|segment| segment.contains(&[25.0, 98.75]) && segment.contains(&[175.0, 98.75])),
        "the bottom edge must map to the magnified position: {detail_geometry:?}"
    );
    // A radius-5 region keeps only the four long edges crossing it, each ending exactly
    // on the mapped circle of radius 25 around (100, 100).
    let clipped = resolved_detail(
        projection::resolve_detail(&drawing, &drawing.details()[1], &snapshot).unwrap(),
    );
    assert_eq!(clipped.edges.len(), 4);
    for segment in endpoints(&clipped.edges) {
        for point in segment {
            let d = (point[0] - 100.0).hypot(point[1] - 100.0);
            assert!(
                (d - 25.0).abs() < 1e-9,
                "{point:?} is not on the mapped circle"
            );
        }
    }

    // A model change marks the section and the detail stale together with the document.
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
            PLATE,
        ],
    );
    let moved = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    let moved_snapshot = moved.inspect().unwrap().snapshot;
    assert_eq!(
        projection::resolve_section(&drawing, section, &moved_snapshot)
            .unwrap_err()
            .code,
        ErrorCode::StaleWorkspace
    );
    assert_eq!(
        projection::resolve_detail(&drawing, &drawing.details()[0], &moved_snapshot)
            .unwrap_err()
            .code,
        ErrorCode::StaleWorkspace
    );
}
