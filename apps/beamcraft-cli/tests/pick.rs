// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, members and a foreign USDA source, MPL-2.0.

//! V04 end to end: `pick`, `feature-lookup` and `snap` through the real CLI binary against
//! committed revisions. Every expected position is derived here from the command inputs and the
//! F06 frame rules, never read back from the engine. FeatureIds survive a move and differ on a
//! copy; no answer contains an index and an index is refused as an address; the queries write
//! nothing; a foreign reference is never a hit; a body without box features (an I-section
//! member, a plate with a through hole) has no FeatureIds but is picked at element level by its
//! mesh, through the hole as well as onto the material.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const PLATE: &str = "019a7bf0-0000-7000-8000-000000700001";
const MEMBER: &str = "019a7bf0-0000-7000-8000-000000700002";
const COPY: &str = "019a7bf0-0000-7000-8000-000000700003";
const HOLED: &str = "019a7bf0-0000-7000-8000-000000700004";
const IBEAM: &str = "019a7bf0-0000-7000-8000-000000700005";
const SITE: &str = "#usda 1.0\n(\n    defaultPrim = \"Site\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\ndef Xform \"Site\"\n{\n    def Mesh \"Body\"\n    {\n        point3f[] points = [(0, 0, 0), (1, 0, 0), (1, 1, 0)]\n        int[] faceVertexCounts = [3]\n        int[] faceVertexIndices = [0, 1, 2]\n    }\n}\n";

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("v04-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        Self { root }
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        let store = self.root.join("store").to_str().unwrap().to_owned();
        let mut args = vec![command, "--store", store.as_str()];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
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
        text(&self.ok("revision-open", &[]), "revision_id")
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
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
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
fn faces_vertices_and_axes_are_found_at_independent_positions() {
    let f = Fixture::new();
    f.model();
    let head = f.head();

    // A vertical ray over the plate's centre: the top face (z = 300 + 10) at t = 1000 - 310.
    let picked = f.ok(
        "pick",
        &[
            "--origin",
            "600,450,1000",
            "--direction",
            "0,0,-1",
            "--targets",
            "face",
        ],
    );
    assert_eq!(text(&picked, "outcome"), "picked");
    let hits = list(&picked, "hits");
    assert_eq!(hits.len(), 1);
    assert_eq!(text(&hits[0], "feature_id"), fid(PLATE, "plate:f.z1"));
    assert_eq!(text(&hits[0], "element_id"), PLATE);
    assert_eq!(number(at(&hits[0], &["t_mm"])), 690.0);
    assert_eq!(
        triple(at(&hits[0], &["point_world_mm"])),
        [600.0, 450.0, 310.0]
    );
    assert_eq!(triple(at(&hits[0], &["normal_world"])), [0.0, 0.0, 1.0]);

    // Over the beam: a horizontal member's Y_m is world up (F06 P-2), so the top is f.y1 at
    // half the 50 mm height.
    let picked = f.ok(
        "pick",
        &[
            "--origin",
            "1500,2000,1000",
            "--direction",
            "0,0,-1",
            "--targets",
            "face",
        ],
    );
    let hits = list(&picked, "hits");
    assert_eq!(text(&hits[0], "feature_id"), fid(MEMBER, "member:f.y1"));
    assert_eq!(
        triple(at(&hits[0], &["point_world_mm"])),
        [1500.0, 2000.0, 25.0]
    );

    // The plate's far top corner: local (1000, 500, 10), world plus the translation.
    let corner = fid(PLATE, "plate:v.x1.y1.z1");
    let found = f.ok("feature-lookup", &["--feature-id", &corner]);
    assert_eq!(text(&found, "feature_kind"), "vertex");
    assert_eq!(text(&found, "local_id"), "plate:v.x1.y1.z1");
    assert_eq!(
        triple(at(&found, &["local", "position"])),
        [1000.0, 500.0, 10.0]
    );
    assert_eq!(
        triple(at(&found, &["world", "position"])),
        [1100.0, 700.0, 310.0]
    );
    // A point pick 0.5 mm off that corner finds the vertex, the three edges and three faces.
    let near = f.ok(
        "pick",
        &[
            "--point",
            "1100.5,700,310",
            "--radius",
            "1",
            "--targets",
            "vertex,edge,face",
        ],
    );
    let kinds: Vec<String> = list(&near, "hits")
        .iter()
        .map(|h| text(h, "feature_kind"))
        .collect();
    assert_eq!(kinds.iter().filter(|k| *k == "vertex").count(), 1);
    assert_eq!(kinds.iter().filter(|k| *k == "edge").count(), 3);
    assert_eq!(kinds.iter().filter(|k| *k == "face").count(), 3);

    // Snap: the beam's axis start is (0, 2000, 0); the nearest axis point to (1500, 2003, 4) is
    // its middle, 5 mm away.
    let snapped = f.ok(
        "snap",
        &[
            "--point", "2,2001,1", "--radius", "5", "--kinds", "axis_end",
        ],
    );
    let c = &list(&snapped, "candidates")[0];
    assert_eq!(text(c, "element_id"), MEMBER);
    assert_eq!(triple(at(c, &["point_world_mm"])), [0.0, 2000.0, 0.0]);
    assert_eq!(number(at(c, &["axis_parameter"])), 0.0);
    let snapped = f.ok(
        "snap",
        &[
            "--point",
            "1500,2003,4",
            "--radius",
            "10",
            "--kinds",
            "axis",
        ],
    );
    let c = &list(&snapped, "candidates")[0];
    assert_eq!(text(c, "kind"), "axis");
    assert_eq!(triple(at(c, &["point_world_mm"])), [1500.0, 2000.0, 0.0]);
    assert_eq!(number(at(c, &["axis_parameter"])), 0.5);
    assert_eq!(number(at(c, &["distance_mm"])), 5.0);
    let snapped = f.ok(
        "snap",
        &[
            "--point",
            "1100,700,311",
            "--radius",
            "2",
            "--kinds",
            "vertex",
        ],
    );
    let c = &list(&snapped, "candidates")[0];
    assert_eq!(text(c, "feature_id"), corner);

    // Queries never write, and no answer carries a triangle, vertex or buffer index.
    assert_eq!(f.head(), head);
    let mut all = Vec::new();
    keys(&near, &mut all);
    keys(&found, &mut all);
    assert!(
        all.iter()
            .all(|k| !k.contains("index") && !k.contains("triangle") && !k.contains("slot")),
        "{all:?}"
    );
}

#[test]
fn feature_ids_survive_a_move_differ_on_a_copy_and_indices_are_refused() {
    let f = Fixture::new();
    f.model();
    let corner = fid(PLATE, "plate:v.x1.y1.z1");
    // A move keeps the name and moves the feature (F06 section 5 rule 4).
    f.ok("update-element", &["--id", PLATE, "--translation", "0,0,0"]);
    let found = f.ok("feature-lookup", &["--feature-id", &corner]);
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
        "feature-lookup",
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
        f.refused("feature-lookup", &["--feature-id", &fid(PLATE, "plate:7")]),
        ("UNSUPPORTED_PROPERTY".to_owned(), forbidden.clone())
    );
    assert_eq!(
        f.refused(
            "pick",
            &["--point", "0,0,0", "--targets", "face", "--triangle", "3"]
        ),
        ("UNSUPPORTED_PROPERTY".to_owned(), forbidden)
    );
    assert_eq!(
        f.refused(
            "feature-lookup",
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
            "feature-lookup",
            &["--feature-id", &fid(MEMBER, "plate:f.z1")]
        )
        .1,
        Some("GEO_FEATURE_NOT_FOUND".to_owned())
    );
    assert_eq!(
        f.refused("feature-lookup", &["--feature-id", "fid1:nope"]),
        (
            "INVALID_ARGUMENT".to_owned(),
            Some("GEO_FEATURE_ID_INVALID".to_owned())
        )
    );
}

#[test]
fn foreign_references_are_never_hit_and_mesh_bodies_pick_at_element_level() {
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
    let source = f.root.join("site.usda");
    fs::write(&source, SITE).unwrap();
    f.ok(
        "reference-attach",
        &[
            "--source",
            source.to_str().unwrap(),
            "--translation",
            "8000,0,0",
        ],
    );
    let head = f.head();
    // Solid material of the holed plate (world x 5200) and of the I-beam's top flange are
    // hits at element level: no FeatureId, but selectable.
    for (element, origin, point) in [
        (HOLED, "5200,200,100", [5200.0, 200.0, 10.0]),
        (IBEAM, "1500,400,1000", [1500.0, 400.0, 100.0]),
    ] {
        let picked = f.ok(
            "pick",
            &[
                "--origin",
                origin,
                "--direction",
                "0,0,-1",
                "--targets",
                "face",
                "--max-hits",
                "1",
            ],
        );
        let hits = list(&picked, "hits");
        assert_eq!(hits.len(), 1, "{origin}");
        assert_eq!(text(&hits[0], "element_id"), element);
        assert!(matches!(at(&hits[0], &["feature_id"]), Json::Null));
        assert_eq!(text(&hits[0], "feature_kind"), "face");
        assert_eq!(triple(at(&hits[0], &["point_world_mm"])), point);
        assert!(list(&picked, "skipped").is_empty(), "{origin}");
    }
    // Through the plate's hole (world x 5100..5150, y 100..150) there is no hit.
    let picked = f.ok(
        "pick",
        &[
            "--origin",
            "5125,125,100",
            "--direction",
            "0,0,-1",
            "--targets",
            "face",
        ],
    );
    assert!(list(&picked, "hits").is_empty(), "{picked:?}");
    // The foreign content is never a hit, and nothing is skipped on the ray form: every
    // engine body was considered.
    let picked = f.ok(
        "pick",
        &[
            "--origin",
            "8000.5,0.5,100",
            "--direction",
            "0,0,-1",
            "--targets",
            "face,edge,vertex",
            "--radius",
            "1",
        ],
    );
    assert!(list(&picked, "hits").is_empty(), "{picked:?}");
    assert!(list(&picked, "skipped").is_empty(), "{picked:?}");
    // The point form measures to features, which a mesh body has none of: both stay skipped.
    let picked = f.ok(
        "pick",
        &[
            "--point",
            "5200,200,5",
            "--radius",
            "2",
            "--targets",
            "face",
        ],
    );
    let skipped: Vec<String> = list(&picked, "skipped")
        .iter()
        .map(|s| s.string().unwrap().to_owned())
        .collect();
    assert_eq!(skipped, [HOLED, IBEAM]);
    assert_eq!(
        f.refused(
            "feature-lookup",
            &["--feature-id", &fid(HOLED, "plate:f.z1")]
        )
        .1,
        Some("GEO_FEATURE_NOT_FOUND".to_owned())
    );
    assert_eq!(f.head(), head);
}

#[test]
fn bad_query_inputs_are_refused_before_any_scan() {
    let f = Fixture::new();
    f.model();
    let ray = |extra: &[&str]| {
        let mut args = vec![
            "--origin",
            "0,0,100",
            "--direction",
            "0,0,-1",
            "--targets",
            "face",
        ];
        for pair in extra.chunks(2) {
            if let Some(i) = args.iter().position(|a| *a == pair[0]) {
                args[i + 1] = pair[1];
            } else {
                args.extend_from_slice(pair);
            }
        }
        f.refused("pick", &args)
    };
    let reason = |r: (String, Option<String>)| r.1.unwrap_or(r.0);
    assert_eq!(reason(ray(&["--max-hits", "65"])), "GEO_PICK_LIMIT");
    assert_eq!(reason(ray(&["--radius", "1000001"])), "GEO_PICK_LIMIT");
    for bad in [
        ["--max-hits", "0"],
        ["--radius", "-1"],
        ["--t-max", "-5"],
        ["--direction", "0,0,0"],
        ["--targets", "face,face"],
    ] {
        assert_eq!(reason(ray(&bad)), "GEO_PICK_INPUT_INVALID", "{bad:?}");
    }
    assert_eq!(
        ray(&["--origin", "NaN,0,0"]),
        (
            "NON_FINITE_NUMBER".to_owned(),
            Some("GEO_NON_FINITE".to_owned())
        )
    );
    assert_eq!(reason(ray(&["--targets", "solid"])), "INVALID_ARGUMENT");
    assert_eq!(reason(ray(&["--point", "0,0,0"])), "INVALID_ARGUMENT");
    assert_eq!(
        reason(f.refused("snap", &["--point", "0,0,0", "--kinds", "axis,axis"])),
        "GEO_PICK_INPUT_INVALID"
    );
    // A store without a revision has no snapshot to query.
    let empty = Fixture::new();
    assert_eq!(
        empty
            .refused("pick", &["--point", "0,0,0", "--targets", "face"])
            .0,
        "NOT_FOUND"
    );
}

#[test]
fn a_small_member_far_from_the_origin_keeps_its_features() {
    let f = Fixture::new();
    let profile = text(
        at(
            &f.ok("define-profile", &["--width", "1", "--height", "1"]),
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
            "3,4,0",
            "--translation",
            "1000000000,1000000000,0",
            "--profile-digest",
            &profile,
            "--new-id",
            MEMBER,
        ],
    );
    // A placement-only move never removes FeatureIds: the body is checked in local space.
    let picked = f.ok(
        "pick",
        &[
            "--point",
            "1000000000,1000000000,0",
            "--radius",
            "2",
            "--targets",
            "vertex",
        ],
    );
    assert!(list(&picked, "skipped").is_empty(), "{picked:?}");
    assert!(!list(&picked, "hits").is_empty());
    // Local geometry is exact, not world minus translation: for an axis along (0.6, 0.8, 0),
    // X_m = (-0.8, 0.6, 0) and Y_m = +Z (F06 P-2), so v.x0.y0.z0 = (0.4, -0.3, -0.5).
    let found = f.ok(
        "feature-lookup",
        &["--feature-id", &fid(MEMBER, "member:v.x0.y0.z0")],
    );
    let local = triple(at(&found, &["local", "position"]));
    for (got, want) in local.iter().zip([0.4, -0.3, -0.5]) {
        assert!((got - want).abs() < 1e-12, "{local:?}");
    }
    let snapped = f.ok(
        "snap",
        &[
            "--point",
            "1000000000,1000000000,0",
            "--radius",
            "1",
            "--kinds",
            "axis_end",
        ],
    );
    let c = &list(&snapped, "candidates")[0];
    assert!(number(at(c, &["distance_mm"])) < 1e-3);
}
