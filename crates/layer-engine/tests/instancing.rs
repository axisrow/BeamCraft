// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U11 instancing of shared geometry: the composed `instanceable` reference form.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use openusd::ar;
use openusd::sdf;
use openusd::usd;
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

const LAYER: &str = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    defaultPrim = \"World\"\n)\n\ndef Xform \"World\"\n{\n    def Scope \"Definitions\"\n    {\n        def Mesh \"GEO_x\"\n        {\n            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]\n            int[] faceVertexCounts = [3]\n            int[] faceVertexIndices = [0, 1, 2]\n            custom string bim:shapeKey = \"aa\"\n        }\n    }\n\n    def Xform \"E1\"\n    {\n        def Mesh \"Geometry\" (\n            instanceable = true\n            kind = \"subcomponent\"\n            references = </World/Definitions/GEO_x>\n        )\n        {\n            custom double bim:massKg = 5\n        }\n    }\n}\n";

struct Memory {
    assets: BTreeMap<String, Arc<[u8]>>,
}

impl ar::Resolver for Memory {
    fn create_identifier(&self, asset: &str, _anchor: Option<&ar::ResolvedPath>) -> String {
        asset.to_string()
    }
    fn resolve(&self, asset: &str) -> Option<ar::ResolvedPath> {
        self.assets
            .contains_key(asset)
            .then(|| ar::ResolvedPath::new(asset))
    }
    fn resolve_for_new_asset(&self, _: &str) -> Option<ar::ResolvedPath> {
        None
    }
    fn open_asset(&self, resolved: &ar::ResolvedPath) -> io::Result<Box<dyn ar::Asset>> {
        let bytes = resolved
            .to_str()
            .and_then(|s| self.assets.get(s))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "asset not supplied"))?;
        Ok(Box::new(MemoryAsset(io::Cursor::new(bytes.clone()))))
    }
    fn get_modification_timestamp(
        &self,
        _: &str,
        _: &ar::ResolvedPath,
    ) -> Option<std::time::SystemTime> {
        None
    }
    fn identity(&self) -> String {
        "spike".into()
    }
}

struct MemoryAsset(io::Cursor<Arc<[u8]>>);

impl io::Read for MemoryAsset {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.0, out)
    }
}
impl io::Seek for MemoryAsset {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        io::Seek::seek(&mut self.0, pos)
    }
}
impl ar::Asset for MemoryAsset {
    fn size(&self) -> io::Result<u64> {
        Ok(self.0.get_ref().len() as u64)
    }
}

#[test]
fn internal_reference_and_instanceable_compose() {
    let bytes: Arc<[u8]> = LAYER.as_bytes().into();
    let resolver = Memory {
        assets: BTreeMap::from([("root.usda".to_string(), bytes)]),
    };
    let stage = usd::Stage::builder()
        .resolver(resolver)
        .open("root.usda")
        .unwrap();
    for error in stage.composition_errors() {
        println!("composition error: {error}");
    }
    let geometry = stage.prim("/World/E1/Geometry").unwrap();
    println!("type: {:?}", geometry.type_name());
    println!(
        "instanceable: {:?}",
        geometry.get_metadata::<sdf::Value>("instanceable")
    );
    let points = stage
        .attribute("/World/E1/Geometry.points")
        .unwrap()
        .get::<sdf::Value>()
        .unwrap();
    println!("points: {points:?}");
    let shape_key = stage
        .attribute("/World/E1/Geometry.bim:shapeKey")
        .unwrap()
        .get::<sdf::Value>()
        .unwrap();
    println!("shapeKey: {shape_key:?}");
    let sources: Vec<String> = geometry
        .prim_stack()
        .unwrap()
        .iter()
        .map(|site| format!("{} @ {}", site.path, site.layer))
        .collect();
    println!("sites: {sources:#?}");
    let type_name = geometry
        .type_name()
        .unwrap()
        .expect("type")
        .as_str()
        .to_string();
    assert_eq!(type_name, "Mesh");
}
