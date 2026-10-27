// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic source and driver, MPL-2.0. The oracle is a std-only Rust
// binary that shares no code with the layer engine.

//! Independent-reader oracle for R01: a revision with a reference wrapper, written by the real
//! `beamcraft` binary, is re-read from the committed layer text by an independent reader (the
//! `beamcraft-usd-oracle` process, with its own parser, composition and vector algebra). The
//! reader composes the foreign model below the owned wrapper, places its points exactly where
//! the engine's `source_point_to_world` says (a three-way agreement: reader, the engine
//! function and a closed form), sees no owned opinion below the arc holder, and finds the
//! foreign layer byte-identical to the file that was attached.
//!
//! Unlike the retired OpenUSD Python oracle, the reader runs everywhere and skips nowhere: the
//! independent-reader gate runs in every `cargo test`, locally and in CI.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use support::text;

use beamcraft_engine::reference::ReferenceRecord;
use beamcraft_json::{Json, Limits};
use beamcraft_model::command::Placement;
use beamcraft_model::digest::Sha256;
use beamcraft_model::id::ElementId;
use beamcraft_model::reference::{SourceFormat, SourcePin, UnitsInterpretation, UpAxis};
use std::fs;
use std::path::Path;
use std::process::Command;

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
            point3f[] points = [(0, 0, 0), (2, 0, 0), (2, 1, 0), (0, 1, 0), (0, 0, 3), (2, 0, 3), (2, 1, 3), (0, 1, 3)]
            int[] faceVertexCounts = [4, 4, 4, 4, 4, 4]
            int[] faceVertexIndices = [0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7]
        }
    }
}
"#;

fn run_binary(args: &[&str]) -> Json {
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{args:?}");
    beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap()
}

#[test]
fn the_reader_composes_the_reference_and_places_the_foreign_points_as_the_engine_says() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("r01-reader");
    let _ = fs::remove_dir_all(&base);
    let root = base.join(format!("run-{}", std::process::id()));
    fs::create_dir_all(root.join("store")).unwrap();
    fs::create_dir_all(root.join("layers").join("external")).unwrap();
    let store = root.join("store");
    let layers = root.join("layers");
    let source = root.join("site.usda");
    fs::write(&source, SITE).unwrap();
    let store_arg = store.to_str().unwrap();

    let attached = run_binary(&[
        "attach-reference",
        "--store",
        store_arg,
        "--source",
        source.to_str().unwrap(),
        "--name",
        "Oracle site",
    ]);
    let id = text(attached.get("reference").unwrap(), "id");
    // The source declares metres and Y up. The owner states exactly that, then places the wrapper
    // by a quarter turn about Z and a translation.
    run_binary(&[
        "correct-reference-units",
        "--store",
        store_arg,
        "--id",
        &id,
        "--meters-per-unit",
        "1",
        "--up-axis",
        "Y",
    ]);
    run_binary(&[
        "move-reference",
        "--store",
        store_arg,
        "--id",
        &id,
        "--translation",
        "10,20,30",
        "--rotation-z-deg",
        "90",
    ]);
    let inspected = run_binary(&["inspect-reference", "--store", store_arg]);
    let manifest = inspected
        .get("revision")
        .unwrap()
        .get("manifest")
        .unwrap()
        .get("layers")
        .unwrap()
        .array()
        .unwrap();
    for entry in manifest {
        let digest = text(entry, "digest");
        fs::copy(
            store.join("blobs").join(&digest[..2]).join(&digest),
            layers.join(text(entry, "name")),
        )
        .unwrap();
    }
    // The foreign layer the reader composes is byte-identical to the attached file.
    let digest = Sha256::digest(SITE.as_bytes()).unwrap().to_string();
    assert_eq!(
        fs::read(layers.join(format!("external/{digest}.usda"))).unwrap(),
        SITE.as_bytes()
    );

    // Expected world points: the engine's own mapping of the eight corners.
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let record = ReferenceRecord {
        id: ElementId::parse(&id).unwrap(),
        name: "Oracle site".into(),
        placement: Placement {
            translation_mm: [10.0, 20.0, 30.0],
            orientation_xyzw: [0.0, 0.0, half, half],
        },
        source: SourcePin {
            digest: Sha256::digest(SITE.as_bytes()).unwrap(),
            bytes: SITE.len() as u64,
            format: SourceFormat::Usda,
            default_prim: "Site".into(),
            version: 1,
            label: "site.usda".into(),
            declared_meters_per_unit: Some(1.0),
            declared_up_axis: Some(UpAxis::Y),
        },
        units: Some(UnitsInterpretation {
            meters_per_unit: 1.0,
            up_axis: UpAxis::Y,
        }),
    };
    let corners = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 3.0],
        [2.0, 0.0, 3.0],
        [2.0, 1.0, 3.0],
        [0.0, 1.0, 3.0],
    ];
    let world: Vec<[f64; 3]> = corners
        .iter()
        .map(|p| record.source_point_to_world(*p).unwrap())
        .collect();
    // Closed form for the far corner (2, 1, 3) m: scale to mm (2000, 1000, 3000), Y up to Z up
    // (2000, -3000, 1000), a quarter turn about Z (3000, 2000, 1000), then the translation.
    let far = world[6];
    for (got, want) in far.iter().zip([3010.0, 2020.0, 1030.0]) {
        assert!((got - want).abs() < 1e-9, "{far:?}");
    }
    let min = (0..3).map(|i| world.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min));
    let max = (0..3).map(|i| world.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max));
    let list = |values: &mut dyn Iterator<Item = f64>| {
        format!(
            "[{}]",
            values
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let expect = format!(
        r#"{{"id":"{id}","name":"Oracle site","digest":"{digest}","world_min":{},"world_max":{},"world_points":[{}]}}"#,
        list(&mut min.clone()),
        list(&mut max.clone()),
        world
            .iter()
            .map(|p| list(&mut p.iter().copied()))
            .collect::<Vec<_>>()
            .join(","),
    );
    let expect_path = layers.join("expect.json");
    fs::write(&expect_path, expect).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft-usd-oracle"))
        .arg("reference")
        .arg(layers.join("root.usda"))
        .arg(&expect_path)
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("READER REPORT: {report}");
    assert!(
        output.status.success(),
        "the independent reader disagreed: {}\n{report}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&root);
}
