// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic inputs, MPL-2.0. No third-party layer or asset is used.

//! U17: USD is read in the CLI process by the layer engine, so a hostile layer in the project's
//! own store must end as a stable F04 code, never a panic or a hang. `commit-revision` verifies
//! blob digests but not USD content, so it can commit layers that no BeamCraft command would
//! author; every reading command must then refuse them the same way. Hostile foreign files are
//! covered by `reference.rs` (`attach-reference` refuses them before anything is stored).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use support::text;

use beamcraft_json::{Json, Limits};
use beamcraft_layer_engine::authored::schema_identity;
use beamcraft_model::digest::Sha256;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

const PROJECT: &str = "0199e7a0-0000-7000-8000-000000000001";
const WORKSPACE: &str = "0199e7a0-0000-7000-8000-000000000002";
const REVISION: &str = "0199e7a0-0000-7000-8000-0000000000a1";
const OP: &str = "0199e7a0-0000-7000-8000-0000000000b1";

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("u17-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "hostile-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        Self(root)
    }

    fn store(&self) -> String {
        self.0.join("store").to_str().unwrap().to_owned()
    }

    fn run(&self, args: &[&str]) -> (i32, Json, Duration) {
        let started = Instant::now();
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let elapsed = started.elapsed();
        let code = output
            .status
            .code()
            .expect("the CLI exits, it is not killed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains(self.0.to_str().unwrap()),
            "a refusal never echoes a path: {stdout}"
        );
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT)
            .unwrap_or_else(|_| panic!("stdout is bounded JSON: {stdout}"));
        (code, body, elapsed)
    }

    /// Stores `root` as the authored root layer, next to an empty derived layer, and commits
    /// them as the first revision.
    fn commit(&self, root: &[u8]) {
        let derived = b"#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\n";
        let store = self.store();
        let mut digests = Vec::new();
        for (name, bytes) in [("root.usda", root), ("derived.usda", &derived[..])] {
            let file = self.0.join(name);
            fs::write(&file, bytes).unwrap();
            let (code, body, _) = self.run(&[
                "put-blob",
                "--store",
                &store,
                "--file",
                file.to_str().unwrap(),
            ]);
            assert_eq!(code, 0, "{body:?}");
            digests.push(Sha256::digest(bytes).unwrap().to_string());
        }
        let schema = schema_identity().unwrap().digest.to_string();
        let layers = format!(
            "authored:root.usda:{},derived:derived.usda:{}",
            digests[0], digests[1]
        );
        let (code, body, _) = self.run(&[
            "commit-revision",
            "--store",
            &store,
            "--project-id",
            PROJECT,
            "--workspace-id",
            WORKSPACE,
            "--revision-id",
            REVISION,
            "--op-id",
            OP,
            "--generation",
            "1",
            "--base",
            "none",
            "--schema",
            &schema,
            "--root-layer",
            "root.usda",
            "--layers",
            &layers,
        ]);
        assert_eq!(code, 0, "{body:?}");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.file_name().is_some());
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn nested(depth: usize) -> Vec<u8> {
    let mut s = String::from("#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\n");
    for n in 0..depth {
        s.push_str(&format!("def Xform \"P{n}\" {{\n"));
    }
    s.push_str(&"}\n".repeat(depth));
    s.into_bytes()
}

#[test]
fn unsupported_authored_plates_refuse_even_malformed_partial_derived_opinions() {
    let source = Fixture::new();
    let id = "019a7bf0-0000-7000-8000-000000000005";
    let (code, _, _) = source.run(&[
        "create-plate",
        "--store",
        &source.store(),
        "--new-id",
        id,
        "--contour",
        "999000000,0,0;999393216,0,0.75;999393216,524288,0;999000000,524288,0.75",
        "--thickness",
        "10",
    ]);
    assert_eq!(code, 0);
    let (_, inspected, _) = source.run(&["inspect-elements", "--store", &source.store()]);
    let layer = inspected
        .get("revision")
        .unwrap()
        .get("manifest")
        .unwrap()
        .get("layers")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .find(|l| l.get("name").unwrap().string().unwrap() == "authored.usda")
        .unwrap();
    let digest = layer.get("digest").unwrap().string().unwrap();
    let bytes = fs::read(
        Path::new(&source.store())
            .join("blobs")
            .join(&digest[..2])
            .join(digest),
    )
    .unwrap();
    let baseline = Fixture::new();
    baseline.commit(&bytes);
    assert_eq!(
        baseline
            .run(&["inspect-elements", "--store", &baseline.store()])
            .0,
        0
    );
    for opinion in [
        "def Xform \"Geometry\" {}",
        "custom double bim:volumeMm3 = 1",
        "def Mesh \"OtherGeometry\" {}",
    ] {
        let target = Fixture::new();
        let mut layer = String::from_utf8(bytes.clone()).unwrap();
        layer.push_str(&format!("\nover \"World\" {{ over \"Steel\" {{ over \"Parts\" {{ over \"PRT_{}\" {{ {opinion}\n }} }} }} }}\n",id.replace('-', "")));
        target.commit(layer.as_bytes());
        let (code, body, _) = target.run(&["inspect-elements", "--store", &target.store()]);
        assert_eq!(code, 2, "{opinion}: {body:?}");
        let code = body.get("code").unwrap().string().unwrap();
        assert!(
            matches!(code, "INTEGRITY_ERROR" | "INVALID_ID"),
            "{opinion}: {code}"
        );
    }
}

#[test]
fn hostile_own_store_layers_give_stable_codes_in_process() {
    let mut binary = b"PXR-USDC\x00\x08\x00\x00\x00\x00\x00\x00".to_vec();
    binary.extend((0u8..=255).cycle().take(4096));
    // Every reader refuses the same stage for the same reason.
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "not a layer",
            b"this is not a USD layer".to_vec(),
            "INTEGRITY_ERROR",
        ),
        ("truncated", b"#usda 1.0\n(".to_vec(), "INTEGRITY_ERROR"),
        (
            "non-finite value",
            b"#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\ndef Xform \"P\" {custom double demo:value=1e999999999\n}\n".to_vec(),
            "NON_FINITE_NUMBER",
        ),
        ("deep nesting", nested(4096), "LIMIT_EXCEEDED"),
        ("corrupt binary", binary, "INTEGRITY_ERROR"),
    ];
    for (what, root, expected) in cases {
        let f = Fixture::new();
        f.commit(&root);
        let store = f.store();
        let out = f.0.join("out.bin");
        let csv = f.0.join("bom.csv");
        let ifc = f.0.join("model.ifc");
        for args in [
            vec!["inspect-elements", "--store", &store],
            vec!["definitions", "--store", &store],
            vec!["inspect-reference", "--store", &store],
            vec!["preview-numbering", "--store", &store],
            vec![
                "export-view",
                "--store",
                &store,
                "--out",
                out.to_str().unwrap(),
            ],
            vec![
                "export-bom",
                "--store",
                &store,
                "--out",
                csv.to_str().unwrap(),
            ],
            vec![
                "export-ifc",
                "--store",
                &store,
                "--out",
                ifc.to_str().unwrap(),
            ],
        ] {
            let (code, body, elapsed) = f.run(&args);
            assert_eq!(code, 2, "{what}, {}: {body:?}", args[0]);
            assert_eq!(text(&body, "outcome"), "rejected", "{what}, {}", args[0]);
            assert!(
                elapsed < Duration::from_secs(30),
                "{what}, {}: {elapsed:?}",
                args[0]
            );
            assert_eq!(text(&body, "code"), expected, "{what}, {}", args[0]);
        }
        assert!(!out.exists() && !csv.exists() && !ifc.exists(), "{what}");
    }
}
