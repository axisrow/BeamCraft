// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and members, MPL-2.0.

//! Issue 26: a reader evaluates bit-identical shape inputs once per thread (the body cache behind
//! `beamcraft_cli::plate::inspect`). A warm cache returns exactly what a cold evaluation returns,
//! and every stored mesh it receives is still compared with the kernel body: a tampered derived
//! layer is refused however warm the cache is.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_cli::plate::{Inspection, body_cache_counts, inspect};
use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{error::ErrorCode, id::ElementId, prim::ElementCategory};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const PLATE: &str = "019a7bf0-0000-7000-8000-000000000010";
const PLATE_COPY: &str = "019a7bf0-0000-7000-8000-000000000011";
const L_SHAPE: &str = "0,0;1000,0;1000,300;400,300;400,600;0,600";

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("issue26-cache");
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

    fn store(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.root.join("store")).unwrap())
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

    fn definition(&self, command: &str, extra: &[&str]) -> String {
        let reply = self.ok(command, extra);
        reply
            .get("definition")
            .unwrap()
            .get("digest")
            .unwrap()
            .string()
            .unwrap()
            .to_owned()
    }

    /// Commits a head-based revision with changed layer text and no content verification, the way
    /// a layer edited outside BeamCraft would arrive.
    fn tamper(&self, edit: impl Fn(&str, String) -> String) {
        let store = self.store();
        let head = store.open().unwrap();
        let content = head.manifest().content();
        let mut ids = IdSource::system();
        let mut entries = Vec::new();
        for entry in content.entries() {
            let original = String::from_utf8(head.layer_bytes(entry.name()).unwrap()).unwrap();
            let edited = edit(entry.name(), original);
            store.blobs().publish(edited.as_bytes()).unwrap();
            entries.push(
                LayerEntry::from_bytes(entry.name(), entry.role(), edited.as_bytes()).unwrap(),
            );
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
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_same(a: &Inspection, b: &Inspection) {
    assert_eq!(a.revision, b.revision);
    assert_eq!(a.plates, b.plates);
    assert_eq!(a.members, b.members);
    assert_eq!(a.quantities, b.quantities);
    assert_eq!(a.other_elements, b.other_elements);
    assert_eq!(a.definitions, b.definitions);
    assert_eq!(a.snapshot, b.snapshot);
    assert_eq!(a.project_record, b.project_record);
    assert_eq!(a.levels, b.levels);
    assert_eq!(a.elements, b.elements);
}

/// Cache hits and misses of this thread so far.
fn counts() -> (u64, u64) {
    body_cache_counts().unwrap()
}

#[test]
fn a_warm_cache_returns_what_a_cold_evaluation_returns() {
    let f = Fixture::new();
    let steel = f.definition("define-material", &["--density", "7850"]);
    let i300 = f.definition(
        "define-profile",
        &[
            "--family", "i_shape", "--h", "300", "--b", "150", "--tw", "7", "--tf", "10", "--r",
            "15",
        ],
    );
    let rectangle = f.definition("define-profile", &["--width", "200", "--height", "300"]);
    let plate = [
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--material-digest",
        &steel,
        "--new-id",
        PLATE,
    ];
    f.ok("create-plate", &plate);
    f.ok("copy-element", &["--id", PLATE, "--offset", "2000,0,0"]);
    let gusset = "019a7bf0-0000-7000-8000-000000000020";
    let contour = [
        "--contour",
        L_SHAPE,
        "--thickness",
        "10",
        "--hole-rect",
        "100,100,100,100",
        "--new-id",
        gusset,
    ];
    f.ok("create-plate", &contour);
    f.ok("copy-element", &["--id", gusset, "--offset", "0,2000,0"]);
    let cut = [
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--cut-point",
        "250,0,0",
        "--cut-normal",
        "1,0,0",
    ];
    f.ok("create-plate", &cut);
    let beam = "019a7bf0-0000-7000-8000-000000000030";
    let section = [
        "--start",
        "0,0,0",
        "--end",
        "3000,0,0",
        "--profile-digest",
        &i300,
        "--material-digest",
        &steel,
        "--new-id",
        beam,
    ];
    f.ok("create-member", &section);
    f.ok("copy-element", &["--id", beam, "--offset", "0,0,1000"]);
    let column = [
        "--kind",
        "column",
        "--start",
        "0,0,0",
        "--end",
        "0,0,3000",
        "--profile-digest",
        &rectangle,
    ];
    f.ok("create-member", &column);

    // A fresh thread starts with an empty cache: its result is the cold evaluation.
    let cold = {
        let store = f.store();
        std::thread::spawn(move || inspect(&store).unwrap())
            .join()
            .unwrap()
    };
    let store = f.store();
    let before = counts();
    let first = inspect(&store).unwrap();
    let after_first = counts();
    // Eight elements, five distinct shapes: three copies share a body within the first call,
    // and the unsupported-listing pass rides the same cache, so every evaluated body hits again.
    assert_eq!(after_first.1 - before.1, 5);
    assert_eq!(after_first.0 - before.0, 8);
    let second = inspect(&store).unwrap();
    let after_second = counts();
    // The second call evaluates nothing.
    assert_eq!(after_second.1, after_first.1);
    assert_eq!(after_second.0 - after_first.0, 13);
    assert_same(&cold, &first);
    assert_same(&first, &second);
    assert_eq!(first.plates.len() + first.members.len(), 8);
}

#[test]
fn a_warm_cache_still_refuses_a_tampered_derived_mesh() {
    let f = Fixture::new();
    let plate = [
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--new-id",
        PLATE,
    ];
    f.ok("create-plate", &plate);
    f.ok(
        "copy-element",
        &[
            "--id", PLATE, "--offset", "2000,0,0", "--new-id", PLATE_COPY,
        ],
    );
    let store = f.store();
    // Warm this thread's cache with the shape both plates share.
    inspect(&store).unwrap();
    // The copy's stored box is stretched from 10 to 12 mm while its authored inputs stay those
    // of the source. Points and extent change together and an axis-aligned box keeps its face
    // normals, so the layer passes every structural rule: only the kernel comparison can tell.
    let path = ElementId::parse(PLATE_COPY)
        .unwrap()
        .prim_path(ElementCategory::Part);
    let marker = format!("\"{}\"", path.rsplit('/').next().unwrap());
    f.tamper(|name, mut text| {
        if name == "derived.usda" {
            let block = text.find(&marker).unwrap();
            for attribute in ["point3f[] points = [", "float3[] extent = ["] {
                let start = block + text[block..].find(attribute).unwrap();
                let end = start + text[start..].find('\n').unwrap();
                let line = text[start..end].replace(", 10.0)", ", 12.0)");
                text.replace_range(start..end, &line);
            }
        }
        text
    });
    let before = counts();
    let refused = inspect(&store).unwrap_err();
    // The body came from the cache, and the received mesh was still compared with it.
    assert_eq!(counts().1, before.1);
    assert!(counts().0 > before.0);
    assert_eq!(refused.code, ErrorCode::IntegrityError);
    assert_eq!(
        refused.message,
        "derived mesh differs from the kernel result"
    );
    // A cold reader and the CLI refuse it the same way.
    let cold = {
        let store = f.store();
        std::thread::spawn(move || inspect(&store).unwrap_err())
            .join()
            .unwrap()
    };
    assert_eq!(cold, refused);
    let (code, body) = f.run("inspect-elements", &[]);
    assert_eq!(code, 2);
    assert_eq!(
        body.get("code").unwrap().string().unwrap(),
        "INTEGRITY_ERROR"
    );
}
