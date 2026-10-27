// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original generated synthetic fixtures, MPL-2.0. Expectations follow F04 sections
// 2/6/8 and the U06 plan row; no third-party layer or asset is used.

//! U06 end to end: layers authored by the real U02 layer engine are stored and committed through
//! the real `beamcraft` binary, then reopened by a fresh process and by a fresh in-process
//! reader. A second group drives the host API with a content verifier built on the real
//! U01/U03 layer engine (duplicate/short IDs, dangling links, stale, partial and foreign indexes).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_host::{
    revision::{
        CommitOutcome, ContentRejection, ContentVerifier, ExpectedBase, IndexEvidence, LayerEntry,
        LayerRole, NoFault, RevisionDraft, RevisionError, RevisionStore, Uuid7, VerifyRequest,
    },
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_layer_engine::{
    authored::{
        Author, Change, Decision, Definition, EditTargets, Element, MemberKind, Mesh, Placement,
        Prepare, Resource, Shape, schema_identity, validate_layers,
    },
    index::{Completeness, ElementIndex, IndexBuilder},
    *,
};
use beamcraft_model::{
    ErrorCode,
    digest::{ContentDigest, Sha256},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

const PROJECT: &str = "0199e7a0-0000-7000-8000-000000000001";
const WORKSPACE: &str = "0199e7a0-0000-7000-8000-000000000002";
const PLATE: &str = "0199e7a0-0000-7000-8000-000000000003";
const MEMBER: &str = "0199e7a0-0000-7000-8000-000000000004";
const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000005";
const OP1: &str = "0199e7a0-0000-7000-8000-000000000006";
const OP2: &str = "0199e7a0-0000-7000-8000-000000000007";
const REV1: &str = "0199e7a0-0000-7000-8000-0000000000a1";
const REV2: &str = "0199e7a0-0000-7000-8000-0000000000a2";
const COMMIT1: &str = "0199e7a0-0000-7000-8000-0000000000b1";
const COMMIT2: &str = "0199e7a0-0000-7000-8000-0000000000b2";

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("u06-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        fs::create_dir(root.join("files")).unwrap();
        Self(root)
    }
    fn store_dir(&self) -> PathBuf {
        self.0.join("store")
    }
    fn blobs(&self) -> BlobStore {
        BlobStore::new(self.store_dir()).unwrap()
    }
    fn revisions(&self) -> RevisionStore {
        RevisionStore::new(self.blobs())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.file_name().is_some());
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---- authoring fixtures (U02) ---------------------------------------------------------------

fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}
fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(20),
    }
}
fn asset(name: &str, text: impl AsRef<[u8]>) -> Asset {
    Asset {
        name: name.into(),
        digest: digest(text.as_ref()),
        bytes: text.as_ref().to_vec(),
    }
}
fn layer(body: &str) -> String {
    format!("#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\n{body}\n")
}
fn request(assets: Vec<Asset>, owned: &[&str], page_items: usize) -> OpenRequest {
    let mut request = OpenRequest {
        identity: Identity {
            project: PROJECT.into(),
            workspace: WORKSPACE.into(),
            revision: digest(b"pinned U06 base"),
            generation: 42,
        },
        root: "root.usda".into(),
        assets,
        owned_layers: owned.iter().map(|s| (*s).into()).collect(),
        manifest: digest(b"placeholder"),
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets {
            page_items,
            ..Budgets::default()
        },
    };
    request.manifest = request.manifest_digest().unwrap();
    request
}
fn initial(page_items: usize) -> OpenRequest {
    request(
        vec![
            asset(
                "root.usda",
                "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\nsubLayers=[@authored.usda@,@derived.usda@,@weak.usda@])\n",
            ),
            asset("authored.usda", layer("")),
            asset("derived.usda", layer("")),
            asset("weak.usda", layer("")),
        ],
        &["root.usda", "authored.usda", "derived.usda", "weak.usda"],
        page_items,
    )
}
fn targets() -> EditTargets {
    EditTargets {
        authored: "authored.usda".into(),
        derived: "derived.usda".into(),
    }
}
fn pose() -> Placement {
    Placement {
        translation_mm: [12.25, -42.125, 0.0],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
    }
}
fn resources() -> Vec<Resource> {
    vec![
        Resource::new(Definition::RectangleProfile {
            width_mm: 100.0,
            height_mm: 50.0,
            section: Default::default(),
        })
        .unwrap(),
        Resource::new(Definition::Material {
            density_kg_m3: Some(1000.0),
            grade: None,
            standard: None,
            display_name: None,
        })
        .unwrap(),
    ]
}
fn authored_batch() -> Vec<Change> {
    let r = resources();
    vec![
        Change::Create(Element {
            id: PLATE.into(),
            name: "Plate".into(),
            shape: Shape::Plate {
                contour_mm: vec![
                    [0.0, 0.0, 0.0],
                    [1000.0, 0.0, 0.0],
                    [1000.0, 500.0, 0.0],
                    [0.0, 500.0, 0.0],
                ],
                thickness_mm: 10.0,
                thickness_side: None,
                holes: Vec::new(),
                hole_records: Vec::new(),
                cuts: Vec::new(),
            },
            placement: pose(),
            material: Some(r[1].digest),
        }),
        Change::Create(Element {
            id: MEMBER.into(),
            name: "Member".into(),
            shape: Shape::Member {
                kind: MemberKind::Beam,
                axis_mm: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 2000.0]],
                profile: r[0].digest,
                roll_rad: Some(0.125),
                cuts: Vec::new(),
            },
            placement: pose(),
            material: None,
        }),
        Change::Mesh(Mesh {
            id: PLATE.into(),
            shape_key: digest(b"engine-supplied-shape-key"),
            points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            face_counts: vec![4],
            indices: vec![0, 1, 2, 3],
            quantities: None,
        }),
    ]
}
fn install(
    author: &mut Author,
    context: &Context,
    op: &str,
    resources: Vec<Resource>,
    changes: Vec<Change>,
) -> Opened {
    let prepared = author
        .prepare(
            Prepare {
                context: context.clone(),
                op_id: op.into(),
                actor: ACTOR.into(),
                schema: schema_identity().unwrap(),
                resources,
                changes,
            },
            &control(),
        )
        .unwrap();
    author
        .install(
            context,
            &prepared.token,
            &prepared.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap()
}
fn relabel(author: &mut Author, context: &Context, op: &str, id: &str, name: &str) -> Opened {
    install(
        author,
        context,
        op,
        Vec::new(),
        vec![Change::Label {
            id: id.into(),
            name: name.into(),
        }],
    )
}
/// A U02 author holding the first populated state (generation 43).
fn populated(page_items: usize) -> (Author, Opened) {
    let (mut author, opened) = Author::open(
        initial(page_items),
        targets(),
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap();
    let installed = install(
        &mut author,
        &opened.context,
        OP1,
        resources(),
        authored_batch(),
    );
    (author, installed)
}
fn role_of(name: &str) -> LayerRole {
    match name {
        "derived.usda" => LayerRole::Derived,
        "weak.usda" => LayerRole::External,
        _ => LayerRole::Authored,
    }
}
fn exported(author: &Author, context: &Context) -> Vec<Asset> {
    author.export(context, &control()).unwrap()
}
fn entries(assets: &[Asset]) -> Vec<LayerEntry> {
    assets
        .iter()
        .map(|a| {
            LayerEntry::new(&a.name, role_of(&a.name), a.digest, a.bytes.len() as u64).unwrap()
        })
        .collect()
}
fn draft(assets: &[Asset], revision: &str, op: &str, generation: u64) -> RevisionDraft {
    RevisionDraft {
        project: Uuid7::parse(PROJECT).unwrap(),
        workspace: Uuid7::parse(WORKSPACE).unwrap(),
        revision: Uuid7::parse(revision).unwrap(),
        op: Uuid7::parse(op).unwrap(),
        workspace_generation: generation,
        schema: schema_identity().unwrap().digest,
        root_layer: "root.usda".into(),
        entries: entries(assets),
    }
}
fn publish(store: &BlobStore, assets: &[Asset]) {
    for a in assets {
        assert_eq!(store.publish(&a.bytes).unwrap().0, a.digest);
    }
}
fn base(revision: &str) -> ExpectedBase {
    ExpectedBase::Revision(Uuid7::parse(revision).unwrap())
}

// ---- content verifier built on the real layer engine -----------------------------------------

fn reject(d: Diagnostic) -> ContentRejection {
    ContentRejection {
        code: d.code,
        message: d.message,
    }
}
fn evidence(index: &ElementIndex, root_layer: &str, schema: ContentDigest) -> IndexEvidence {
    let token = index.token();
    IndexEvidence {
        project: token.identity.project.clone(),
        workspace: token.identity.workspace.clone(),
        generation: token.identity.generation,
        pinned_revision: token.identity.revision,
        root_layer: root_layer.to_owned(),
        schema,
        complete: token.completeness == Completeness::Complete,
        structure_digest: token.structure_digest,
        prim_count: token.prim_count as u64,
    }
}

/// Opens exactly the committed bytes in a fresh reader and derives the index from them, so no
/// index can be stale, partial or foreign. Used with U02-authored layers it also runs the U02
/// structural validator.
struct Derived {
    generation: u64,
    pinned: ContentDigest,
    u02: bool,
}
impl Derived {
    fn open_request(&self, request: &VerifyRequest<'_>) -> OpenRequest {
        let mut open = OpenRequest {
            identity: Identity {
                project: PROJECT.into(),
                workspace: WORKSPACE.into(),
                revision: self.pinned,
                generation: self.generation,
            },
            root: request.root_layer.into(),
            assets: request
                .layers
                .iter()
                .map(|l| Asset {
                    name: l.name.into(),
                    digest: l.digest,
                    bytes: l.bytes.to_vec(),
                })
                .collect(),
            owned_layers: request
                .layers
                .iter()
                .filter(|l| l.role != LayerRole::External)
                .map(|l| l.name.to_owned())
                .collect(),
            manifest: digest(b"placeholder"),
            backend: BACKEND,
            profile: Profile::StaticLocal,
            budgets: Budgets::default(),
        };
        open.manifest = open.manifest_digest().unwrap();
        open
    }
}
impl ContentVerifier for Derived {
    fn verify(&self, request: &VerifyRequest<'_>) -> Result<IndexEvidence, ContentRejection> {
        let open = self.open_request(request);
        if self.u02 {
            validate_layers(open.clone(), schema_identity().unwrap(), &control())
                .map_err(reject)?;
        }
        let mut reader = Reader::default();
        let opened = reader.open(open, &control()).map_err(reject)?;
        let index = reader
            .element_index(&opened.context, &control())
            .map_err(reject)?;
        reader.verify_index(&index, &control()).map_err(reject)?;
        Ok(evidence(&index, request.root_layer, request.schema))
    }
}

/// A caller-supplied index of a live author: accepted only if it covers exactly the layers being
/// committed and passes the U03 guard (`verify_index`: live context, complete, untampered).
struct Supplied<'a> {
    author: &'a Author,
    context: &'a Context,
    index: &'a ElementIndex,
}
impl ContentVerifier for Supplied<'_> {
    fn verify(&self, request: &VerifyRequest<'_>) -> Result<IndexEvidence, ContentRejection> {
        let layers = request.layers;
        let live = self
            .author
            .export(self.context, &control())
            .map_err(reject)?;
        let same = live.len() == layers.len()
            && layers.iter().all(|l| {
                live.iter()
                    .any(|a| a.name == l.name && a.digest == l.digest)
            });
        if !same {
            return Err(ContentRejection {
                code: ErrorCode::IntegrityError,
                message: "layers differ from the context of the supplied index",
            });
        }
        self.author
            .verify_index(self.index, &control())
            .map_err(reject)?;
        // The supplied index was built by this author from its own root and schema.
        Ok(evidence(
            self.index,
            "root.usda",
            schema_identity().unwrap().digest,
        ))
    }
}

fn commit_verified(
    store: &RevisionStore,
    d: &RevisionDraft,
    expected: &ExpectedBase,
    verifier: &dyn ContentVerifier,
) -> Result<beamcraft_host::revision::Committed, RevisionError> {
    store.commit_with(d, expected, Some(verifier), &mut NoFault)
}

/// Reopens a committed revision in a brand-new reader from verified stored bytes only.
fn reopen_index(store: &RevisionStore) -> (ContentDigest, ElementIndex) {
    let revision = store.open().unwrap();
    let manifest = revision.manifest();
    let assets: Vec<Asset> = manifest
        .content()
        .entries()
        .iter()
        .map(|e| Asset {
            name: e.name().into(),
            digest: e.digest(),
            bytes: revision.layer_bytes(e.name()).unwrap(),
        })
        .collect();
    let owned: Vec<&str> = manifest
        .content()
        .entries()
        .iter()
        .filter(|e| e.role() != LayerRole::External)
        .map(LayerEntry::name)
        .collect();
    let mut open = request(assets, &owned, 64);
    open.identity = Identity {
        project: manifest.content().project().to_string(),
        workspace: manifest.content().workspace().to_string(),
        revision: revision.info().manifest_digest(),
        generation: manifest.content().workspace_generation(),
    };
    open.root = manifest.content().root_layer().into();
    open.manifest = open.manifest_digest().unwrap();
    let mut reader = Reader::default();
    let opened = reader.open(open, &control()).unwrap();
    let index = reader.element_index(&opened.context, &control()).unwrap();
    (revision.info().manifest_digest(), index)
}

// ---- hand-built (non-U02) layer sets for negative content cases -----------------------------

fn hex(id: &str) -> String {
    id.replace('-', "")
}
fn seq(n: u32) -> String {
    format!("0199e7a0-0000-7000-8000-{n:012x}")
}
fn el(id: &str, extra: &str) -> String {
    format!(
        "def Xform \"PRT_{}\" {{\n custom string bim:id = \"{id}\"\n custom string bim:name = \"label\"\n{extra}\n}}\n",
        hex(id)
    )
}
/// A root layer holding the world plus an empty derived layer: the smallest valid layer set.
fn hand_built(elements: &str, other: &str) -> Vec<Asset> {
    vec![
        asset(
            "root.usda",
            layer(&format!(
                "def Xform \"World\" {{\n def Scope \"Steel\" {{\n def Scope \"Parts\" {{\n{elements}\n}}\n}}\n def Scope \"Definitions\" {{\n}}\n{other}\n}}"
            )),
        ),
        asset("derived.usda", layer("")),
    ]
}

// ---- tests ------------------------------------------------------------------------------------

#[test]
fn real_binary_commits_u02_layers_and_a_fresh_process_and_reader_verify_them() {
    let fx = Fixture::new();
    let store_arg = fx.store_dir();
    let store_s = store_arg.to_str().unwrap();
    let run = |args: &[&str]| -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    };
    let text = |j: &Json, key: &str| j.get(key).unwrap().string().unwrap().to_owned();

    // Author revision 1 with the real U02 engine and keep its index for the final comparison.
    let (mut author, opened) = populated(64);
    let authored_index = author.element_index(&opened.context, &control()).unwrap();
    let assets = exported(&author, &opened.context);
    let generation = opened.context.identity().generation;
    assert_eq!(generation, 43);

    let blob_put = |assets: &[Asset]| {
        for a in assets {
            let file = fx.0.join("files").join(&a.name);
            fs::write(&file, &a.bytes).unwrap();
            let (exit, body) = run(&[
                "put-blob",
                "--store",
                store_s,
                "--file",
                file.to_str().unwrap(),
            ]);
            assert_eq!(exit, 0);
            assert_eq!(text(&body, "digest"), a.digest.to_string());
        }
    };
    let layer_arg = |assets: &[Asset]| {
        assets
            .iter()
            .map(|a| format!("{}:{}:{}", role_of(&a.name).as_str(), a.name, a.digest))
            .collect::<Vec<_>>()
            .join(",")
    };
    let schema = schema_identity().unwrap().digest.to_string();
    let commit_args = |assets: &[Asset], revision: &str, op: &str, base: &str, generation: u64| {
        vec![
            "commit-revision".to_owned(),
            "--store".into(),
            store_s.to_owned(),
            "--project-id".into(),
            PROJECT.into(),
            "--workspace-id".into(),
            WORKSPACE.into(),
            "--revision-id".into(),
            revision.into(),
            "--op-id".into(),
            op.into(),
            "--generation".into(),
            generation.to_string(),
            "--base".into(),
            base.into(),
            "--schema".into(),
            schema.clone(),
            "--root-layer".into(),
            "root.usda".into(),
            "--layers".into(),
            layer_arg(assets),
        ]
    };
    let run_owned = |args: Vec<String>| run(&args.iter().map(String::as_str).collect::<Vec<_>>());

    // Nothing is published yet; a layer that was never stored cannot be committed.
    let (exit, body) = run(&["open-revision", "--store", store_s]);
    assert_eq!((exit, text(&body, "code").as_str()), (2, "NOT_FOUND"));
    let (exit, body) = run_owned(commit_args(&assets, REV1, COMMIT1, "none", generation));
    assert_eq!((exit, text(&body, "code").as_str()), (2, "NOT_FOUND"));
    assert!(!store_arg.join("revisions").exists());

    // Store the blobs, then commit revision 1.
    blob_put(&assets);
    let (exit, first) = run_owned(commit_args(&assets, REV1, COMMIT1, "none", generation));
    assert_eq!(exit, 0, "{first:?}");
    assert_eq!(text(&first, "outcome"), "revision_committed");
    assert_eq!(text(&first, "sequence"), "1");
    assert_eq!(text(&first, "revision_id"), REV1);
    assert_eq!(text(&first, "workspace_generation"), "43");
    assert_eq!(first.get("base").unwrap(), &Json::Null);
    assert_eq!(first.get("index").unwrap(), &Json::Null);
    let manifest1 = text(&first, "manifest_digest");

    // The identical retry, from a new process, returns the recorded revision.
    let (exit, again) = run_owned(commit_args(&assets, REV1, COMMIT1, "none", generation));
    assert_eq!(
        (exit, text(&again, "outcome").as_str()),
        (0, "revision_replayed")
    );
    assert_eq!(text(&again, "manifest_digest"), manifest1);
    // The same op with other content, and a stale base, are typed conflicts.
    let mut other = assets.clone();
    other[1] = asset("authored.usda", layer("# other"));
    blob_put(&other);
    let (exit, body) = run_owned(commit_args(&other, REV1, COMMIT1, "none", generation));
    assert_eq!((exit, text(&body, "code").as_str()), (2, "OP_ID_REUSE"));
    let (exit, body) = run_owned(commit_args(&other, REV2, COMMIT2, "none", generation + 1));
    assert_eq!(
        (exit, text(&body, "code").as_str()),
        (2, "BASE_REVISION_MISMATCH")
    );

    // A fresh process opens and verifies the root.
    let (exit, opened_json) = run(&["open-revision", "--store", store_s]);
    assert_eq!(exit, 0, "{opened_json:?}");
    assert_eq!(text(&opened_json, "outcome"), "revision_verified");
    assert_eq!(text(&opened_json, "manifest_digest"), manifest1);
    assert_eq!(opened_json.get("layers").unwrap().array().unwrap().len(), 4);

    // A second revision from the real engine, on top of the first.
    let opened2 = relabel(&mut author, &opened.context, OP2, PLATE, "Renamed plate");
    let assets2 = exported(&author, &opened2.context);
    let generation2 = opened2.context.identity().generation;
    assert_eq!(generation2, 44);
    blob_put(&assets2);
    let (exit, second) = run_owned(commit_args(&assets2, REV2, COMMIT2, REV1, generation2));
    assert_eq!(exit, 0, "{second:?}");
    assert_eq!(text(&second, "sequence"), "2");
    let base_json = second.get("base").unwrap();
    assert_eq!(text(base_json, "revision_id"), REV1);
    assert_eq!(text(base_json, "manifest_digest"), manifest1);
    let manifest2 = text(&second, "manifest_digest");

    // Fresh process and fresh in-process reader both see revision 2 as complete.
    let (exit, now) = run(&["open-revision", "--store", store_s]);
    assert_eq!(
        (exit, text(&now, "manifest_digest")),
        (0, manifest2.clone())
    );
    let store = fx.revisions();
    let (pinned, reopened) = reopen_index(&store);
    assert_eq!(pinned.to_string(), manifest2);
    let authored_now = author.element_index(&opened2.context, &control()).unwrap();
    assert_eq!(reopened, authored_now);
    assert_eq!(reopened.len(), 2);
    // A rename moved no identity and no structure, so revision 1's index still matches.
    assert_eq!(
        authored_index.structure_digest(),
        reopened.structure_digest()
    );

    // Hostile state is refused by a fresh process with an integrity code, not a partial read.
    let weak = assets2[3].digest.to_string();
    fs::remove_file(store_arg.join("blobs").join(&weak[..2]).join(&weak)).unwrap();
    let (exit, body) = run(&["open-revision", "--store", store_s]);
    assert_eq!((exit, text(&body, "code").as_str()), (2, "INTEGRITY_ERROR"));
    // CLI argument rules: unknown role, missing option, bad base.
    let mut bad = commit_args(&assets2, REV2, COMMIT2, REV1, generation2);
    let at = bad.iter().position(|a| a == "--layers").unwrap();
    bad[at + 1] = format!("binary:root.usda:{}", assets2[0].digest);
    assert_eq!(run_owned(bad).0, 2);
    let mut missing = commit_args(&assets2, REV2, COMMIT2, REV1, generation2);
    missing.truncate(missing.len() - 2);
    assert_eq!(run_owned(missing).0, 2);
    assert_eq!(
        run_owned(commit_args(
            &assets2,
            REV2,
            COMMIT2,
            "not-a-uuid",
            generation2
        ))
        .0,
        2
    );
}

#[test]
fn verified_commit_of_authored_layers_records_the_index_and_a_fresh_reader_reproduces_it() {
    let fx = Fixture::new();
    let store = fx.revisions();
    let (mut author, opened) = populated(64);
    let assets = exported(&author, &opened.context);
    let generation = opened.context.identity().generation;
    publish(store.blobs(), &assets);
    let index = author.element_index(&opened.context, &control()).unwrap();

    // The U03 evidence derived from the committed bytes equals the author's own index.
    let derived = Derived {
        generation,
        pinned: digest(b"pinned U06 base"),
        u02: true,
    };
    let one = commit_verified(
        &store,
        &draft(&assets, REV1, COMMIT1, generation),
        &ExpectedBase::Initial,
        &derived,
    )
    .unwrap();
    assert_eq!(one.outcome, CommitOutcome::Created);
    let recorded = one.info.manifest().index().unwrap();
    assert_eq!(recorded.structure, index.structure_digest());
    assert_eq!(recorded.prims, index.token().prim_count as u64);
    let (pinned, reopened) = reopen_index(&fx.revisions());
    assert_eq!(pinned, one.info.manifest_digest());
    assert_eq!(reopened.structure_digest(), recorded.structure);

    // Revision 2 pins revision 1's manifest digest as its base context.
    let opened2 = relabel(&mut author, &opened.context, OP2, MEMBER, "Renamed member");
    let assets2 = exported(&author, &opened2.context);
    publish(store.blobs(), &assets2);
    let d2 = draft(
        &assets2,
        REV2,
        COMMIT2,
        opened2.context.identity().generation,
    );
    let wrong_pin = Derived {
        generation: d2.workspace_generation,
        pinned: digest(b"some other base"),
        u02: true,
    };
    let error = commit_verified(&store, &d2, &base(REV1), &wrong_pin).unwrap_err();
    assert_eq!(error.code(), ErrorCode::StaleWorkspace);
    assert_eq!(store.head().unwrap().unwrap().sequence(), 1);
    let right_pin = Derived {
        generation: d2.workspace_generation,
        pinned: one.info.manifest_digest(),
        u02: true,
    };
    let two = commit_verified(&store, &d2, &base(REV1), &right_pin).unwrap();
    assert_eq!(two.info.sequence(), 2);
    // A rename does not change the structure digest recorded in the manifest.
    assert_eq!(
        two.info.manifest().index().unwrap().structure,
        recorded.structure
    );
}

#[test]
fn duplicate_short_and_dangling_identities_abort_the_commit_with_the_root_unchanged() {
    let fx = Fixture::new();
    let store = fx.revisions();
    let id = seq(0x30);
    // The duplicate is at another valid category path, so the index checks UUID uniqueness
    // rather than the reader rejecting a misplaced prim first.
    let other = format!(
        "def Scope \"Levels\" {{ {} }}",
        el(&id, "").replace("PRT_", "LVL_")
    );
    let dangling = format!(
        "custom rel bim:profile = </World/Steel/Parts/PRT_{}>",
        hex(&seq(0x99))
    );
    let cases: [(&str, Vec<Asset>, ErrorCode); 3] = [
        (
            "duplicate",
            hand_built(&el(&id, ""), &other),
            ErrorCode::DuplicateId,
        ),
        (
            "short",
            hand_built(
                "def Xform \"PRT_0199e7a0\" { custom string bim:id = \"0199e7a0\" }",
                "",
            ),
            ErrorCode::InvalidId,
        ),
        (
            "dangling",
            hand_built(&el(&id, &dangling), ""),
            ErrorCode::DanglingRelationship,
        ),
    ];
    let verifier = Derived {
        generation: 5,
        pinned: digest(b"base"),
        u02: false,
    };
    for (name, assets, expected) in cases {
        publish(store.blobs(), &assets);
        let result = commit_verified(
            &store,
            &draft(&assets, REV1, COMMIT1, 5),
            &ExpectedBase::Initial,
            &verifier,
        );
        let error = result.unwrap_err();
        assert!(matches!(error, RevisionError::Content(_)), "{name}");
        assert_eq!(error.code(), expected, "{name}");
        assert_eq!(store.head().unwrap(), None, "{name}");
    }
    // The valid layer set of the same shape commits, so the failures above were the content.
    let good = hand_built(&el(&id, ""), "");
    publish(store.blobs(), &good);
    let ok = commit_verified(
        &store,
        &draft(&good, REV1, COMMIT1, 5),
        &ExpectedBase::Initial,
        &verifier,
    )
    .unwrap();
    assert!(ok.info.manifest().index().unwrap().prims >= 1);
}

#[test]
fn stale_partial_foreign_and_mismatched_indexes_cannot_carry_a_commit() {
    let fx = Fixture::new();
    let store = fx.revisions();
    // Small pages so a partial index is possible.
    let (mut author, opened) = populated(2);
    let generation = opened.context.identity().generation;
    let assets = exported(&author, &opened.context);
    publish(store.blobs(), &assets);
    let d = draft(&assets, REV1, COMMIT1, generation);
    let live = author.element_index(&opened.context, &control()).unwrap();
    let supplied = |index| Supplied {
        author: &author,
        context: &opened.context,
        index,
    };

    // Partial: only the first page of the snapshot.
    let mut builder = IndexBuilder::new(&opened).unwrap();
    let page = author.read(&opened.context, None, &control()).unwrap();
    assert!(page.next.is_some());
    builder.push(&page, &control()).unwrap();
    let partial = builder.finish_available(&control()).unwrap();
    assert_eq!(partial.completeness(), Completeness::Partial);
    let error =
        commit_verified(&store, &d, &ExpectedBase::Initial, &supplied(&partial)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DependencyUnavailable);
    assert_eq!(store.head().unwrap(), None);

    // Foreign: an index of another author (same layers, other incarnation) is not live here.
    let (other_author, other_opened) = populated(2);
    let foreign = other_author
        .element_index(&other_opened.context, &control())
        .unwrap();
    let error =
        commit_verified(&store, &d, &ExpectedBase::Initial, &supplied(&foreign)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::StaleWorkspace);
    assert_eq!(store.head().unwrap(), None);

    // Layers that are not the ones the index covers.
    let mut elsewhere = assets.clone();
    elsewhere[1] = asset("authored.usda", layer("# not the indexed bytes"));
    publish(store.blobs(), &elsewhere);
    let error = commit_verified(
        &store,
        &draft(&elsewhere, REV1, COMMIT1, generation),
        &ExpectedBase::Initial,
        &supplied(&live),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::IntegrityError);

    // A draft whose generation is not the index's generation is a mismatched context.
    let error = commit_verified(
        &store,
        &draft(&assets, REV1, COMMIT1, generation + 1),
        &ExpectedBase::Initial,
        &supplied(&live),
    )
    .unwrap_err();
    assert_eq!(error, RevisionError::IndexMismatch);
    assert_eq!(store.head().unwrap(), None);

    // The live, complete index of the committed layers carries the commit ...
    let one = commit_verified(&store, &d, &ExpectedBase::Initial, &supplied(&live)).unwrap();
    assert_eq!(
        one.info.manifest().index().unwrap().structure,
        live.structure_digest()
    );

    // ... and becomes stale once the author installs the next state.
    let opened2 = relabel(&mut author, &opened.context, OP2, PLATE, "Next");
    let assets2 = exported(&author, &opened2.context);
    publish(store.blobs(), &assets2);
    let d2 = draft(
        &assets2,
        REV2,
        COMMIT2,
        opened2.context.identity().generation,
    );
    let stale = Supplied {
        author: &author,
        context: &opened2.context,
        index: &live,
    };
    let error = commit_verified(&store, &d2, &base(REV1), &stale).unwrap_err();
    assert_eq!(error.code(), ErrorCode::StaleWorkspace);
    assert_eq!(store.head().unwrap().unwrap(), one.info);
}

#[test]
fn the_verifier_indexes_the_selected_root_and_evidence_for_another_root_is_refused() {
    let fx = Fixture::new();
    let store = fx.revisions();
    let (author, opened) = populated(64);
    let generation = opened.context.identity().generation;
    let assets = exported(&author, &opened.context);
    publish(store.blobs(), &assets);
    let index = author.element_index(&opened.context, &control()).unwrap();
    assert!(!index.is_empty());

    // The same layers, composed from the empty weak layer instead of root.usda: the verifier
    // must open that root, so the recorded index describes it (no prims), not root.usda's.
    let mut other_root = draft(&assets, REV1, COMMIT1, generation);
    other_root.root_layer = "weak.usda".into();
    let derived = Derived {
        generation,
        pinned: digest(b"pinned U06 base"),
        u02: false,
    };
    let one = commit_verified(&store, &other_root, &ExpectedBase::Initial, &derived).unwrap();
    let recorded = one.info.manifest().index().unwrap();
    assert_eq!(recorded.prims, 0);
    assert_ne!(recorded.structure, index.structure_digest());

    // A verifier that indexed root.usda cannot vouch for a draft that selects another root.
    let fx2 = Fixture::new();
    let store2 = fx2.revisions();
    publish(store2.blobs(), &assets);
    let supplied = Supplied {
        author: &author,
        context: &opened.context,
        index: &index,
    };
    let error =
        commit_verified(&store2, &other_root, &ExpectedBase::Initial, &supplied).unwrap_err();
    assert_eq!(error, RevisionError::IndexMismatch);
    assert_eq!(store2.head().unwrap(), None);
    // The same evidence is accepted for the root it describes.
    let ok = commit_verified(
        &store2,
        &draft(&assets, REV1, COMMIT1, generation),
        &ExpectedBase::Initial,
        &supplied,
    )
    .unwrap();
    assert_eq!(
        ok.info.manifest().index().unwrap().structure,
        index.structure_digest()
    );
}
