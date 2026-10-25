// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected volumes are the closed form
// width * length * thickness (F04 section 5), never production math. The USD text reader below is
// original test code that knows only the public USDA text syntax and the standard UsdGeom
// attribute names; it uses no BeamCraft crate to read the saved layers.

//! P01 end to end through the real `beamcraft` binary: `create-plate` commits a USD revision,
//! fresh processes reopen it (`inspect-elements`, `open-revision`), and a hand-written USDA reader
//! checks the saved bytes without BeamCraft or `openusd`. Failure injection kills a real child
//! process at every commit point.
//!
//! The independent reader is exercised separately by independent_reader.rs and
//! reference_reader.rs, which run in every `cargo test`. Here the independent-reader evidence is
//! the pure-Rust `openusd` backend in a fresh process plus the hand-written USDA reader.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_cli::plate::{CreateRequest, PlateError, create_plate, inspect};
use beamcraft_host::{
    ids::IdSource,
    revision::{CommitPoint, Injected, NoFault, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::command::{CreatePlate, Placement};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p01-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        Self(root)
    }
    fn store(&self) -> PathBuf {
        self.0.join("store")
    }
    fn store_arg(&self) -> String {
        self.store().to_str().unwrap().to_owned()
    }
    fn revisions(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.store()).unwrap())
    }
    fn run(&self, args: &[&str]) -> (i32, Json) {
        run_binary(args)
    }
    fn create(&self, extra: &[&str]) -> (i32, Json) {
        let store = self.store_arg();
        let mut args = vec!["create-plate", "--store", store.as_str()];
        args.extend_from_slice(extra);
        self.run(&args)
    }
    fn inspect(&self) -> (i32, Json) {
        self.run(&["inspect-elements", "--store", &self.store_arg()])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.file_name().is_some());
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_binary(args: &[&str]) -> (i32, Json) {
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .output()
        .unwrap();
    let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    (output.status.code().unwrap(), body)
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-6 * expected.abs()
}

// ---- an independent USDA text reader ---------------------------------------------------------

/// What a plain reader of the USDA text needs: layer metadata and per-prim attribute text.
#[derive(Default, Debug)]
struct Usda {
    metadata: BTreeMap<String, String>,
    prims: BTreeMap<String, BTreeMap<String, String>>,
}

fn parse_usda(source: &str) -> Usda {
    let mut usda = Usda::default();
    let mut lines = source.lines().map(str::trim);
    assert_eq!(lines.next(), Some("#usda 1.0"));
    let mut in_metadata = false;
    let mut depth_in_dict = 0usize;
    let mut stack: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    let mut skipping = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if !in_metadata && stack.is_empty() && pending.is_none() && line == "(" {
            in_metadata = true;
            continue;
        }
        if in_metadata {
            if line == ")" && depth_in_dict == 0 {
                in_metadata = false;
            } else if line.ends_with('{') {
                depth_in_dict += 1;
            } else if line == "}" {
                depth_in_dict = depth_in_dict.saturating_sub(1);
            } else if depth_in_dict == 0 {
                let (k, v) = line.split_once(" = ").unwrap();
                usda.metadata.insert(k.to_owned(), v.to_owned());
            }
            continue;
        }
        if line == "{" {
            let name = pending.take().expect("a prim header before {");
            let parent = stack.last().cloned().unwrap_or_default();
            stack.push(format!("{parent}/{name}"));
            usda.prims.entry(stack.last().unwrap().clone()).or_default();
            continue;
        }
        if line == "}" {
            stack.pop().expect("balanced braces");
            continue;
        }
        if skipping {
            // Prim or attribute metadata (kind, displayName, interpolation): not needed here.
            skipping = line != ")";
            continue;
        }
        let is_header = ["def ", "over ", "class "]
            .iter()
            .any(|k| line.starts_with(k))
            && (line.ends_with('"') || line.ends_with(" ("));
        if is_header {
            let name = line.trim_end_matches(" (").rsplit('"').nth(1).unwrap();
            pending = Some(name.to_owned());
            skipping = line.ends_with(" (");
            continue;
        }
        let skip_after = line.ends_with(" (");
        let line = line.strip_suffix(" (").unwrap_or(line);
        let (lhs, rhs) = line.split_once(" = ").expect("an attribute line");
        let name = lhs.split_whitespace().last().unwrap();
        let path = stack.last().expect("attribute inside a prim").clone();
        usda.prims
            .get_mut(&path)
            .unwrap()
            .insert(name.to_owned(), rhs.to_owned());
        skipping = skip_after;
    }
    assert!(stack.is_empty() && pending.is_none() && !in_metadata);
    usda
}

fn floats(raw: &str) -> Vec<f64> {
    raw.replace(['[', ']', '(', ')'], " ")
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().unwrap())
        .collect()
}

/// The saved layers of the head revision, read straight from the store's blob files by their
/// manifest digests, merged like a stage of sublayers (stronger first).
fn read_stage(fx: &Fixture, layers: &Json) -> Usda {
    let mut merged = Usda::default();
    for layer in at(layers, &["revision", "manifest", "layers"])
        .array()
        .unwrap()
    {
        let digest = text(layer, "digest");
        let path = fx.store().join("blobs").join(&digest[..2]).join(&digest);
        let source = fs::read_to_string(path).unwrap();
        let parsed = parse_usda(&source);
        merged.metadata.extend(parsed.metadata);
        for (prim, attrs) in parsed.prims {
            merged.prims.entry(prim).or_default().extend(attrs);
        }
    }
    merged
}

struct Mesh {
    points: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
}

fn mesh_of(stage: &Usda, element_path: &str) -> Mesh {
    let prim = &stage.prims[&format!("{element_path}/Geometry")];
    let flat = floats(&prim["points"]);
    let points = flat.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
    assert!(floats(&prim["faceVertexCounts"]).iter().all(|c| *c == 3.0));
    let idx: Vec<usize> = floats(&prim["faceVertexIndices"])
        .iter()
        .map(|v| *v as usize)
        .collect();
    Mesh {
        points,
        triangles: idx.chunks(3).map(|c| [c[0], c[1], c[2]]).collect(),
    }
}

/// Signed volume by the divergence theorem, computed here and nowhere in production.
fn signed_volume(mesh: &Mesh) -> f64 {
    mesh.triangles
        .iter()
        .map(|[a, b, c]| {
            let (p, q, r) = (mesh.points[*a], mesh.points[*b], mesh.points[*c]);
            let cross = [
                q[1] * r[2] - q[2] * r[1],
                q[2] * r[0] - q[0] * r[2],
                q[0] * r[1] - q[1] * r[0],
            ];
            (p[0] * cross[0] + p[1] * cross[1] + p[2] * cross[2]) / 6.0
        })
        .sum()
}

fn is_closed_two_manifold(mesh: &Mesh) -> bool {
    let mut directed = BTreeMap::new();
    for [a, b, c] in &mesh.triangles {
        for edge in [(*a, *b), (*b, *c), (*c, *a)] {
            *directed.entry(edge).or_insert(0u32) += 1;
        }
    }
    directed
        .iter()
        .all(|((a, b), n)| *n == 1 && directed.get(&(*b, *a)) == Some(&1))
}

fn bounds(mesh: &Mesh) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in &mesh.points {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

// ---- the tests -------------------------------------------------------------------------------

#[test]
fn two_plates_in_two_commits_are_independent_complete_revisions_readable_without_beamcraft() {
    let fx = Fixture::new();
    let (code, first) = fx.create(&[
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--name",
        "First plate",
    ]);
    assert_eq!(code, 0, "{first:?}");
    assert_eq!(text(&first, "outcome"), "plate_created");
    let (code, second) = fx.create(&[
        "--width",
        "800",
        "--length",
        "300",
        "--thickness",
        "12.5",
        "--translation",
        "2000,-150.5,25",
        "--name",
        "Second plate",
    ]);
    assert_eq!(code, 0, "{second:?}");

    // Two independent full identities, complete and canonical.
    let id1 = text(at(&first, &["plate"]), "element_id");
    let id2 = text(at(&second, &["plate"]), "element_id");
    assert_ne!(id1, id2);
    for id in [&id1, &id2] {
        assert_eq!(id.len(), 36);
        assert!(ElementId::parse(id).is_ok());
    }
    assert_eq!(
        text(at(&first, &["plate"]), "prim_path"),
        format!("/World/Steel/Parts/PRT_{}", id1.replace('-', ""))
    );

    // Two atomic revisions in one project: sequence, generation and base chain.
    assert_eq!(text(at(&first, &["revision"]), "sequence"), "1");
    assert_eq!(text(at(&second, &["revision"]), "sequence"), "2");
    assert_eq!(
        text(
            at(&first, &["revision", "manifest"]),
            "workspace_generation"
        ),
        "1"
    );
    assert_eq!(
        text(
            at(&second, &["revision", "manifest"]),
            "workspace_generation"
        ),
        "2"
    );
    assert_eq!(
        text(at(&first, &["revision", "manifest"]), "project_id"),
        text(at(&second, &["revision", "manifest"]), "project_id")
    );
    assert_ne!(
        text(at(&first, &["revision"]), "revision_id"),
        text(at(&second, &["revision"]), "revision_id")
    );
    assert_ne!(
        text(at(&first, &["revision"]), "manifest_digest"),
        text(at(&second, &["revision"]), "manifest_digest")
    );

    // Volume against the closed form, not production math: 1000*500*10 mm3 and 800*300*12.5.
    let v1 = 1000.0 * 500.0 * 10.0;
    let v2 = 800.0 * 300.0 * 12.5;
    assert!(close(
        num(&first, &["plate", "quantities", "volume", "mm3"]),
        v1
    ));
    assert!(close(
        num(&first, &["plate", "quantities", "volume", "m3"]),
        v1 / 1e9
    ));
    assert!(close(
        num(&second, &["plate", "quantities", "volume", "mm3"]),
        v2
    ));
    assert_eq!(
        text(at(&first, &["plate", "quantities", "mass"]), "status"),
        "unknown"
    );

    // A fresh process reopens the committed revision and sees both plates.
    let (code, inspected) = fx.inspect();
    assert_eq!(code, 0, "{inspected:?}");
    assert_eq!(text(&inspected, "outcome"), "plate_inspection");
    let plates = inspected.get("plates").unwrap().array().unwrap();
    assert_eq!(plates.len(), 2);
    let listed: BTreeSet<String> = plates.iter().map(|p| text(p, "element_id")).collect();
    assert_eq!(listed, BTreeSet::from([id1.clone(), id2.clone()]));
    assert_eq!(text(at(&inspected, &["revision"]), "sequence"), "2");
    assert_eq!(
        text(at(&inspected, &["revision"]), "manifest_digest"),
        text(at(&second, &["revision"]), "manifest_digest")
    );
    let by_id = |id: &str| plates.iter().find(|p| text(p, "element_id") == id).unwrap();
    assert!(close(
        num(by_id(&id1), &["quantities", "volume", "mm3"]),
        v1
    ));
    assert!(close(
        num(by_id(&id2), &["quantities", "volume", "mm3"]),
        v2
    ));
    // A viewer can load real data from the committed revision: pinned snapshot, two meshes.
    let snapshot = at(&inspected, &["snapshot"]);
    assert_eq!(*snapshot.get("synthetic").unwrap(), Json::Bool(false));
    assert_eq!(snapshot.get("elements").unwrap().array().unwrap().len(), 2);
    let meshes = snapshot.get("meshes").unwrap().array().unwrap();
    assert_eq!(meshes.len(), 2);
    assert!(
        meshes
            .iter()
            .all(|m| m.get("bytes").unwrap().unsigned().unwrap() == 392)
    );
    let element = snapshot
        .get("elements")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .find(|e| text(e, "id") == id2)
        .unwrap();
    assert_eq!(
        element.get("world_min_mm").unwrap().array().unwrap()[0]
            .float()
            .unwrap(),
        2000.0
    );
    assert_eq!(
        element.get("world_max_mm").unwrap().array().unwrap()[1]
            .float()
            .unwrap(),
        149.5
    );

    // `open-revision` (the U06 verified reader) agrees.
    let (code, opened) = fx.run(&["open-revision", "--store", &fx.store_arg()]);
    assert_eq!(code, 0, "{opened:?}");
    assert_eq!(text(&opened, "outcome"), "revision_verified");
    assert_eq!(
        text(&opened, "revision_id"),
        text(at(&second, &["revision"]), "revision_id")
    );

    // The independent reader: the saved text, parsed by code that knows only USDA and UsdGeom.
    let stage = read_stage(&fx, &inspected);
    assert_eq!(stage.metadata["metersPerUnit"], "0.001");
    assert_eq!(stage.metadata["upAxis"], "\"Z\"");
    assert_eq!(stage.metadata["defaultPrim"], "\"World\"");
    assert_eq!(
        stage.metadata["subLayers"],
        "[@authored.usda@, @derived.usda@]"
    );
    for (id, (w, l, t), name, translation) in [
        (&id1, (1000.0, 500.0, 10.0), "First plate", [0.0, 0.0, 0.0]),
        (
            &id2,
            (800.0, 300.0, 12.5),
            "Second plate",
            [2000.0, -150.5, 25.0],
        ),
    ] {
        let path = format!("/World/Steel/Parts/PRT_{}", id.replace('-', ""));
        let prim = &stage.prims[&path];
        assert_eq!(prim["bim:id"], format!("\"{id}\""));
        assert_eq!(prim["bim:name"], format!("\"{name}\""));
        assert_eq!(prim["bim:elementKind"], "\"plate\"");
        assert_eq!(prim["bim:geometryMode"], "\"parametric\"");
        assert_eq!(prim["steel:plate:thicknessMm"].parse::<f64>().unwrap(), t);
        assert_eq!(floats(&prim["xformOp:translate"]), translation);
        assert_eq!(
            prim["xformOpOrder"],
            "[\"xformOp:translate\", \"xformOp:orient\"]"
        );
        assert_eq!(
            floats(&prim["steel:plate:contourPointsMm"]),
            [0.0, 0.0, 0.0, w, 0.0, 0.0, w, l, 0.0, 0.0, l, 0.0]
        );
        let mesh = mesh_of(&stage, &path);
        assert_eq!(mesh.points.len(), 8);
        assert_eq!(mesh.triangles.len(), 12);
        assert!(is_closed_two_manifold(&mesh), "closed 2-manifold");
        let volume = signed_volume(&mesh);
        assert!(volume > 0.0, "outward right-handed orientation");
        assert!(close(volume, w * l * t), "{volume} vs {}", w * l * t);
        let (lo, hi) = bounds(&mesh);
        assert_eq!(lo, [0.0, 0.0, 0.0]);
        assert_eq!(hi, [w, l, t]);
        let geometry = &stage.prims[&format!("{path}/Geometry")];
        assert_eq!(geometry["orientation"], "\"rightHanded\"");
        assert_eq!(geometry["subdivisionScheme"], "\"none\"");
    }
}

#[test]
fn the_same_operation_is_replayed_and_a_changed_one_is_refused() {
    let fx = Fixture::new();
    let mut ids = IdSource::system();
    let op = ids.next_id().unwrap().to_string();
    let element = ids.next_id().unwrap().to_string();
    let common = [
        "--op-id",
        op.as_str(),
        "--new-id",
        element.as_str(),
        "--thickness",
        "10",
        "--length",
        "500",
    ];
    let mut args = common.to_vec();
    args.extend(["--width", "1000"]);
    let (code, created) = fx.create(&args);
    assert_eq!(code, 0, "{created:?}");
    // F04-POS-04: the same op id with identical content returns the recorded result.
    let (code, replay) = fx.create(&args);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(text(&replay, "outcome"), "plate_replayed");
    assert_eq!(
        text(at(&replay, &["revision"]), "revision_id"),
        text(at(&created, &["revision"]), "revision_id")
    );
    assert_eq!(text(at(&replay, &["revision"]), "sequence"), "1");
    assert_eq!(*replay.get("footprint").unwrap(), Json::Null);
    // F04-NEG-10: the same op id with a changed width.
    let mut changed = common.to_vec();
    changed.extend(["--width", "750"]);
    let (code, refused) = fx.create(&changed);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "OP_ID_REUSE");
    // F04-NEG-03: another operation may not create an element id that is already owned.
    let other = ids.next_id().unwrap().to_string();
    let (code, duplicate) = fx.create(&[
        "--op-id",
        other.as_str(),
        "--new-id",
        element.as_str(),
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
    ]);
    assert_eq!(code, 2, "{duplicate:?}");
    assert_eq!(text(&duplicate, "code"), "DUPLICATE_ID");
    // Nothing but the first revision exists, and a replay after another commit still holds.
    let (_, third) = fx.create(&["--width", "10", "--length", "10", "--thickness", "1"]);
    assert_eq!(text(at(&third, &["revision"]), "sequence"), "2");
    let (code, replay_later) = fx.create(&args);
    assert_eq!(code, 0, "{replay_later:?}");
    assert_eq!(text(&replay_later, "outcome"), "plate_replayed");
    assert_eq!(text(at(&replay_later, &["revision"]), "sequence"), "1");
    let (_, inspected) = fx.inspect();
    assert_eq!(inspected.get("plates").unwrap().array().unwrap().len(), 2);
}

#[test]
fn rejected_commands_leave_no_revision_and_report_the_f04_code() {
    let fx = Fixture::new();
    for (args, expected) in [
        (
            ["--width", "1000", "--length", "500", "--thickness", "0"],
            "INVALID_ARGUMENT",
        ), // F04-NEG-04
        (
            ["--width", "1000", "--length", "500", "--thickness", "-3"],
            "INVALID_ARGUMENT",
        ), // (negative thickness; the UpdateElement case F04-NEG-14 is not run)
        (
            [
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "0.0009",
            ],
            "INVALID_ARGUMENT",
        ), // F04-NEG-27
        (
            ["--width", "1000001", "--length", "500", "--thickness", "10"],
            "LIMIT_EXCEEDED",
        ), // F04-NEG-05
        (
            [
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "1000000.5",
            ],
            "LIMIT_EXCEEDED",
        ),
        (
            ["--width", "NaN", "--length", "500", "--thickness", "10"],
            "NON_FINITE_NUMBER",
        ),
        (
            ["--width", "1000", "--length", "inf", "--thickness", "10"],
            "NON_FINITE_NUMBER",
        ),
        (
            ["--width", "0", "--length", "500", "--thickness", "10"],
            "INVALID_ARGUMENT",
        ),
        (
            ["--width", "ten", "--length", "500", "--thickness", "10"],
            "INVALID_ARGUMENT",
        ),
    ] {
        let (code, body) = fx.create(&args);
        assert_eq!(code, 2, "{args:?} {body:?}");
        assert_eq!(text(&body, "outcome"), "rejected");
        assert_eq!(text(&body, "code"), expected, "{args:?}");
    }
    let (code, body) = fx.create(&[
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--new-id",
        "019a7bf00000",
    ]);
    assert_eq!((code, text(&body, "code")), (2, "INVALID_ID".to_owned())); // F04-NEG-01
    let (_, body) = fx.create(&[
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--new-id",
        "019a7bf0-0000-4000-8000-000000000005",
    ]);
    assert_eq!(text(&body, "code"), "INVALID_ID"); // F04-NEG-02
    let (_, body) = fx.create(&[
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--name",
        "line\nbreak",
    ]);
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    // No rejected command published anything: there is no revision at all.
    let (code, opened) = fx.run(&["open-revision", "--store", &fx.store_arg()]);
    assert_eq!(code, 2, "{opened:?}");
    assert!(fx.revisions().head().unwrap().is_none());
    let (code, body) = fx.inspect();
    assert_eq!(code, 2, "{body:?}");
    // An unknown option and a missing store are refused before anything is written.
    let (code, _) = fx.run(&["create-plate", "--store", &fx.store_arg(), "--wdith", "1"]);
    assert_eq!(code, 2);
    let (code, _) = fx.run(&["create-plate", "--width", "1"]);
    assert_eq!(code, 2);
}

#[test]
fn concurrent_creates_publish_only_complete_revisions_with_distinct_ids() {
    let fx = Fixture::new();
    let store = fx.store_arg();
    let children: Vec<_> = (0..4)
        .map(|n| {
            Command::new(env!("CARGO_BIN_EXE_beamcraft"))
                .args([
                    "create-plate",
                    "--store",
                    store.as_str(),
                    "--width",
                    "100",
                    "--length",
                    "100",
                    "--thickness",
                    &format!("{}", 5 + n),
                ])
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut created = BTreeSet::new();
    let mut losers = 0;
    for child in children {
        let output = child.wait_with_output().unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        match output.status.code().unwrap() {
            0 => {
                assert!(created.insert(text(at(&body, &["plate"]), "element_id")));
            }
            _ => {
                losers += 1;
                // A lost race is a typed rejection, never a half-written root. Windows can report
                // a sharing violation on the root switch as PERMISSION_DENIED.
                assert!(
                    [
                        "BASE_REVISION_MISMATCH",
                        "INTEGRITY_ERROR",
                        "OUTCOME_UNKNOWN",
                        "PERMISSION_DENIED"
                    ]
                    .contains(&text(&body, "code").as_str())
                        || text(&body, "code") == "BACKEND_UNAVAILABLE",
                    "{body:?}"
                );
            }
        }
    }
    assert!(!created.is_empty());
    assert_eq!(created.len() + losers, 4);
    let (code, inspected) = fx.inspect();
    assert_eq!(code, 0, "{inspected:?}");
    let listed: BTreeSet<String> = inspected
        .get("plates")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|p| text(p, "element_id"))
        .collect();
    assert_eq!(
        listed, created,
        "exactly the acknowledged plates were published"
    );
}

// ---- failure injection: a real process dies at each commit point ------------------------------

const CRASH_ENV: &str = "BEAMCRAFT_P01_CRASH_CHILD";

fn point_name(point: CommitPoint) -> &'static str {
    match point {
        CommitPoint::Checked => "checked",
        CommitPoint::IntentRecorded => "intent",
        CommitPoint::BlobsVerified => "blobs",
        CommitPoint::ManifestStored => "manifest",
        CommitPoint::RecordStaged => "staged",
        CommitPoint::Switched => "switched",
        CommitPoint::Recorded => "recorded",
    }
}

const ALL_POINTS: [CommitPoint; 7] = [
    CommitPoint::Checked,
    CommitPoint::IntentRecorded,
    CommitPoint::BlobsVerified,
    CommitPoint::ManifestStored,
    CommitPoint::RecordStaged,
    CommitPoint::Switched,
    CommitPoint::Recorded,
];

fn request(op: &str, element: &str, revision: &str, thickness: f64) -> CreateRequest {
    let mut ids = IdSource::system();
    CreateRequest {
        op_id: OpId::parse(op).unwrap(),
        revision_id: RevisionId::parse(revision).unwrap(),
        actor: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
        project: ProjectId::from_uuid(ids.next_id().unwrap()),
        workspace: WorkspaceId::from_uuid(ids.next_id().unwrap()),
        command: CreatePlate {
            new_id: ElementId::parse(element).unwrap(),
            name: "Crash plate".into(),
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [400.0, 0.0, 0.0],
                [400.0, 200.0, 0.0],
                [0.0, 200.0, 0.0],
            ],
            thickness_mm: thickness,
            placement: Placement::IDENTITY,
            material_digest: None,
            holes_mm: Vec::new(),
            cut: None,
        },
    }
}

/// Runs only inside a child spawned by `a_killed_process_...`: it creates a plate and aborts the
/// whole process (no unwinding, no destructors) when the commit reaches the chosen point.
#[test]
fn crash_child() {
    let Ok(spec) = std::env::var(CRASH_ENV) else {
        return;
    };
    let [store, point, op, element, revision]: [&str; 5] =
        spec.split('|').collect::<Vec<_>>().try_into().unwrap();
    let store = RevisionStore::new(BlobStore::new(store).unwrap());
    let mut hook = |reached: CommitPoint| -> Result<(), Injected> {
        if point_name(reached) == point {
            std::process::abort();
        }
        Ok(())
    };
    let outcome = create_plate(&store, &request(op, element, revision, 8.0), &mut hook);
    // Reached only when the point is never hit; a surviving child is a test failure.
    std::process::exit(if outcome.is_ok() { 0 } else { 3 });
}

fn spawn_crash(fx: &Fixture, point: CommitPoint, op: &str, element: &str, revision: &str) {
    let spec = format!(
        "{}|{}|{op}|{element}|{revision}",
        fx.store_arg(),
        point_name(point)
    );
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
        .env(CRASH_ENV, spec)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "the child must have been killed at {point:?}"
    );
    assert_ne!(status.code(), Some(0));
    assert_ne!(
        status.code(),
        Some(3),
        "the commit failed instead of dying at {point:?}"
    );
}

#[test]
fn a_killed_process_leaves_the_old_or_the_new_complete_revision_at_every_commit_point() {
    for with_base in [false, true] {
        for point in ALL_POINTS {
            let fx = Fixture::new();
            let mut ids = IdSource::system();
            let mut next = || ids.next_id().unwrap().to_string();
            if with_base {
                let (code, base) =
                    fx.create(&["--width", "100", "--length", "100", "--thickness", "5"]);
                assert_eq!(code, 0, "{base:?}");
            }
            let (op, element, revision) = (next(), next(), next());
            spawn_crash(&fx, point, &op, &element, &revision);

            let switched = matches!(point, CommitPoint::Switched | CommitPoint::Recorded);
            let old_count = usize::from(with_base);
            let new_count = old_count + 1;
            let expected_sequence = if switched { new_count } else { old_count };
            let label = format!("{point:?} base={with_base}");

            // Before any recovery a fresh process opens one complete revision (or none).
            let (code, inspected) = fx.inspect();
            if expected_sequence == 0 {
                assert_eq!(code, 2, "{label}: {inspected:?}");
                assert_eq!(text(&inspected, "code"), "NOT_FOUND", "{label}");
            } else {
                assert_eq!(code, 0, "{label}: {inspected:?}");
                assert_eq!(
                    inspected.get("plates").unwrap().array().unwrap().len(),
                    expected_sequence,
                    "{label}"
                );
                assert_eq!(
                    text(at(&inspected, &["revision"]), "sequence"),
                    expected_sequence.to_string(),
                    "{label}"
                );
            }
            // The ledger claims no more than is proven.
            let (_, outcome) = fx.run(&[
                "inspect-revision-outcome",
                "--store",
                &fx.store_arg(),
                "--op-id",
                &op,
            ]);
            let name = text(&outcome, "outcome");
            if switched {
                assert_eq!(name, "outcome_committed", "{label}");
            } else if point == CommitPoint::Checked {
                assert!(
                    ["outcome_not_recorded", "outcome_unknown"].contains(&name.as_str()),
                    "{label} {name}"
                );
            } else {
                assert_eq!(name, "outcome_unknown", "{label}");
            }
            // Recovery is idempotent and leaves the same complete revision.
            for _ in 0..2 {
                let (code, recovery) = fx.run(&[
                    "recover-revision",
                    "--store",
                    &fx.store_arg(),
                    "--min-age-secs",
                    "0",
                ]);
                assert_eq!(code, 0, "{label}: {recovery:?}");
            }
            // Retrying the same logical command yields exactly one plate: a replay if the root
            // had switched, one creation otherwise.
            let (code, retry) = fx.create(&[
                "--op-id",
                &op,
                "--new-id",
                &element,
                "--revision-id",
                &next(),
                "--width",
                "400",
                "--length",
                "200",
                "--thickness",
                "8",
                "--name",
                "Crash plate",
            ]);
            assert_eq!(code, 0, "{label}: {retry:?}");
            assert_eq!(
                text(&retry, "outcome"),
                if switched {
                    "plate_replayed"
                } else {
                    "plate_created"
                },
                "{label}"
            );
            let (code, final_view) = fx.inspect();
            assert_eq!(code, 0, "{label}: {final_view:?}");
            let plates = final_view.get("plates").unwrap().array().unwrap();
            assert_eq!(plates.len(), new_count, "{label}");
            let ids: BTreeSet<String> = plates.iter().map(|p| text(p, "element_id")).collect();
            assert!(ids.contains(&element), "{label}");
            let crashed = plates
                .iter()
                .find(|p| text(p, "element_id") == element)
                .unwrap();
            assert!(close(
                num(crashed, &["quantities", "volume", "mm3"]),
                400.0 * 200.0 * 8.0
            ));
        }
    }
}

#[test]
fn a_library_error_is_typed_and_a_failed_commit_leaves_the_old_revision() {
    let fx = Fixture::new();
    let store = fx.revisions();
    let mut ids = IdSource::system();
    let mut next = || ids.next_id().unwrap().to_string();
    let first = request(&next(), &next(), &next(), 8.0);
    create_plate(&store, &first, &mut NoFault).unwrap();
    let before = inspect(&store).unwrap().revision;
    // A hook failure before the switch aborts the commit with the root unchanged.
    let second = CreateRequest {
        op_id: OpId::parse(&next()).unwrap(),
        revision_id: RevisionId::parse(&next()).unwrap(),
        command: CreatePlate {
            new_id: ElementId::parse(&next()).unwrap(),
            ..first.command.clone()
        },
        ..first.clone()
    };
    let mut fail = |point: CommitPoint| {
        if point == CommitPoint::RecordStaged {
            Err(Injected)
        } else {
            Ok(())
        }
    };
    let error: PlateError = create_plate(&store, &second, &mut fail).unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(inspect(&store).unwrap().revision, before);
    // The same operation then succeeds exactly once.
    let done = create_plate(&store, &second, &mut NoFault).unwrap();
    assert!(!done.replayed);
    assert_eq!(inspect(&store).unwrap().plates.len(), 2);
}

#[test]
fn a_plate_above_one_cubic_metre_is_within_the_stage_budget() {
    // 1e9 mm3 is exactly one cubic metre: the bounded read used to refuse any stored double
    // above it, including the plate's own volume in mm3, so this very plate was impossible.
    let fx = Fixture::new();
    let (code, reply) = fx.create(&[
        "--width",
        "5000",
        "--length",
        "5000",
        "--thickness",
        "41",
        "--name",
        "Slab",
    ]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "outcome"), "plate_created");
    assert!(close(
        num(at(&reply, &["plate", "quantities", "volume"]), &["mm3"]),
        5_000.0 * 5_000.0 * 41.0
    ));
    // A fresh process still opens the revision and re-reads the quantities.
    let (code, inspected) = fx.inspect();
    assert_eq!(code, 0, "{inspected:?}");
    let plates = inspected.get("plates").unwrap().array().unwrap();
    assert_eq!(plates.len(), 1);
    assert!(close(
        num(
            at(plates.first().unwrap(), &["quantities", "volume"]),
            &["mm3"]
        ),
        5_000.0 * 5_000.0 * 41.0
    ));
}
