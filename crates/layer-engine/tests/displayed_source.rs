// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original generated synthetic fixtures, MPL-2.0; covered by ATTRIBUTION.md's
// test-data row. The "foreign sources" are invented text, not third-party content.

//! A displayed (foreign, non-owned) source is admitted without being validated: what the profile
//! cannot represent is left out of the snapshot, never a reason to refuse the source. The project's
//! own layers keep the strict profile.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use beamcraft_layer_engine::*;
use beamcraft_model::{
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
};
use openusd::usdc::CrateWriter;
use std::{
    io::Cursor,
    time::{Duration, Instant},
};

const FOREIGN: &str = "external/source.usda";
const FOREIGN_BINARY: &str = "external/source.usdc";

const ROOT: &str = "#usda 1.0
(
    defaultPrim = \"World\"
    metersPerUnit = 0.001
    upAxis = \"Z\"
)
def Xform \"World\"
{
    def \"Source\" (references = @LAYER@</Site>)
    {
    }
}
";

/// Everything the strict profile refuses, around one mesh that must survive.
const AWKWARD: &str = r#"#usda 1.0
(
    defaultPrim = "Site"
    metersPerUnit = 0.01
    upAxis = "Y"
    customLayerData = {
        string note = 'single quoted'
        dictionary nested = {
            string deeper = "x"
        }
    }
)
def Xform "Site" (
    apiSchemas = ["MaterialBindingAPI"]
    assetInfo = { asset identifier = @omniverse://server/site.usd@ }
    customData = { string kind = "scene" }
    instanceable = false
    variantSets = ["look"]
    prepend payload = @./SubUSDs/missing.usd@
    prepend references = @https://example.invalid/other.usd@</Other>
)
{
    matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (5, 6, 7, 1) )
    quatf xformOp:orient = (1, 0, 0, 0)
    uniform token[] xformOpOrder = ["xformOp:transform"]
    custom asset texture = @https://example.invalid/t.png@
    custom string single = 'single quoted'
    custom string triple = """two
lines"""
    custom double beyond = 1e300
    custom double endless = inf
    custom float[] weights = [1, 2, 3]
    def Mesh "Body"
    {
        point3f[] points = [(0, 0, 0), (2, 0, 0), (2, 1, 0), (0, 1, 0)]
        int[] faceVertexCounts = [4]
        int[] faceVertexIndices = [0, 1, 2, 3]
        uniform token subdivisionScheme = "none"
    }
}
"#;

fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}

fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

fn asset(name: &str, bytes: &[u8]) -> Asset {
    Asset {
        name: name.into(),
        digest: digest(bytes),
        bytes: bytes.to_vec(),
    }
}

fn request(assets: Vec<Asset>, budgets: Budgets) -> OpenRequest {
    let mut request = OpenRequest {
        identity: Identity {
            project: "0199e7a0-0000-7000-8000-000000000001".into(),
            workspace: "0199e7a0-0000-7000-8000-000000000002".into(),
            revision: digest(b"displayed source"),
            generation: 1,
        },
        root: "root.usda".into(),
        assets,
        owned_layers: ["root.usda".to_owned()].into(),
        manifest: digest(b"placeholder"),
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets,
    };
    request.manifest = request.manifest_digest().unwrap();
    request
}

/// Opens a project root that references `bytes` as a non-owned layer named `name`.
fn open_source(name: &str, bytes: &[u8], budgets: Budgets) -> Result<(Reader, Opened), Diagnostic> {
    let root = ROOT.replace("LAYER", name);
    let mut reader = Reader::default();
    let opened = reader.open(
        request(
            vec![asset("root.usda", root.as_bytes()), asset(name, bytes)],
            budgets,
        ),
        &control(),
    )?;
    Ok((reader, opened))
}

fn prims(reader: &Reader, opened: &Opened) -> Vec<Prim> {
    let mut result = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader
            .read(&opened.context, cursor.as_ref(), &control())
            .unwrap();
        result.extend(page.items);
        cursor = page.next;
        if cursor.is_none() {
            return result;
        }
    }
}

fn prim<'a>(prims: &'a [Prim], path: &str) -> &'a Prim {
    prims
        .iter()
        .find(|p| p.path == path)
        .unwrap_or_else(|| panic!("{path} is not in the snapshot"))
}

fn value<'a>(prim: &'a Prim, name: &str) -> Option<&'a Value> {
    prim.properties
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| p.value.as_ref())
}

/// The scalars of an array value, flattened (a `point3f[]` is an array of triples).
fn numbers(value: Option<&Value>) -> Vec<f64> {
    fn flatten(value: &Value, out: &mut Vec<f64>) {
        match value {
            Value::Number(n) => out.push(*n),
            Value::Array(items) => items.iter().for_each(|item| flatten(item, out)),
            other => panic!("not numeric: {other:?}"),
        }
    }
    let mut out = Vec::new();
    flatten(value.unwrap(), &mut out);
    out
}

fn binary(text: &str) -> Vec<u8> {
    let data = openusd::usda::parse(text).unwrap();
    let mut buffer = Cursor::new(Vec::new());
    CrateWriter::write(&data, &mut buffer).unwrap();
    buffer.into_inner()
}

#[test]
fn what_the_profile_cannot_represent_is_left_out_and_the_geometry_is_kept() {
    let (reader, opened) = open_source(FOREIGN, AWKWARD.as_bytes(), Budgets::default()).unwrap();
    let all = prims(&reader, &opened);
    let site = prim(&all, "/World/Source");
    assert_eq!(site.sources.iter().filter(|s| s.owned).count(), 1);
    // The geometry and the placement survive, a matrix and a float quaternion included.
    let body = prim(&all, "/World/Source/Body");
    assert_eq!(numbers(value(body, "points")).len(), 12);
    assert_eq!(
        numbers(value(site, "xformOp:transform")),
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 6.0, 7.0, 1.0
        ]
    );
    assert_eq!(numbers(value(site, "xformOp:orient")), [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(
        value(site, "single"),
        Some(&Value::Text("single quoted".into()))
    );
    assert_eq!(numbers(value(site, "weights")), [1.0, 2.0, 3.0]);
    // Values the snapshot cannot hold cost their property, nothing else.
    for gone in ["texture", "beyond", "endless"] {
        assert!(
            !site.properties.iter().any(|p| p.name == gone),
            "{gone} must not be in the snapshot"
        );
    }
    // The internal reference composes; arcs to assets that were never supplied stay inert.
    assert!(all.iter().any(|p| p.path == "/World/Source/Body"));
}

#[test]
fn a_binary_displayed_source_is_admitted_the_same_way() {
    let bytes = binary(&AWKWARD.replace('\'', "\"").replace("\"\"\"", "\""));
    let (reader, opened) = open_source(FOREIGN_BINARY, &bytes, Budgets::default()).unwrap();
    let all = prims(&reader, &opened);
    assert_eq!(
        numbers(value(prim(&all, "/World/Source/Body"), "points")).len(),
        12
    );
}

#[test]
fn a_long_string_full_of_escapes_does_not_exhaust_the_stack() {
    // The upstream text parser recurses once per escape sequence; this runs on a default test
    // thread, which is far below what 30,000 escapes need.
    let escaped = "\\\"".repeat(30_000);
    let text = format!(
        "#usda 1.0\n(\n    defaultPrim = \"Site\"\n)\ndef Xform \"Site\"\n{{\n    custom string baked = \"{escaped}\"\n}}\n"
    );
    let (reader, opened) = open_source(FOREIGN, text.as_bytes(), Budgets::default()).unwrap();
    let all = prims(&reader, &opened);
    assert!(matches!(
        value(prim(&all, "/World/Source"), "baked"),
        Some(Value::Text(t)) if t.len() == 30_000
    ));
}

#[test]
fn a_displayed_source_may_nest_deeper_than_a_project_stage() {
    let depth = 100;
    let mut text =
        String::from("#usda 1.0\n(\n    defaultPrim = \"Site\"\n)\ndef Xform \"Site\"\n{\n");
    for level in 0..depth {
        text.push_str(&format!("def Xform \"N{level}\"\n{{\n"));
    }
    text.push_str(&"}\n".repeat(depth));
    text.push_str("}\n");
    let (reader, opened) = open_source(FOREIGN, text.as_bytes(), Budgets::default()).unwrap();
    assert_eq!(opened.prim_count, depth + 2);
    assert_eq!(prims(&reader, &opened).len(), depth + 2);
}

#[test]
fn the_size_ceilings_and_the_project_profile_still_bind() {
    // A displayed source is still bounded: the prim ceiling refuses it.
    let tight = Budgets {
        prims: 3,
        ..Budgets::default()
    };
    let error = open_source(FOREIGN, AWKWARD.as_bytes(), tight)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::LimitExceeded);
    // An owned layer keeps the strict profile: the same content is refused there.
    let strict = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\ndef Xform \"World\" (variantSets = [\"look\"])\n{\n}\n";
    let mut reader = Reader::default();
    let error = reader
        .open(
            request(
                vec![asset("root.usda", strict.as_bytes())],
                Budgets::default(),
            ),
            &control(),
        )
        .map(|_| ())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnsupportedProperty);
    // So does a dependency an owned layer declares and does not supply.
    let root = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\ndef Xform \"World\" (references = @absent.usda@</Site>)\n{\n}\n";
    let error = Reader::default()
        .open(
            request(
                vec![asset("root.usda", root.as_bytes())],
                Budgets::default(),
            ),
            &control(),
        )
        .map(|_| ())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DependencyUnavailable);
}

#[test]
fn urls_and_escaping_identifiers_in_a_displayed_source_are_inert() {
    for url in [
        "https://example.invalid/model.usd",
        "omniverse://server/model.usd",
        "file:///tmp/model.usd",
        "../up/model.usd",
        "/abs/model.usd",
    ] {
        let text = format!(
            "#usda 1.0\n(\n    defaultPrim = \"Site\"\n    subLayers = [@{url}@]\n)\ndef Xform \"Site\" (payload = @{url}@</X>)\n{{\n    def Scope \"Kept\" {{}}\n}}\n"
        );
        let info = inspect_foreign_layer(text.as_bytes(), Budgets::default(), &control()).unwrap();
        assert_eq!(info.prim_count, 2, "{url}");
        let (reader, opened) = open_source(FOREIGN, text.as_bytes(), Budgets::default()).unwrap();
        let all = prims(&reader, &opened);
        assert_eq!(all.len(), 3, "{url}");
        assert!(all.iter().any(|p| p.path == "/World/Source/Kept"), "{url}");
    }
}
