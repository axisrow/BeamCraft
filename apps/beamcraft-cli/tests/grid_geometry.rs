// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Derived grid guides remain readable standard USD and never become engineering bodies.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
const G: &str = "01990000-0000-7000-8000-000000004001";
fn ok(f: &support::StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn create(f: &support::StoreFixture) {
    ok(
        f,
        "create-grid",
        &[
            "--name",
            "Guides",
            "--new-id",
            G,
            "--u-expression",
            "0 6000",
            "--v-expression",
            "0",
            "--origin",
            "1000,2000",
            "--rotation",
            "90",
        ],
    );
}
fn layers(f: &support::StoreFixture) -> (String, String) {
    let store = RevisionStore::new(BlobStore::new(f.root.join("store")).unwrap());
    let rev = store.open().unwrap();
    (
        String::from_utf8(rev.layer_bytes("authored.usda").unwrap()).unwrap(),
        String::from_utf8(rev.layer_bytes("derived.usda").unwrap()).unwrap(),
    )
}
#[test]
fn guides_are_derived_only_and_relabel_is_geometry_inert() {
    let f = support::StoreFixture::new("t01-guide");
    create(&f);
    let (a, d) = layers(&f);
    assert!(!a.contains("def Mesh"));
    assert!(d.contains("purpose = \"guide\""));
    assert!(!d.contains("bim:massKg"));
    let view = ok(&f, "inspect-elements", &[]);
    assert!(view.get("plates").unwrap().array().unwrap().is_empty());
    assert!(view.get("members").unwrap().array().unwrap().is_empty());
    let snap = ok(
        &f,
        "snap",
        &[
            "--point",
            "1000,2000,0",
            "--radius",
            "100",
            "--kinds",
            "vertex,axis,axis_end",
        ],
    );
    assert!(snap.get("candidates").unwrap().array().unwrap().is_empty());
    ok(
        &f,
        "relabel-grid-line",
        &["--id", G, "--family", "u", "--labels", "X Y"],
    );
    assert_eq!(layers(&f).1, d);
    ok(
        &f,
        "update-grid",
        &["--id", G, "--u-expression", "0 7000", "--u-labels", "X Y"],
    );
    assert_ne!(layers(&f).1, d);
    ok(&f, "delete-element", &["--id", G]);
    assert!(!layers(&f).1.contains("def Mesh"));
}
#[test]
fn standard_usd_guide_reopens_in_openusd_when_available() {
    let f = support::StoreFixture::new("t01-guide-pxr");
    create(&f);
    let Some(python) = std::env::var_os("BEAMCRAFT_PXR_PYTHON") else {
        assert!(
            std::env::var_os("BEAMCRAFT_REQUIRE_PXR").is_none(),
            "OpenUSD oracle was required"
        );
        return;
    };
    let root = f.root.join("usd");
    std::fs::create_dir(&root).unwrap();
    let store = RevisionStore::new(BlobStore::new(f.root.join("store")).unwrap());
    let rev = store.open().unwrap();
    for e in rev.manifest().content().entries() {
        let path = root.join(e.name());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, rev.layer_bytes(e.name()).unwrap()).unwrap();
    }
    let script = r#"from pxr import Usd, UsdGeom
import sys
stage=Usd.Stage.Open(sys.argv[1]); assert stage
meshes=[p for p in stage.Traverse() if p.IsA(UsdGeom.Mesh)]
assert len(meshes)==1
mesh=UsdGeom.Mesh(meshes[0]); assert mesh.GetPurposeAttr().Get()=='guide'
assert len(mesh.GetPointsAttr().Get())==3*32
assert len(mesh.GetFaceVertexCountsAttr().Get())==3*18
assert mesh.GetSubdivisionSchemeAttr().Get()=='none'
assert mesh.GetNormalsInterpolation()=='uniform'
box=UsdGeom.BBoxCache(Usd.TimeCode.Default(),['guide']).ComputeWorldBound(mesh.GetPrim()).ComputeAlignedRange()
assert not box.IsEmpty()
assert abs(box.GetMin()[2]+25)<1e-5 and abs(box.GetMax()[2]-25)<1e-5
assert not mesh.GetPrim().HasAttribute('bim:massKg')
"#;
    let output = std::process::Command::new(python)
        .args(["-c", script])
        .arg(root.join("root.usda"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
