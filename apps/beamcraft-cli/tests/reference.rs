// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic sources and plates, MPL-2.0. The "foreign" models below are
// invented text written by this test; no third-party model, sample or asset is used.

//! R01 end to end: a foreign USD file is attached read-only through the real `beamcraft` binary.
//! The foreign bytes are stored under their SHA-256 and survive every later save unchanged; the
//! wrapper (placement, units correction) is stored apart from them; a network path is refused
//! before any I/O; and the API and CLI refuse every write to the source or to a foreign prim.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_engine::{ElementKind, Limits as ViewLimits, ViewSnapshot};
use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::digest::Sha256;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// An invented architect model: metres, Y up, a block with a body mesh and a declared note.
const SITE: &str = r#"#usda 1.0
(
    defaultPrim = "Site"
    metersPerUnit = 1.0
    upAxis = "Y"
)
def Xform "Site"
{
    def Xform "Block"
    {
        custom string note = "declared by the source"
        def Mesh "Body"
        {
            uniform token subdivisionScheme = "none"
            point3f[] points = [(0, 0, 0), (2, 0, 0), (2, 1, 0), (0, 1, 0), (0, 0, 3), (2, 0, 3), (2, 1, 3), (0, 1, 3)]
            int[] faceVertexCounts = [4, 4, 4, 4, 4, 4]
            int[] faceVertexIndices = [0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7]
        }
    }
}
"#;

#[test]
fn an_attached_reference_enters_the_pinned_view_scene() {
    let f = Fixture::new();
    let source = f.source("site.usda", SITE);
    let id = "019a7bf0-0000-7000-8000-000000000137";
    f.ok(
        "reference-attach",
        &[
            "--source",
            &source,
            "--new-id",
            id,
            "--translation",
            "100,200,300",
        ],
    );
    let snapshot = view(&f);
    assert_eq!(
        snapshot.len(),
        1,
        "the attached wrapper must reach the viewport"
    );
    let element = &snapshot.elements()[0];
    assert_eq!(element.id.to_string(), id);
    assert_eq!(element.kind, ElementKind::Reference);
    assert_eq!(element.placement_mm, [100.0, 200.0, 300.0]);
    assert_eq!(element.full.world_bounds.min, [100.0, 200.0, 300.0]);
    assert_eq!(element.full.world_bounds.max, [102.0, 201.0, 303.0]);
    assert_eq!(snapshot.meshes()[0].triangle_count(), 12);
    assert_eq!(snapshot.pin().generation(), 1);
    f.ok(
        "reference-correct-units",
        &["--id", id, "--meters-per-unit", "1", "--up-axis", "Y"],
    );
    let corrected = view(&f);
    assert_eq!(
        corrected.elements()[0].full.world_bounds.min,
        [100.0, -2800.0, 300.0]
    );
    assert_eq!(
        corrected.elements()[0].full.world_bounds.max,
        [2100.0, 200.0, 1300.0]
    );
    assert_ne!(snapshot.meshes(), corrected.meshes());
    f.ok(
        "reference-move",
        &["--id", id, "--translation", "200,300,400"],
    );
    let moved = view(&f);
    assert_eq!(moved.meshes(), corrected.meshes());
    assert_eq!(
        moved.elements()[0].full.world_bounds.min,
        [200.0, -2700.0, 400.0]
    );
    assert_ne!(moved.token(), corrected.token());
    assert_eq!(moved.pin().generation(), 3);
    assert_eq!(std::fs::read(source).unwrap(), SITE.as_bytes());
}

fn view(f: &Fixture) -> ViewSnapshot {
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(["view-export", "--store", &f.store(), "--out", "-"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    ViewSnapshot::from_bundle(&output.stdout, &ViewLimits::default()).unwrap()
}

#[test]
fn source_translations_visibility_multiple_surfaces_and_shared_sources_reach_the_scene() {
    let f = Fixture::new();
    let bytes = r#"#usda 1.0
(defaultPrim = "Site")
def Xform "Site" {
    double3 xformOp:translate = (10, 20, 30)
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def Cube "Translated" {
        double size = 2
        double3 xformOp:translate:offset = (4, 5, 6)
        uniform token[] xformOpOrder = ["xformOp:translate:offset"]
    }
    def Cube "DefaultSize" {
        double xformOp:rotateZ = 45
    }
    def Mesh "Triangle" {
        point3f[] points = [(0, 0, 0), (2, 0, 0), (0, 2, 0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        uniform token subdivisionScheme = "none"
        uniform token orientation = "leftHanded"
    }
    def Xform "Hidden" {
        token visibility = "invisible"
        double xformOp:rotateZ = 30
        uniform token[] xformOpOrder = ["xformOp:rotateZ"]
        def Cube "NotDrawn" {
            double size = 10000
            token visibility = "inherited"
        }
    }
}
"#;
    let source = f.source("translated.usda", bytes);
    let ids = [
        "019a7bf0-0000-7000-8000-000000000137",
        "019a7bf0-0000-7000-8000-000000000138",
    ];
    for id in ids {
        f.ok(
            "reference-attach",
            &[
                "--source",
                &source,
                "--new-id",
                id,
                "--translation",
                "100,200,300",
            ],
        );
    }
    f.ok(
        "create-plate",
        &["--width", "3", "--length", "4", "--thickness", "5"],
    );
    let snapshot = view(&f);
    assert_eq!(
        snapshot.len(),
        3,
        "two references coexist with native steel"
    );
    assert_eq!(
        snapshot.meshes().len(),
        2,
        "identical foreign representations share one mesh"
    );
    for id in ids {
        let element = snapshot
            .element(&beamcraft_engine::ElementId::parse(id).unwrap())
            .unwrap();
        assert_eq!(element.kind, ElementKind::Reference);
        assert_eq!(element.full.world_bounds.min, [109.0, 219.0, 329.0]);
        assert_eq!(element.full.world_bounds.max, [115.0, 226.0, 337.0]);
        let mesh = snapshot.mesh_at(element.full.mesh_slot).unwrap();
        assert_eq!(mesh.vertex_count(), 19);
        assert_eq!(
            mesh.triangle_count(),
            25,
            "the invisible subtree contributes nothing"
        );
    }
    assert_eq!(std::fs::read(source).unwrap(), bytes.as_bytes());
}

#[test]
fn unsupported_views_are_explicit_refusals_without_breaking_r01_or_creating_a_file() {
    for (kind, properties, expected) in [
        (
            "Cube",
            "double xformOp:rotateZ = 30\nuniform token[] xformOpOrder = [\"xformOp:rotateZ\"]",
            "UNSUPPORTED_GEOMETRY",
        ),
        (
            "Cube",
            "double3 xformOp:scale = (2, 2, 2)\nuniform token[] xformOpOrder = [\"xformOp:scale\"]",
            "UNSUPPORTED_GEOMETRY",
        ),
        (
            "Cube",
            "uniform token[] xformOpOrder = [\"!resetXformStack!\"]",
            "UNSUPPORTED_GEOMETRY",
        ),
        ("Sphere", "double radius = 3", "UNSUPPORTED_GEOMETRY"),
        (
            "Mesh",
            "uniform token subdivisionScheme = \"none\"\npoint3f[] points = [(0, 0, 0), (2, 0, 0), (0, 2, 0)]\nint[] faceVertexCounts = [3]\nint[] faceVertexIndices = [-1, 1, 2]",
            "INVALID_ARGUMENT",
        ),
        (
            "Mesh",
            "point3f[] points = [(0, 0, 0), (2, 0, 0), (0, 2, 0)]\nint[] faceVertexCounts = [3]\nint[] faceVertexIndices = [0, 1, 2]",
            "UNSUPPORTED_GEOMETRY",
        ),
    ] {
        let f = Fixture::new();
        let source = f.source(
            "unsupported.usda",
            format!(
                "#usda 1.0\n(defaultPrim = \"Site\")\ndef {kind} \"Site\" {{\n{properties}\n}}\n"
            ),
        );
        f.ok("reference-attach", &["--source", &source]);
        f.ok("reference-inspect", &[]);
        f.ok("plate-inspect", &[]);
        let out = f.root.join("refused.bundle");
        assert_eq!(
            f.rejected("view-export", &["--out", out.to_str().unwrap()]),
            expected
        );
        assert!(!out.exists());
    }
    let f = Fixture::new();
    let source = f.source("site.usda", SITE);
    let id = "019a7bf0-0000-7000-8000-000000000137";
    f.ok(
        "reference-attach",
        &[
            "--source",
            &source,
            "--new-id",
            id,
            "--rotation-z-deg",
            "90",
        ],
    );
    assert_eq!(
        f.rejected("view-export", &["--out", "-"]),
        "UNSUPPORTED_GEOMETRY"
    );
    f.ok("reference-inspect", &[]);
    f.ok("plate-inspect", &[]);
    f.ok("reference-move", &["--id", id, "--translation", "0,0,0"]);
    assert_eq!(
        view(&f).len(),
        1,
        "an identity placement restores the supported view"
    );
}

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("r01-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        fs::create_dir(root.join("sources")).unwrap();
        Self { root }
    }
    fn store(&self) -> String {
        self.root.join("store").to_str().unwrap().to_owned()
    }
    /// Writes a source file and returns its path text.
    fn source(&self, name: &str, bytes: impl AsRef<[u8]>) -> String {
        let path = self.root.join("sources").join(name);
        fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }
    fn run_in(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        let mut args = vec![command, "--store", store];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.run_in(&self.store(), command, extra)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn ok_in(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run_in(store, command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        text(&body, "code")
    }
    fn attach(&self, source: &str, extra: &[&str]) -> Json {
        let mut args = vec!["--source", source];
        args.extend_from_slice(extra);
        self.ok("reference-attach", &args)
    }
    fn inspect(&self) -> Json {
        self.ok("reference-inspect", &[])
    }
    /// Every file below the store root with its size, in path order.
    fn listing(&self) -> BTreeMap<String, u64> {
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, u64>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else {
                    let name = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/");
                    out.insert(name, entry.metadata().unwrap().len());
                }
            }
        }
        let root = self.root.join("store");
        let mut out = BTreeMap::new();
        walk(&root, &root, &mut out);
        out
    }
    /// Commits a head-based revision with changed text in the owned `authored.usda` and no
    /// content verification, the way a layer edited outside BeamCraft would arrive.
    fn tamper_authored(&self, edit: impl Fn(String) -> String) {
        let store = RevisionStore::new(BlobStore::new(self.root.join("store")).unwrap());
        let head = store.open().unwrap();
        let content = head.manifest().content();
        let mut ids = IdSource::system();
        let mut entries = Vec::new();
        for entry in content.entries() {
            let mut bytes = head.layer_bytes(entry.name()).unwrap();
            if entry.name() == "authored.usda" {
                bytes = edit(String::from_utf8(bytes).unwrap()).into_bytes();
                store.blobs().publish(&bytes).unwrap();
            }
            entries.push(LayerEntry::from_bytes(entry.name(), entry.role(), &bytes).unwrap());
        }
        let draft = RevisionDraft {
            project: content.project().clone(),
            workspace: content.workspace().clone(),
            revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            workspace_generation: content.workspace_generation() + 1,
            schema: content.schema(),
            root_layer: content.root_layer().to_owned(),
            entries,
        };
        store
            .commit_with(
                &draft,
                &ExpectedBase::Revision(head.info().revision().clone()),
                None,
                &mut NoFault,
            )
            .unwrap();
    }
    fn blob_path(&self, digest: &str) -> PathBuf {
        self.root
            .join("store")
            .join("blobs")
            .join(&digest[..2])
            .join(digest)
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
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).unwrap().to_string()
}
fn reference_id(body: &Json) -> String {
    text(at(body, &["reference"]), "id")
}
/// The `(name, digest, role)` of every layer of the revision in a command result.
fn layers(body: &Json) -> Vec<(String, String, String)> {
    at(body, &["revision", "manifest", "layers"])
        .array()
        .unwrap()
        .iter()
        .map(|l| (text(l, "name"), text(l, "digest"), text(l, "role")))
        .collect()
}
fn external(body: &Json) -> Vec<(String, String)> {
    layers(body)
        .into_iter()
        .filter(|(_, _, role)| role == "external")
        .map(|(name, digest, _)| (name, digest))
        .collect()
}
fn number(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

#[test]
fn attach_stores_the_foreign_bytes_unchanged_and_the_original_file_is_only_read() {
    let f = Fixture::new();
    let source = f.source("site.usda", SITE);
    let before = fs::metadata(&source).unwrap().modified().unwrap();
    let body = f.attach(&source, &["--translation", "1000,2000,0", "--name", "Site"]);
    let digest = sha(SITE.as_bytes());
    assert_eq!(text(&body, "outcome"), "reference_attached");
    let reference = at(&body, &["reference"]);
    assert_eq!(text(reference, "name"), "Site");
    assert_eq!(text(at(reference, &["source"]), "digest"), digest);
    assert_eq!(text(at(reference, &["source"]), "format"), "usda");
    assert_eq!(text(at(reference, &["source"]), "default_prim"), "Site");
    assert_eq!(text(at(reference, &["source"]), "label"), "site.usda");
    assert_eq!(
        at(reference, &["source"])
            .get("version")
            .unwrap()
            .unsigned()
            .unwrap(),
        1
    );
    assert_eq!(
        number(reference, &["source", "declared_meters_per_unit"]),
        1.0
    );
    assert_eq!(text(at(reference, &["source"]), "declared_up_axis"), "Y");
    // The source is taken as stored until the owner decides: the status says so.
    assert_eq!(text(reference, "units_status"), "needs_correction");
    assert!(matches!(at(reference, &["units"]), Json::Null));
    assert_eq!(
        at(reference, &["placement", "translation_mm"])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [1000.0, 2000.0, 0.0]
    );
    // The foreign layer is an external manifest entry with exactly the file's digest, and its
    // blob is the file's bytes.
    let external = external(&body);
    assert_eq!(external.len(), 1);
    assert_eq!(external[0].1, digest);
    assert_eq!(external[0].0, format!("external/{digest}.usda"));
    assert_eq!(fs::read(f.blob_path(&digest)).unwrap(), SITE.as_bytes());
    let blob_check = f.ok("blob-verify", &["--digest", &digest]);
    assert_eq!(text(&blob_check, "outcome"), "blob_verified");
    // The user's file was only read.
    assert_eq!(fs::read(&source).unwrap(), SITE.as_bytes());
    assert_eq!(fs::metadata(&source).unwrap().modified().unwrap(), before);
    // The path of the original is not recorded anywhere in the store.
    for (name, _) in f.listing() {
        let bytes = fs::read(f.root.join("store").join(&name)).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("sources"),
            "{name} mentions the source path"
        );
    }
    // The wrapper carries the foreign prims, listed in the source layer's own namespace.
    let paths: Vec<_> = at(reference, &["foreign_prims"])
        .array()
        .unwrap()
        .iter()
        .map(|p| text(p, "source_path"))
        .collect();
    assert_eq!(paths, ["/Site", "/Site/Block", "/Site/Block/Body"]);
    // A reopen in a fresh process reads the same thing, with the layer re-verified.
    let again = f.inspect();
    assert_eq!(at(&again, &["references"]).array().unwrap().len(), 1);
    let foreign = &at(&again, &["foreign_layers"]).array().unwrap()[0];
    assert_eq!(text(foreign, "digest"), digest);
    assert!(foreign.get("verified").unwrap().boolean().unwrap());
    // Plates and wrappers coexist; the wrapper is not an unknown element.
    let plates = f.ok("plate-inspect", &[]);
    assert_eq!(
        plates
            .get("other_element_count")
            .unwrap()
            .unsigned()
            .unwrap(),
        0
    );
}

#[test]
fn every_later_save_carries_the_foreign_layer_unchanged() {
    let f = Fixture::new();
    let source = f.source("site.usda", SITE);
    let attached = f.attach(&source, &[]);
    let digest = sha(SITE.as_bytes());
    let name = format!("external/{digest}.usda");
    let mut steps = vec![("attach", attached)];
    steps.push((
        "create-plate",
        f.ok(
            "create-plate",
            &["--width", "1000", "--length", "500", "--thickness", "10"],
        ),
    ));
    let plate = text(at(&steps[1].1, &["plate"]), "element_id");
    steps.push((
        "update-element",
        f.ok("update-element", &["--id", &plate, "--name", "Renamed"]),
    ));
    steps.push((
        "copy-element",
        f.ok("copy-element", &["--id", &plate, "--offset", "5,0,0"]),
    ));
    steps.push((
        "define-material",
        f.ok("define-material", &["--density", "7850"]),
    ));
    let id = reference_id(&steps[0].1);
    steps.push((
        "reference-move",
        f.ok("reference-move", &["--id", &id, "--translation", "5,6,7"]),
    ));
    steps.push(("delete-element", f.ok("delete-element", &["--id", &plate])));
    let mut sequences = Vec::new();
    for (what, body) in &steps {
        let ext = external(body);
        assert_eq!(ext, vec![(name.clone(), digest.clone())], "after {what}");
        sequences.push(text(at(body, &["revision"]), "sequence"));
    }
    sequences.dedup();
    assert_eq!(
        sequences.len(),
        steps.len(),
        "every step is its own revision"
    );
    assert_eq!(fs::read(f.blob_path(&digest)).unwrap(), SITE.as_bytes());
    // The wrapper survives plate edits and deletes untouched.
    let inspected = f.inspect();
    let wrapper = &at(&inspected, &["references"]).array().unwrap()[0];
    assert_eq!(text(wrapper, "id"), id);
    assert_eq!(text(wrapper, "name"), "site.usda");
    assert_eq!(
        at(wrapper, &["placement", "translation_mm"])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [5.0, 6.0, 7.0]
    );
}

#[test]
fn moving_and_correcting_change_only_the_wrapper_and_the_correction_is_explicit() {
    let f = Fixture::new();
    let source = f.source("site.usda", SITE);
    let attached = f.attach(&source, &[]);
    let id = reference_id(&attached);
    let digest = sha(SITE.as_bytes());
    // A move: the owned authored layer changes, the derived layer and the foreign layer do not.
    let moved = f.ok(
        "reference-move",
        &[
            "--id",
            &id,
            "--translation",
            "10,20,30",
            "--rotation-z-deg",
            "90",
        ],
    );
    assert_eq!(text(&moved, "outcome"), "reference_moved");
    let changed: Vec<_> = at(&moved, &["changed_layers"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect();
    assert_eq!(changed, ["authored.usda"]);
    assert_eq!(
        external(&moved),
        vec![(format!("external/{digest}.usda"), digest.clone())]
    );
    let placement = at(&moved, &["reference", "placement"]);
    let q: Vec<f64> = at(placement, &["orientation_xyzw"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    assert!((q[2] - h).abs() < 1e-12 && (q[3] - h).abs() < 1e-12 && q[0] == 0.0 && q[1] == 0.0);
    // The declared units are kept as declared; nothing was corrected by moving.
    assert_eq!(
        text(at(&moved, &["reference"]), "units_status"),
        "needs_correction"
    );
    // The explicit correction: metres, Y up. The status becomes corrected and the source pin is
    // unchanged.
    let corrected = f.ok(
        "reference-correct-units",
        &["--id", &id, "--meters-per-unit", "1", "--up-axis", "Y"],
    );
    assert_eq!(text(&corrected, "outcome"), "reference_units_corrected");
    let units = at(&corrected, &["reference", "units"]);
    assert_eq!(number(units, &["scale"]), 1000.0);
    assert_eq!(number(units, &["rotate_x_degrees"]), 90.0);
    assert_eq!(text(units, "up_axis"), "Y");
    assert_eq!(
        text(at(&corrected, &["reference"]), "units_status"),
        "corrected"
    );
    assert_eq!(
        text(at(&corrected, &["reference", "source"]), "digest"),
        digest
    );
    assert_eq!(
        external(&corrected),
        vec![(format!("external/{digest}.usda"), digest.clone())]
    );
    // The move survived the correction.
    assert_eq!(
        at(&corrected, &["reference", "placement", "translation_mm"])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [10.0, 20.0, 30.0]
    );
    // A correction that contradicts the declaration is honoured and flagged.
    let differs = f.ok(
        "reference-correct-units",
        &["--id", &id, "--meters-per-unit", "0.01", "--up-axis", "Z"],
    );
    assert_eq!(
        text(at(&differs, &["reference"]), "units_status"),
        "corrected_differs_from_declared"
    );
    // Clearing removes the interpretation and nothing else.
    let cleared = f.ok("reference-correct-units", &["--id", &id, "--clear", "true"]);
    assert!(matches!(at(&cleared, &["reference", "units"]), Json::Null));
    assert_eq!(
        text(at(&cleared, &["reference"]), "units_status"),
        "needs_correction"
    );
    // Bad interpretations are rejected with nothing written.
    let listing = f.listing();
    for (extra, code) in [
        (
            vec!["--meters-per-unit", "0", "--up-axis", "Y"],
            "INVALID_ARGUMENT",
        ),
        (
            vec!["--meters-per-unit", "-1", "--up-axis", "Y"],
            "INVALID_ARGUMENT",
        ),
        (
            vec!["--meters-per-unit", "1e9", "--up-axis", "Y"],
            "LIMIT_EXCEEDED",
        ),
        (
            vec!["--meters-per-unit", "1", "--up-axis", "X"],
            "INVALID_ARGUMENT",
        ),
        (vec!["--meters-per-unit", "1"], "INVALID_ARGUMENT"),
        (
            vec!["--clear", "true", "--up-axis", "Y"],
            "INVALID_ARGUMENT",
        ),
        (vec!["--clear", "false"], "INVALID_ARGUMENT"),
    ] {
        let mut args = vec!["--id", id.as_str()];
        args.extend(extra.iter().copied());
        assert_eq!(
            f.rejected("reference-correct-units", &args),
            code,
            "{args:?}"
        );
    }
    assert_eq!(f.listing(), listing, "rejections write nothing");
    // Unknown wrapper and a non-wrapper.
    let plate = text(
        at(
            &f.ok(
                "create-plate",
                &["--width", "1000", "--length", "500", "--thickness", "10"],
            ),
            &["plate"],
        ),
        "element_id",
    );
    for target in [plate.as_str(), "019a7bf0-0000-7000-8000-0000000000ff"] {
        assert_eq!(
            f.rejected(
                "reference-move",
                &["--id", target, "--translation", "1,1,1"]
            ),
            "NOT_FOUND"
        );
        assert_eq!(
            f.rejected(
                "reference-correct-units",
                &["--id", target, "--clear", "true"]
            ),
            "NOT_FOUND"
        );
    }
    // update-element and delete-element do not touch a wrapper either.
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--name", "x"]),
        "NOT_FOUND"
    );
    assert_eq!(f.rejected("delete-element", &["--id", &id]), "NOT_FOUND");
}

#[test]
fn replay_and_reuse_of_an_operation_id() {
    let f = Fixture::new();
    let source = f.source("site.usda", SITE);
    let op = "019a7bf0-0000-7000-8000-0000000000e1";
    let new_id = "019a7bf0-0000-7000-8000-0000000000e2";
    let first = f.attach(&source, &["--op-id", op, "--new-id", new_id]);
    assert_eq!(text(&first, "outcome"), "reference_attached");
    let listing = f.listing();
    let again = f.attach(&source, &["--op-id", op, "--new-id", new_id]);
    assert_eq!(text(&again, "outcome"), "reference_attach_replayed");
    assert_eq!(f.listing(), listing, "a replay writes nothing");
    // The same operation with other content is refused.
    let other = f.source(
        "other.usda",
        SITE.replace("declared by the source", "other note"),
    );
    let (code, body) = f.run(
        "reference-attach",
        &["--source", &other, "--op-id", op, "--new-id", new_id],
    );
    assert_eq!((code, text(&body, "code")), (2, "OP_ID_REUSE".to_owned()));
    // A used identity is a duplicate.
    let (code, body) = f.run(
        "reference-attach",
        &["--source", &other, "--new-id", new_id],
    );
    assert_eq!((code, text(&body, "code")), (2, "DUPLICATE_ID".to_owned()));
    // A move replays too.
    let move_op = "019a7bf0-0000-7000-8000-0000000000e3";
    let args = ["--id", new_id, "--translation", "1,2,3", "--op-id", move_op];
    assert_eq!(
        text(&f.ok("reference-move", &args), "outcome"),
        "reference_moved"
    );
    assert_eq!(
        text(&f.ok("reference-move", &args), "outcome"),
        "reference_moved_replayed"
    );
    assert_eq!(
        f.rejected(
            "reference-move",
            &["--id", new_id, "--translation", "9,9,9", "--op-id", move_op]
        ),
        "OP_ID_REUSE"
    );
}

#[test]
fn the_same_bytes_attached_twice_share_one_stored_layer() {
    let f = Fixture::new();
    let first = f.source("site-a.usda", SITE);
    let second = f.source("site-b.usda", SITE);
    let a = f.attach(&first, &["--translation", "0,0,0"]);
    let b = f.attach(&second, &["--translation", "5000,0,0"]);
    assert!(!a.get("source_reused").unwrap().boolean().unwrap());
    assert!(b.get("source_reused").unwrap().boolean().unwrap());
    assert_eq!(external(&b).len(), 1);
    let digest = sha(SITE.as_bytes());
    let blobs: Vec<_> = f
        .listing()
        .into_keys()
        .filter(|name| name.contains(&digest))
        .collect();
    assert_eq!(blobs.len(), 1, "one blob for the shared source");
    let inspected = f.inspect();
    let wrappers = at(&inspected, &["references"]).array().unwrap();
    assert_eq!(wrappers.len(), 2);
    assert_ne!(text(&wrappers[0], "id"), text(&wrappers[1], "id"));
    assert!(
        wrappers
            .iter()
            .all(|w| w.get("foreign_prim_count").unwrap().unsigned().unwrap() == 3)
    );
    // Correcting one wrapper leaves the other alone.
    let id = reference_id(&b);
    f.ok(
        "reference-correct-units",
        &["--id", &id, "--meters-per-unit", "1", "--up-axis", "Y"],
    );
    let inspected = f.inspect();
    let corrected = at(&inspected, &["references"])
        .array()
        .unwrap()
        .iter()
        .filter(|w| !matches!(w.get("units").unwrap(), Json::Null))
        .count();
    assert_eq!(corrected, 1);
}

#[test]
fn every_write_to_the_source_or_a_foreign_prim_is_refused_and_writes_nothing() {
    let f = Fixture::new();
    let source = f.source("site.usda", SITE);
    let attached = f.attach(&source, &[]);
    let id = reference_id(&attached);
    let digest = sha(SITE.as_bytes());
    let listing = f.listing();
    let head = f.inspect();
    let manifest = text(at(&head, &["revision"]), "manifest_digest");
    let attempts: Vec<(Vec<&str>, &str)> = vec![
        // The stored source layer.
        (vec!["--target", "source"], "FOREIGN_READ_ONLY"),
        // Foreign prims, with and without an attribute, existing or not.
        (vec!["--prim", "/Site"], "FOREIGN_READ_ONLY"),
        (vec!["--prim", "/Site/Block"], "FOREIGN_READ_ONLY"),
        (
            vec!["--prim", "/Site/Block", "--attribute", "note"],
            "FOREIGN_READ_ONLY",
        ),
        (
            vec!["--prim", "/Site/Block/Body", "--attribute", "visibility"],
            "FOREIGN_READ_ONLY",
        ),
        (vec!["--prim", "/Site/NewChild"], "FOREIGN_READ_ONLY"),
        // Not part of this source.
        (vec!["--prim", "/Elsewhere"], "NOT_FOUND"),
        (vec!["--prim", "/SiteX"], "NOT_FOUND"),
        // Malformed.
        (vec!["--prim", "Site/Block"], "INVALID_ARGUMENT"),
        (vec!["--prim", "/Site/../Block"], "INVALID_ARGUMENT"),
        (
            vec!["--prim", "/Site", "--attribute", "a b"],
            "INVALID_ARGUMENT",
        ),
        (
            vec!["--target", "source", "--attribute", "note"],
            "INVALID_ARGUMENT",
        ),
        (
            vec!["--target", "source", "--prim", "/Site"],
            "INVALID_ARGUMENT",
        ),
        (vec!["--target", "layer"], "INVALID_ARGUMENT"),
        (vec![], "INVALID_ARGUMENT"),
    ];
    for (extra, code) in attempts {
        let mut args = vec!["--id", id.as_str()];
        args.extend(extra.iter().copied());
        let (exit, body) = f.run("reference-write", &args);
        assert_eq!(exit, 2, "{args:?}: {body:?}");
        assert_eq!(text(&body, "code"), code, "{args:?}");
        assert_eq!(text(&body, "outcome"), "rejected");
    }
    // An unknown wrapper and a store without one.
    assert_eq!(
        f.rejected(
            "reference-write",
            &[
                "--id",
                "019a7bf0-0000-7000-8000-0000000000ff",
                "--target",
                "source"
            ]
        ),
        "NOT_FOUND"
    );
    // Nothing changed: same files, same head, same foreign bytes, same user file.
    assert_eq!(f.listing(), listing);
    let after = f.inspect();
    assert_eq!(text(at(&after, &["revision"]), "manifest_digest"), manifest);
    assert_eq!(fs::read(f.blob_path(&digest)).unwrap(), SITE.as_bytes());
    assert_eq!(fs::read(&source).unwrap(), SITE.as_bytes());
}

#[test]
fn network_urls_and_device_paths_are_refused_before_any_io() {
    let f = Fixture::new();
    // The store root does not even exist: a refusal that comes from the path text alone never
    // reaches it, so the answer is the same and nothing is created.
    let missing = f.root.join("no-such-store");
    let missing = missing.to_str().unwrap().to_owned();
    for url in [
        "http://example.invalid/model.usd",
        "https://example.invalid/model.usd",
        "HTTPS://EXAMPLE.INVALID/MODEL.USD",
        "ftp://example.invalid/model.usd",
        "file:///etc/model.usd",
        "file://server/share/model.usd",
        "s3://bucket/model.usd",
        "omniverse://server/Projects/model.usd",
        "anon:0x1:layer.usda",
        "\\\\server\\share\\model.usd",
        "//server/share/model.usd",
    ] {
        let (code, body) = f.run_in(&missing, "reference-attach", &["--source", url]);
        assert_eq!(code, 2, "{url}: {body:?}");
        assert_eq!(text(&body, "code"), "PERMISSION_DENIED", "{url}");
    }
    assert!(!f.root.join("no-such-store").exists());
    // The same on a real, empty store: still nothing written.
    let (code, body) = f.run(
        "reference-attach",
        &["--source", "https://example.invalid/m.usd"],
    );
    assert_eq!(
        (code, text(&body, "code")),
        (2, "PERMISSION_DENIED".to_owned())
    );
    assert!(f.listing().is_empty());
    // A local file that itself names a network asset is refused as soon as it is examined, before
    // anything is resolved or stored.
    for text_body in [
        "#usda 1.0\n(defaultPrim = \"A\")\ndef \"A\" (references = @https://example.invalid/x.usd@</X>)\n{\n}\n",
        "#usda 1.0\n(defaultPrim = \"A\")\ndef \"A\" (payload = @http://example.invalid/x.usd@</X>)\n{\n}\n",
        "#usda 1.0\n(defaultPrim = \"A\" subLayers = [@omniverse://server/x.usd@])\ndef Scope \"A\" {}\n",
    ] {
        let source = f.source("net.usda", text_body);
        let (code, body) = f.run("reference-attach", &["--source", &source]);
        assert_eq!(code, 2, "{text_body}: {body:?}");
        assert_eq!(text(&body, "code"), "PERMISSION_DENIED", "{text_body}");
    }
    assert!(f.listing().is_empty());
}

#[test]
fn sources_outside_the_static_profile_are_refused_with_an_explicit_diagnostic() {
    let f = Fixture::new();
    let big = format!(
        "#usda 1.0\n(defaultPrim = \"A\")\ndef Scope \"A\" {{}}\n# {}\n",
        "x".repeat(70_000)
    );
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "payload",
            b"#usda 1.0\n(defaultPrim = \"A\")\ndef \"A\" (payload = @local.usda@</X>)\n{\n}\n".to_vec(),
            "UNSUPPORTED_PROPERTY",
        ),
        (
            "variants",
            b"#usda 1.0\n(defaultPrim = \"A\")\ndef \"A\" (variantSets = [\"v\"])\n{\n}\n".to_vec(),
            "UNSUPPORTED_PROPERTY",
        ),
        (
            "sublayer",
            b"#usda 1.0\n(defaultPrim = \"A\" subLayers = [@other.usda@])\ndef Scope \"A\" {}\n".to_vec(),
            "DEPENDENCY_UNAVAILABLE",
        ),
        (
            "no default prim",
            b"#usda 1.0\ndef Scope \"A\" {}\n".to_vec(),
            "UNSUPPORTED_PROPERTY",
        ),
        (
            "default prim not defined",
            b"#usda 1.0\n(defaultPrim = \"B\")\ndef Scope \"A\" {}\n".to_vec(),
            "NOT_FOUND",
        ),
        (
            "up axis X",
            b"#usda 1.0\n(defaultPrim = \"A\" upAxis = \"X\")\ndef Scope \"A\" {}\n".to_vec(),
            "UNSUPPORTED_PROPERTY",
        ),
        (
            "non-finite units",
            b"#usda 1.0\n(defaultPrim = \"A\" metersPerUnit = inf)\ndef Scope \"A\" {}\n".to_vec(),
            "NON_FINITE_NUMBER",
        ),
        (
            "zero units",
            b"#usda 1.0\n(defaultPrim = \"A\" metersPerUnit = 0.0)\ndef Scope \"A\" {}\n".to_vec(),
            "INVALID_ARGUMENT",
        ),
        (
            "element-like name without identity",
            b"#usda 1.0\n(defaultPrim = \"A\")\ndef Scope \"A\"\n{\n    def Scope \"PRT_beam\" {}\n}\n".to_vec(),
            "INVALID_ID",
        ),
        ("too large", big.into_bytes(), "LIMIT_EXCEEDED"),
        ("not a layer", b"hello".to_vec(), "INTEGRITY_ERROR"),
        ("empty", Vec::new(), "INTEGRITY_ERROR"),
        ("usdz", b"PK\x03\x04zip".to_vec(), "UNSUPPORTED_PROPERTY"),
        ("truncated", b"#usda 1.0\n(".to_vec(), "INTEGRITY_ERROR"),
    ];
    for (what, bytes, code) in cases {
        let source = f.source("bad.usda", &bytes);
        let (exit, body) = f.run("reference-attach", &["--source", &source]);
        assert_eq!(exit, 2, "{what}: {body:?}");
        assert_eq!(text(&body, "code"), code, "{what}: {body:?}");
        assert!(
            f.run("reference-inspect", &[]).0 != 0,
            "{what}: no revision may exist"
        );
    }
    // Missing and non-regular paths.
    let missing = f.root.join("sources").join("missing.usda");
    let (_, body) = f.run("reference-attach", &["--source", missing.to_str().unwrap()]);
    assert_eq!(text(&body, "code"), "NOT_FOUND");
    let dir = f.root.join("sources");
    let (_, body) = f.run("reference-attach", &["--source", dir.to_str().unwrap()]);
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    // A rejected attach does not leave a revision; only blobs of a failed verification could.
    assert!(
        !f.listing()
            .keys()
            .any(|name| name.starts_with("revisions/"))
    );
}

#[test]
fn a_tampered_stored_source_is_detected_when_the_head_is_opened() {
    let f = Fixture::new();
    let source = f.source("site.usda", SITE);
    f.attach(&source, &[]);
    let digest = sha(SITE.as_bytes());
    let blob = f.blob_path(&digest);
    let original = fs::read(&blob).unwrap();
    let mut changed = original.clone();
    let at = changed.len() - 3;
    changed[at] ^= 1;
    fs::write(&blob, &changed).unwrap();
    let (code, body) = f.run("reference-inspect", &[]);
    assert_eq!(
        (code, text(&body, "code")),
        (2, "INTEGRITY_ERROR".to_owned())
    );
    for command in ["plate-inspect", "revision-open"] {
        let (code, body) = f.run(command, &[]);
        assert_eq!(
            (code, text(&body, "code")),
            (2, "INTEGRITY_ERROR".to_owned()),
            "{command}"
        );
    }
    fs::write(&blob, &original).unwrap();
    f.inspect();
}

#[test]
fn a_workspace_forks_publishes_and_undoes_with_the_foreign_layer_intact() {
    let f = Fixture::new();
    let published = f.store();
    let source = f.source("site.usda", SITE);
    let attached = f.attach(&source, &[]);
    let id = reference_id(&attached);
    let digest = sha(SITE.as_bytes());
    let name = format!("external/{digest}.usda");
    let draft = f.root.join("draft").to_str().unwrap().to_owned();
    f.ok_in(&published, "workspace-create", &["--workspace-dir", &draft]);
    // The draft is a separate store holding the same foreign bytes.
    let moved = f.ok_in(
        &draft,
        "reference-move",
        &["--id", &id, "--translation", "7,8,9"],
    );
    assert_eq!(external(&moved), vec![(name.clone(), digest.clone())]);
    let inspected = f.ok_in(&draft, "reference-inspect", &[]);
    assert_eq!(at(&inspected, &["references"]).array().unwrap().len(), 1);
    // The published head has not moved.
    let before = f.inspect();
    assert_eq!(
        at(&before, &["references"]).array().unwrap()[0]
            .get("placement")
            .unwrap()
            .get("translation_mm")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [0.0, 0.0, 0.0]
    );
    f.ok_in(
        &published,
        "workspace-publish",
        &["--workspace-dir", &draft],
    );
    let after = f.inspect();
    let wrapper = &at(&after, &["references"]).array().unwrap()[0];
    assert_eq!(
        at(wrapper, &["placement", "translation_mm"])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [7.0, 8.0, 9.0]
    );
    assert_eq!(fs::read(f.blob_path(&digest)).unwrap(), SITE.as_bytes());
    // Undo in a draft restores an earlier state and keeps the foreign layer.
    let draft2 = f.root.join("draft2").to_str().unwrap().to_owned();
    f.ok_in(
        &published,
        "workspace-create",
        &["--workspace-dir", &draft2],
    );
    f.ok_in(
        &draft2,
        "reference-move",
        &["--id", &id, "--translation", "1,1,1"],
    );
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(["workspace-undo", "--workspace-dir", &draft2])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let undone = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    assert!(
        layers(&undone)
            .iter()
            .any(|(n, d, r)| *n == name && *d == digest && r == "external")
    );
    let inspected = f.ok_in(&draft2, "reference-inspect", &[]);
    let wrapper = &at(&inspected, &["references"]).array().unwrap()[0];
    assert_eq!(
        at(wrapper, &["placement", "translation_mm"])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>(),
        [7.0, 8.0, 9.0]
    );
}

#[test]
fn unknown_options_and_missing_arguments_are_invalid_arguments() {
    let f = Fixture::new();
    let source = f.source("site.usda", SITE);
    assert_eq!(f.rejected("reference-attach", &[]), "INVALID_ARGUMENT");
    assert_eq!(
        f.rejected(
            "reference-attach",
            &["--source", &source, "--nonsense", "1"]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "reference-attach",
            &["--source", &source, "--rotation-z-deg", "nan"]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "reference-attach",
            &["--source", &source, "--translation", "1,2"]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "reference-attach",
            &["--source", &source, "--translation", "nan,0,0"]
        ),
        "NON_FINITE_NUMBER"
    );
    assert_eq!(
        f.rejected(
            "reference-attach",
            &["--source", &source, "--new-id", "short"]
        ),
        "INVALID_ID"
    );
    assert_eq!(
        f.rejected("reference-move", &["--translation", "1,1,1"]),
        "INVALID_ARGUMENT"
    );
    assert!(f.listing().is_empty());
}

#[test]
fn a_revision_edited_outside_beamcraft_to_override_foreign_content_is_refused_on_read_and_write() {
    // Each edit is one text change to the owned authored layer that an ordinary text editor could
    // make. The commit goes in without content verification (as revision-commit would); every
    // BeamCraft command that opens the head then refuses it.
    let holder_open = "                {
                }
";
    type Edit = Box<dyn Fn(String) -> String>;
    let edits: Vec<(&str, Edit)> = vec![
        (
            "a type on the arc holder",
            Box::new(|t| t.replace("def \"Source\" (", "def Scope \"Source\" (")),
        ),
        (
            "a child below the foreign root",
            Box::new(move |t| {
                t.replace(
                    holder_open,
                    "                {
                    def Scope \"Intruder\" {}
                }
",
                )
            }),
        ),
        (
            "a property on the foreign root",
            Box::new(move |t| {
                t.replace(
                    holder_open,
                    "                {
                    custom string note = \"edited\"
                }
",
                )
            }),
        ),
    ];
    for (what, edit) in edits {
        let f = Fixture::new();
        let source = f.source("site.usda", SITE);
        f.attach(&source, &[]);
        f.tamper_authored(edit);
        let (code, body) = f.run("reference-inspect", &[]);
        assert_eq!(
            (code, text(&body, "code")),
            (2, "FOREIGN_READ_ONLY".to_owned()),
            "{what}: reference-inspect {body:?}"
        );
        let (code, body) = f.run(
            "create-plate",
            &["--width", "1000", "--length", "500", "--thickness", "10"],
        );
        assert_eq!(
            (code, text(&body, "code")),
            (2, "FOREIGN_READ_ONLY".to_owned()),
            "{what}: create-plate {body:?}"
        );
    }
}

#[test]
fn the_stage_budget_limits_how_many_distinct_sources_a_revision_can_hold() {
    // Eight layers in all: root, authored, derived and five distinct foreign layers.
    let f = Fixture::new();
    for n in 0..5 {
        let source = f.source(
            &format!("site-{n}.usda"),
            SITE.replace("declared by the source", &format!("source number {n}")),
        );
        f.attach(&source, &[]);
    }
    let before = f.listing();
    let sixth = f.source(
        "site-5.usda",
        SITE.replace("declared by the source", "number five"),
    );
    let (code, body) = f.run("reference-attach", &["--source", &sixth]);
    assert_eq!(
        (code, text(&body, "code")),
        (2, "LIMIT_EXCEEDED".to_owned())
    );
    assert_eq!(f.listing(), before, "the refused attach stored nothing");
    assert_eq!(at(&f.inspect(), &["references"]).array().unwrap().len(), 5);
}
