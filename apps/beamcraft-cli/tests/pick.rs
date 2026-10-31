// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, members and a foreign USDA source, MPL-2.0.

//! V04 end to end: `list-feature`, `lookup-feature` and `element-edges` through the real CLI binary
//! against committed revisions. Every expected position is derived here from the command inputs and the
//! F06 frame rules, never read back from the engine. FeatureIds survive a move and differ on a
//! copy; no answer contains an index and an index is refused as an address; the queries write
//! nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::Json;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[path = "drawing/projection.rs"]
mod drawing_projection;

#[path = "drawing/archive.rs"]
mod drawing_archive;

#[path = "drawing/lifecycle.rs"]
mod drawing_lifecycle;

#[path = "drawing/derived.rs"]
mod drawing_derived;

#[path = "drawing/export.rs"]
mod drawing_export;

#[path = "drawing/drafting.rs"]
mod drawing_drafting;

const PLATE: &str = "019a7bf0-0000-7000-8000-000000700001";
const MEMBER: &str = "019a7bf0-0000-7000-8000-000000700002";
const COPY: &str = "019a7bf0-0000-7000-8000-000000700003";
const HOLED: &str = "019a7bf0-0000-7000-8000-000000700004";
const IBEAM: &str = "019a7bf0-0000-7000-8000-000000700005";
const REFERENCE: &str = "019a7bf0-0000-7000-8000-000000700006";
const SITE: &str = "#usda 1.0\n(\n    defaultPrim = \"Site\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\ndef Xform \"Site\"\n{\n    def Mesh \"Body\"\n    {\n        point3f[] points = [(0, 0, 0), (1, 0, 0), (1, 1, 0)]\n        int[] faceVertexCounts = [3]\n        int[] faceVertexIndices = [0, 1, 2]\n        uniform token subdivisionScheme = \"none\"\n    }\n}\n";

use crate::support;

use support::{at, text};

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("v04-tests"),
        }
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    /// A refused query: the F04 code and the F06 reason token.
    fn refused(&self, command: &str, extra: &[&str]) -> (String, Option<String>) {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        let reason = body
            .get("reason")
            .ok()
            .and_then(|r| r.string().ok())
            .map(str::to_owned);
        (text(&body, "code"), reason)
    }
    fn head(&self) -> String {
        text(&self.ok("open-revision", &[]), "revision_id")
    }
    /// A 1000 x 500 x 10 plate at (100, 200, 300) and a 3000 mm beam along +X with a 100 x 50
    /// profile, its axis from (0, 2000, 0) to (3000, 2000, 0).
    fn model(&self) {
        self.ok(
            "create-plate",
            &[
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--translation",
                "100,200,300",
                "--new-id",
                PLATE,
            ],
        );
        let profile = text(
            at(
                &self.ok("define-profile", &["--width", "100", "--height", "50"]),
                &["definition"],
            ),
            "digest",
        );
        self.ok(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "3000,0,0",
                "--translation",
                "0,2000,0",
                "--profile-digest",
                &profile,
                "--new-id",
                MEMBER,
            ],
        );
    }
}

fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
fn number(json: &Json) -> f64 {
    match json {
        Json::Number(n) => Some(n.parse().unwrap()),
        _ => None,
    }
    .expect("a JSON number")
}
fn triple(json: &Json) -> [f64; 3] {
    let v: Vec<f64> = json.array().unwrap().iter().map(number).collect();
    v.try_into().unwrap()
}
fn fid(element: &str, local: &str) -> String {
    format!("fid1:{element}:{local}")
}
/// World positions of an oblique member carry the rounding of the irrational frame entries:
/// a 1e-6 absolute slack, far above it and far below any engineering band.
#[test]
fn listed_member_features_are_complete_pinned_read_only_and_stable_after_move_and_copy() {
    let f = Fixture::new();
    f.model();
    let head = f.head();
    let listed = f.ok("list-feature", &["--id", MEMBER]);
    assert_eq!(text(&listed, "outcome"), "features_listed");
    assert_eq!(text(&listed, "revision_id"), head);
    assert_eq!(text(&listed, "element_id"), MEMBER);
    let features = list(&listed, "features");
    assert_eq!(features.len(), 26);
    for (kind, count) in [("face", 6), ("edge", 12), ("vertex", 8)] {
        assert_eq!(
            features.iter().filter(|v| text(v, "kind") == kind).count(),
            count
        );
    }
    let ids: Vec<_> = features.iter().map(|v| text(v, "feature_id")).collect();
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        26
    );
    assert!(
        ids.iter()
            .all(|id| id.starts_with(&format!("fid1:{MEMBER}:member:")))
    );
    let near = fid(MEMBER, "member:v.x0.y0.z0");
    let far = fid(MEMBER, "member:v.x0.y0.z1");
    assert!(ids.contains(&near) && ids.contains(&far));
    assert_eq!(
        triple(at(
            &f.ok("lookup-feature", &["--feature-id", &near]),
            &["world", "position"]
        )),
        [0.0, 1950.0, -25.0]
    );
    assert_eq!(
        triple(at(
            &f.ok("lookup-feature", &["--feature-id", &far]),
            &["world", "position"]
        )),
        [3000.0, 1950.0, -25.0]
    );
    assert_eq!(
        f.head(),
        head,
        "feature queries cannot write a model revision"
    );
    let mut all_keys = Vec::new();
    keys(&listed, &mut all_keys);
    assert!(
        all_keys.iter().all(|key| !key.contains("index")
            && !key.contains("triangle")
            && !key.contains("slot"))
    );
    f.ok(
        "update-element",
        &["--id", MEMBER, "--translation", "10,20,30"],
    );
    let moved = f.ok("list-feature", &["--id", MEMBER]);
    assert_eq!(list(&moved, "features"), features);
    assert_ne!(
        text(&moved, "model_manifest"),
        text(&listed, "model_manifest")
    );
    assert_ne!(
        text(&moved, "workspace_generation"),
        text(&listed, "workspace_generation")
    );
    assert_eq!(
        triple(at(
            &f.ok("lookup-feature", &["--feature-id", &near]),
            &["world", "position"]
        )),
        [10.0, -30.0, 5.0]
    );
    f.ok(
        "copy-element",
        &["--id", MEMBER, "--new-id", COPY, "--offset", "0,0,100"],
    );
    let copied = f.ok("list-feature", &["--id", COPY]);
    let copied_ids: Vec<_> = list(&copied, "features")
        .iter()
        .map(|v| text(v, "feature_id"))
        .collect();
    assert_eq!(
        copied_ids,
        ids.iter()
            .map(|id| id.replace(MEMBER, COPY))
            .collect::<Vec<_>>()
    );
    assert!(ids.iter().all(|id| !copied_ids.contains(id)));
    f.ok("delete-element", &["--id", MEMBER]);
    assert_eq!(
        f.refused("list-feature", &["--id", MEMBER]),
        ("NOT_FOUND".into(), Some("GEO_FEATURE_NOT_FOUND".into()))
    );
    assert_eq!(
        list(&f.ok("list-feature", &["--id", COPY]), "features").len(),
        26
    );
}

#[test]
fn feature_listing_supports_named_non_box_geometry_and_refuses_numeric_addresses() {
    let f = Fixture::new();
    f.model();
    f.ok(
        "create-plate",
        &[
            "--width",
            "400",
            "--length",
            "400",
            "--thickness",
            "10",
            "--hole-rect",
            "100,100,50,50",
            "--new-id",
            HOLED,
        ],
    );
    let profile = text(
        at(
            &f.ok(
                "define-profile",
                &[
                    "--family", "i_shape", "--h", "200", "--b", "100", "--tw", "6", "--tf", "10",
                    "--r", "0",
                ],
            ),
            &["definition"],
        ),
        "digest",
    );
    f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &profile,
            "--new-id",
            IBEAM,
        ],
    );
    let head = f.head();
    for id in [HOLED, IBEAM] {
        assert!(!list(&f.ok("list-feature", &["--id", id]), "features").is_empty());
    }
    assert_eq!(
        f.refused(
            "list-feature",
            &["--id", "019a7bf0-0000-7000-8000-000000700099"]
        ),
        ("NOT_FOUND".into(), Some("GEO_FEATURE_NOT_FOUND".into()))
    );
    for option in ["--triangle", "--vertex-index", "--instance"] {
        let (_, reason) = f.refused("list-feature", &["--id", MEMBER, option, "0"]);
        assert_eq!(reason.as_deref(), Some("GEO_TRIANGLE_ADDRESS_FORBIDDEN"));
    }
    assert_eq!(f.refused("list-feature", &["--id", "0"]).0, "INVALID_ID");
    assert_eq!(f.head(), head);
}
/// Every key anywhere in a JSON value.
fn keys(json: &Json, out: &mut Vec<String>) {
    match json {
        Json::Object(map) => {
            for (k, v) in map {
                out.push(k.clone());
                keys(v, out);
            }
        }
        Json::Array(items) => items.iter().for_each(|v| keys(v, out)),
        _ => {}
    }
}

#[test]
fn feature_ids_survive_a_move_differ_on_a_copy_and_indices_are_refused() {
    let f = Fixture::new();
    f.model();
    let corner = fid(PLATE, "plate:v.x1.y1.z1");
    // A move keeps the name and moves the feature (F06 section 5 rule 4).
    f.ok("update-element", &["--id", PLATE, "--translation", "0,0,0"]);
    let found = f.ok("lookup-feature", &["--feature-id", &corner]);
    assert_eq!(
        triple(at(&found, &["world", "position"])),
        [1000.0, 500.0, 10.0]
    );
    // A copy has new FeatureIds with the same local parts (rule 5).
    f.ok(
        "copy-element",
        &["--id", PLATE, "--new-id", COPY, "--offset", "0,0,100"],
    );
    let copied = f.ok(
        "lookup-feature",
        &["--feature-id", &fid(COPY, "plate:v.x1.y1.z1")],
    );
    assert_eq!(
        triple(at(&copied, &["world", "position"])),
        [1000.0, 500.0, 110.0]
    );
    assert_eq!(text(&copied, "local_id"), text(&found, "local_id"));

    // Indices are never addresses (rule 8); dangling and malformed ids are told apart.
    let forbidden = Some("GEO_TRIANGLE_ADDRESS_FORBIDDEN".to_owned());
    assert_eq!(
        f.refused("lookup-feature", &["--feature-id", &fid(PLATE, "plate:7")]),
        ("UNSUPPORTED_PROPERTY".to_owned(), forbidden.clone())
    );
    assert_eq!(
        f.refused("list-feature", &["--id", PLATE, "--triangle", "3"]),
        ("UNSUPPORTED_PROPERTY".to_owned(), forbidden)
    );
    assert_eq!(
        f.refused(
            "lookup-feature",
            &[
                "--feature-id",
                &fid("019a7bf0-0000-7000-8000-0000007000ff", "plate:f.z1")
            ]
        ),
        (
            "NOT_FOUND".to_owned(),
            Some("GEO_FEATURE_NOT_FOUND".to_owned())
        )
    );
    assert_eq!(
        f.refused(
            "lookup-feature",
            &["--feature-id", &fid(MEMBER, "plate:f.z1")]
        )
        .1,
        Some("GEO_FEATURE_NOT_FOUND".to_owned())
    );
    assert_eq!(
        f.refused("lookup-feature", &["--feature-id", "fid1:nope"]),
        (
            "INVALID_ARGUMENT".to_owned(),
            Some("GEO_FEATURE_ID_INVALID".to_owned())
        )
    );
}

#[test]
fn foreign_references_are_skipped_and_non_box_bodies_have_resolvable_features() {
    let f = Fixture::new();
    f.model();
    f.ok(
        "create-plate",
        &[
            "--width",
            "400",
            "--length",
            "400",
            "--thickness",
            "10",
            "--hole-rect",
            "100,100,50,50",
            "--translation",
            "5000,0,0",
            "--new-id",
            HOLED,
        ],
    );
    // An I-section member along +X at y = 400: h = 200 is world-up (F06 P-2), so the top
    // flange's upper surface is at z = 100.
    let profile = text(
        at(
            &f.ok(
                "define-profile",
                &[
                    "--family", "i_shape", "--h", "200", "--b", "100", "--tw", "6", "--tf", "10",
                    "--r", "0",
                ],
            ),
            &["definition"],
        ),
        "digest",
    );
    f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--translation",
            "0,400,0",
            "--profile-digest",
            &profile,
            "--new-id",
            IBEAM,
        ],
    );
    let source = f.shared.root.join("site.usda");
    fs::write(&source, SITE).unwrap();
    f.ok(
        "attach-reference",
        &[
            "--source",
            source.to_str().unwrap(),
            "--translation",
            "8000,0,0",
            "--new-id",
            REFERENCE,
        ],
    );
    let head = f.head();
    // Named bodies list resolvable semantic FeatureIds; the foreign reference has no engineering
    // features and listing changes nothing.
    for (element, prefix) in [(HOLED, "fid1:"), (IBEAM, "fid2:")] {
        let listed = f.ok("list-feature", &["--id", element]);
        let features = list(&listed, "features");
        assert!(!features.is_empty(), "{element}");
        assert!(
            features
                .iter()
                .all(|feature| text(feature, "feature_id").starts_with(prefix))
        );
        let feature_id = text(&features[0], "feature_id");
        f.ok("lookup-feature", &["--feature-id", &feature_id]);
    }
    f.ok(
        "lookup-feature",
        &["--feature-id", &fid(HOLED, "plate:f.z1")],
    );
    assert_eq!(f.head(), head);
}
