// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D02 actual CLI projections checked against input-derived closed forms, without renderer data.

use super::*;

const DRAWING: &str = "01900000-0000-7000-8000-000000000001";
const VIEW: &str = "01900000-0000-7000-8000-000000000002";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000003";

fn model(f: &Fixture) {
    let definition = f.ok("define-profile", &["--width", "100", "--height", "50"]);
    let digest = text(at(&definition, &["definition"]), "digest");
    f.ok(
        "create-member",
        &[
            "--profile-digest",
            &digest,
            "--start",
            "0,0,0",
            "--end",
            "1000,0,0",
            "--translation",
            "10,20,30",
            "--new-id",
            MEMBER,
        ],
    );
}

fn args(owner: &str, a: &str, b: &str) -> Vec<String> {
    [
        "--drawing-id",
        DRAWING,
        "--view-id",
        VIEW,
        "--dimension-id",
        DIMENSION,
        "--id",
        owner,
        "--anchor-a",
        a,
        "--anchor-b",
        b,
    ]
    .map(str::to_owned)
    .into()
}

fn preview(f: &Fixture, owner: &str, plane: &str, scale: &str) -> Json {
    let a = format!("fid1:{owner}:member:v.x0.y0.z0");
    let b = format!("fid1:{owner}:member:v.x0.y0.z1");
    let mut options = args(owner, &a, &b);
    options.extend(["--plane", plane, "--scale", scale].map(str::to_owned));
    f.ok(
        "preview-drawing",
        &options.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

fn pair(json: &Json) -> [f64; 2] {
    let values = json.array().unwrap();
    assert_eq!(values.len(), 2);
    [number(&values[0]), number(&values[1])]
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[test]
fn a_real_member_has_named_wireframe_and_scale_independent_projected_dimension() {
    let f = Fixture::new();
    model(&f);
    let head = f.head();
    let result = preview(&f, MEMBER, "front", "100");
    assert_eq!(result, preview(&f, MEMBER, "front", "100"));
    assert_eq!(text(&result, "drawing_id"), DRAWING);
    assert_eq!(text(&result, "view_id"), VIEW);
    assert_eq!(text(&result, "dimension_id"), DIMENSION);
    assert_eq!(text(at(&result, &["source"]), "revision_id"), head);
    let listed = f.ok("list-feature", &["--id", MEMBER]);
    assert_eq!(
        text(at(&result, &["source"]), "manifest"),
        text(&listed, "model_manifest")
    );
    assert_eq!(
        text(at(&result, &["source"]), "generation"),
        text(&listed, "workspace_generation")
    );
    let edges = list(at(&result, &["body"]), "edges");
    assert_eq!(edges.len(), 12);
    for edge in edges {
        assert!(text(edge, "feature_id").starts_with(&format!("fid1:{MEMBER}:member:e.")));
    }
    // e.x0.y0 is the member's length edge. Width is along model Y, height along Z.
    let edge = edges
        .iter()
        .find(|e| text(e, "feature_id").ends_with("e.x0.y0"))
        .unwrap();
    let ends = list(edge, "paper_mm");
    assert_eq!(pair(&ends[0]), [0.1, 0.05]);
    assert_eq!(pair(&ends[1]), [10.1, 0.05]);
    let d = at(&result, &["dimension"]);
    assert_eq!(text(d, "state"), "resolved");
    assert_eq!(number(at(d, &["projected_length_mm"])), 1000.0);
    assert_eq!(text(d, "label_mm"), "1000");
    let line = list(d, "line_paper_mm");
    assert_eq!(pair(&line[0]), [0.1, 10.05]);
    assert_eq!(pair(&line[1]), [10.1, 10.05]);
    let enlarged = preview(&f, MEMBER, "front", "50");
    assert_eq!(
        number(at(&enlarged, &["dimension", "projected_length_mm"])),
        1000.0
    );
    let points = list(at(&enlarged, &["dimension"]), "anchors_paper_mm");
    near(pair(&points[1])[0] - pair(&points[0])[0], 20.0);
    let top = preview(&f, MEMBER, "top", "100");
    assert_eq!(
        pair(&list(at(&top, &["dimension"]), "anchors_paper_mm")[0]),
        [0.1, -0.3]
    );
    // Distinct corners differ along X/Y/Z. Front's label measures X/Z only, not true 3D length.
    let a = format!("fid1:{MEMBER}:member:v.x0.y0.z0");
    let b = format!("fid1:{MEMBER}:member:v.x1.y1.z1");
    let options = args(MEMBER, &a, &b);
    let diagonal = f.ok(
        "preview-drawing",
        &options.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    near(
        number(at(&diagonal, &["dimension", "projected_length_mm"])),
        (1_000_000.0_f64 + 2500.0).sqrt(),
    );
    assert_eq!(f.head(), head);
}

#[test]
fn collapse_delete_copy_and_unsupported_body_never_reuse_an_old_numeric_value() {
    let f = Fixture::new();
    model(&f);
    let collapsed = preview(&f, MEMBER, "right", "100");
    assert_eq!(text(at(&collapsed, &["body"]), "state"), "resolved");
    assert_eq!(
        text(at(&collapsed, &["dimension"]), "reason"),
        "collapsed_projection"
    );
    assert_eq!(
        at(&collapsed, &["dimension", "projected_length_mm"]),
        &Json::Null
    );
    assert_eq!(at(&collapsed, &["dimension", "label_mm"]), &Json::Null);
    let first = preview(&f, MEMBER, "front", "100");
    f.ok(
        "update-element",
        &[
            "--id",
            MEMBER,
            "--name",
            "Renamed",
            "--translation",
            "110,20,30",
        ],
    );
    let moved = preview(&f, MEMBER, "front", "100");
    assert_eq!(at(&first, &["anchor_ids"]), at(&moved, &["anchor_ids"]));
    assert_ne!(at(&first, &["source"]), at(&moved, &["source"]));
    assert_eq!(
        number(at(&moved, &["dimension", "projected_length_mm"])),
        1000.0
    );
    near(
        pair(&list(at(&moved, &["dimension"]), "anchors_paper_mm")[0])[0],
        1.1,
    );
    f.ok("copy-element", &["--id", MEMBER, "--new-id", COPY]);
    let copy = preview(&f, COPY, "front", "100");
    assert_ne!(at(&copy, &["anchor_ids"]), at(&moved, &["anchor_ids"]));
    f.ok("delete-element", &["--id", MEMBER]);
    let deleted = preview(&f, MEMBER, "front", "100");
    assert_eq!(
        text(at(&deleted, &["dimension"]), "reason"),
        "missing_owner"
    );
    assert_eq!(
        at(&deleted, &["dimension", "projected_length_mm"]),
        &Json::Null
    );
    assert_eq!(at(&deleted, &["anchor_ids"]), at(&first, &["anchor_ids"]));
    assert_eq!(
        text(
            at(&preview(&f, COPY, "front", "100"), &["dimension"]),
            "state"
        ),
        "resolved"
    );
    // A plate with the selected UUID does not acquire member features from its bounding box.
    f.ok(
        "create-plate",
        &[
            "--width",
            "100",
            "--length",
            "200",
            "--thickness",
            "10",
            "--new-id",
            MEMBER,
        ],
    );
    let wrong_body = preview(&f, MEMBER, "front", "100");
    assert_eq!(
        text(at(&wrong_body, &["dimension"]), "reason"),
        "unsupported_body"
    );
    assert_eq!(
        at(&wrong_body, &["dimension", "projected_length_mm"]),
        &Json::Null
    );
}

#[test]
fn malformed_or_unbounded_preview_refuses_without_writing_the_model() {
    let f = Fixture::new();
    model(&f);
    let head = f.head();
    let a = format!("fid1:{MEMBER}:member:v.x0.y0.z0");
    let b = format!("fid1:{MEMBER}:member:v.x0.y0.z1");
    let good = args(MEMBER, &a, &b);
    for (key, value, expected) in [
        ("--scale", "NaN", "NON_FINITE_NUMBER"),
        ("--scale", "0", "INVALID_ARGUMENT"),
        ("--offset", "10001", "INVALID_ARGUMENT"),
        ("--offset", "inf", "NON_FINITE_NUMBER"),
        ("--origin", "10001,0", "INVALID_ARGUMENT"),
        ("--decimals", "4", "INVALID_ARGUMENT"),
        ("--plane", "camera", "INVALID_ARGUMENT"),
        ("--origin", "1,2,3", "INVALID_ARGUMENT"),
    ] {
        let mut options = good.clone();
        options.extend([key.to_owned(), value.to_owned()]);
        assert_eq!(
            f.refused(
                "preview-drawing",
                &options.iter().map(String::as_str).collect::<Vec<_>>()
            )
            .0,
            expected
        );
    }
    for (anchor, expected) in [
        ("triangle:0".to_owned(), "INVALID_ARGUMENT"),
        (format!("fid1:{MEMBER}:member:0"), "UNSUPPORTED_PROPERTY"),
        (format!("fid1:{MEMBER}:member:f.x0"), "INVALID_ARGUMENT"),
        (format!("fid1:{COPY}:member:v.x0.y0.z1"), "INVALID_ARGUMENT"),
    ] {
        let options = args(MEMBER, &a, &anchor);
        assert_eq!(
            f.refused(
                "preview-drawing",
                &options.iter().map(String::as_str).collect::<Vec<_>>()
            )
            .0,
            expected
        );
    }
    // A valid bounded view origin can still make the resulting geometry exceed the output cap.
    let mut options = good.clone();
    options.extend(["--origin", "10000,0"].map(str::to_owned));
    assert_eq!(
        f.refused(
            "preview-drawing",
            &options.iter().map(String::as_str).collect::<Vec<_>>()
        )
        .0,
        "LIMIT_EXCEEDED"
    );
    let mut duplicate = good.clone();
    let slot = duplicate.iter().position(|s| s == VIEW).unwrap();
    duplicate[slot] = DRAWING.to_owned();
    assert_eq!(
        f.refused(
            "preview-drawing",
            &duplicate.iter().map(String::as_str).collect::<Vec<_>>()
        )
        .0,
        "DUPLICATE_ID"
    );
    assert_eq!(f.head(), head);
}

#[test]
fn real_cli_exports_reopenable_usd_intent_without_overwriting_or_modifying_the_model() {
    use beamcraft_cli::drawing::codec;
    use beamcraft_layer_engine::{Asset, Cancellation, Control, Identity};
    use beamcraft_model::digest::ContentDigest;
    use std::time::{Duration, Instant};
    let f = Fixture::new();
    model(&f);
    let head = f.head();
    let a = format!("fid1:{MEMBER}:member:v.x0.y0.z0");
    let b = format!("fid1:{MEMBER}:member:v.x0.y0.z1");
    let output = f.root.join("intent.usda");
    let mut options = args(MEMBER, &a, &b);
    options.extend(["--intent-out".into(), output.to_str().unwrap().into()]);
    let argv = options.iter().map(String::as_str).collect::<Vec<_>>();
    let result = f.ok("preview-drawing", &argv);
    let bytes = fs::read(&output).unwrap();
    let digest = text(&result, "intent_digest")
        .parse::<ContentDigest>()
        .unwrap();
    let decoded = codec::decode(
        Asset {
            name: codec::INTENT_LAYER.into(),
            digest,
            bytes: bytes.clone(),
        },
        Identity {
            project: text(at(&result, &["source"]), "project_id"),
            workspace: DRAWING.into(),
            revision: digest,
            generation: 1,
        },
        &Control {
            cancellation: Cancellation::default(),
            deadline: Instant::now() + Duration::from_secs(30),
        },
    )
    .unwrap();
    assert_eq!(decoded.spec().id.to_string(), DRAWING);
    assert_eq!(decoded.spec().source.revision.to_string(), head);
    assert_eq!(
        decoded
            .spec()
            .dimension
            .anchors
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>(),
        list(&result, "anchor_ids")
            .iter()
            .map(|j| j.string().unwrap().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(f.refused("preview-drawing", &argv).0, "INVALID_ARGUMENT");
    assert_eq!(fs::read(&output).unwrap(), bytes);
    assert_eq!(f.head(), head);
    let missing = f.root.join("not-created.usda");
    let mut invalid = args(MEMBER, &a, &b);
    invalid.extend([
        "--intent-out".into(),
        missing.to_str().unwrap().into(),
        "--scale".into(),
        "0".into(),
    ]);
    assert_eq!(
        f.refused(
            "preview-drawing",
            &invalid.iter().map(String::as_str).collect::<Vec<_>>()
        )
        .0,
        "INVALID_ARGUMENT"
    );
    assert!(!missing.exists());
}
