// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Original fixtures with hand-declared semantics; no SDK/independent-reader claim.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_layer_engine::*;
use beamcraft_model::{
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
};
use std::{
    collections::BTreeSet,
    io::Cursor as IoCursor,
    time::{Duration, Instant},
};

const ID: &str = "0199e7a0-0000-7000-8000-000000000001";
const PATH: &str = "/World/Steel/Parts/PRT_0199e7a0000070008000000000000001";
const HEADER: &str = "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\")\n";
fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn digest(b: &[u8]) -> ContentDigest {
    Sha256::digest(b).unwrap()
}
fn layer(body: &str) -> String {
    format!("{HEADER}{body}\n")
}
fn model() -> String {
    layer(&format!(
        "def Xform \"World\" {{ def Scope \"Steel\" {{ def Scope \"Parts\" {{ def Xform \"PRT_0199e7a0000070008000000000000001\" {{ custom string bim:id = \"{ID}\"\n custom double demo:value = 77.25\n custom string future:label = \"Original\"\n }} }} }} }}"
    ))
}
fn asset(name: &str, bytes: impl Into<Vec<u8>>) -> Asset {
    let bytes = bytes.into();
    Asset {
        name: name.into(),
        digest: digest(&bytes),
        bytes,
    }
}
fn request(assets: Vec<Asset>) -> OpenRequest {
    let mut request = OpenRequest {
        identity: Identity {
            project: ID.into(),
            workspace: "0199e7a0-0000-7000-8000-000000000002".into(),
            revision: digest(b"original revision"),
            generation: 42,
        },
        root: assets.first().unwrap().name.clone(),
        owned_layers: [assets.first().unwrap().name.clone()].into(),
        assets,
        manifest: digest(b"pending"),
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets::FIRST_SLICE,
    };
    reseal(&mut request);
    request
}
fn reseal(request: &mut OpenRequest) {
    request.manifest = request.manifest_digest().unwrap();
}
fn error(request: OpenRequest) -> ErrorCode {
    Reader::default()
        .open(request, &control())
        .unwrap_err()
        .code
}

#[test]
fn real_usda_open_page_reopen_close_pins_full_identity_metadata_and_values() {
    let bytes = model().into_bytes();
    let mut r = Reader::default();
    let opened = r
        .open(request(vec![asset("root.usda", bytes.clone())]), &control())
        .unwrap();
    assert_eq!(opened.context.identity().generation, 42);
    assert_eq!(opened.context.identity().project, ID);
    assert_eq!(
        opened.context.identity().revision,
        digest(b"original revision")
    );
    assert_eq!(opened.capabilities.backend, BACKEND);
    assert!(!opened.capabilities.parser_contained);
    assert!(!opened.capabilities.mutations);
    assert_eq!(opened.meters_per_unit, 0.001);
    assert_eq!(opened.up_axis, "Z");
    let p = r.read(&opened.context, None, &control()).unwrap();
    assert_eq!(p.items.len(), 4);
    assert!(p.next.is_none());
    let prim = &p.items.last().unwrap();
    assert_eq!(prim.path, PATH);
    assert_eq!(prim.element_id.as_deref(), Some(ID));
    assert_eq!(prim.type_name.as_deref(), Some("Xform"));
    assert_eq!(
        prim.properties
            .iter()
            .find(|p| p.name == "demo:value")
            .unwrap()
            .value,
        Some(Value::Number(77.25))
    );
    assert_eq!(prim.sources[0].digest, digest(&bytes));
    assert!(prim.sources[0].owned);
    assert_eq!(
        r.refuse_write(&opened.context, PATH).unwrap_err().code,
        ErrorCode::UnsupportedCommand
    );
    let reopened = r
        .open(request(vec![asset("root.usda", bytes)]), &control())
        .unwrap();
    assert_eq!(
        r.read(&opened.context, None, &control()).unwrap_err().code,
        ErrorCode::StaleWorkspace
    );
    r.close(&reopened.context, &control()).unwrap();
    assert_eq!(
        r.read(&reopened.context, None, &control())
            .unwrap_err()
            .code,
        ErrorCode::StaleWorkspace
    );
}

#[test]
fn real_usdc_reads_original_binary_fixture_and_corruption_is_atomic() {
    let data = openusd::usda::parse(&model()).unwrap();
    let mut binary = IoCursor::new(Vec::new());
    openusd::usdc::CrateWriter::write(&data, &mut binary).unwrap();
    let bytes = binary.into_inner();
    let mut r = Reader::default();
    let opened = r
        .open(request(vec![asset("root.usdc", bytes.clone())]), &control())
        .unwrap();
    let before = r.read(&opened.context, None, &control()).unwrap().items;
    assert_eq!(before.last().unwrap().element_id.as_deref(), Some(ID));
    assert_eq!(
        before
            .last()
            .unwrap()
            .properties
            .iter()
            .find(|p| p.name == "demo:value")
            .unwrap()
            .value,
        Some(Value::Number(77.25))
    );
    for prefix in [0, 7, 31, bytes.len() - 1] {
        let invalid = request(vec![asset("bad.usdc", bytes[..prefix].to_vec())]);
        assert!(r.open(invalid, &control()).is_err());
        assert_eq!(
            r.read(&opened.context, None, &control()).unwrap().items,
            before
        );
    }
}

#[test]
fn binary_nonfinite_numbers_and_unchanged_expected_digest_fail_before_exposure() {
    let data = openusd::usda::parse(&model().replace("77.25", "1e999999999")).unwrap();
    let mut bytes = IoCursor::new(Vec::new());
    openusd::usdc::CrateWriter::write(&data, &mut bytes).unwrap();
    assert_eq!(
        error(request(vec![asset("root.usdc", bytes.into_inner())])),
        ErrorCode::NonFiniteNumber
    );
    for name in ["root.usda", "root.usdc"] {
        let bytes = if name.ends_with(".usda") {
            model().into_bytes()
        } else {
            usdc_bytes()
        };
        let mut req = request(vec![asset(name, bytes)]);
        // Keep the verified expected digest/manifest while changing or truncating actual bytes.
        req.assets[0].bytes[12] ^= 1;
        assert_eq!(error(req), ErrorCode::IntegrityError);
    }
}

#[test]
fn manifest_and_all_asset_digests_are_verified_before_any_parser() {
    let mut bad = request(vec![
        asset("root.usda", model()),
        asset("unused.usda", model()),
    ]);
    bad.assets[0].bytes = b"#usda 1.0\ndef {".to_vec();
    bad.assets[0].digest = digest(&bad.assets[0].bytes);
    bad.assets[1].bytes.push(0);
    reseal(&mut bad);
    assert_eq!(error(bad), ErrorCode::IntegrityError);
    let mut bad = request(vec![asset("root.usda", model())]);
    bad.identity.generation += 1;
    assert_eq!(error(bad), ErrorCode::IntegrityError);
    let mut bad = request(vec![asset("root.usda", model())]);
    bad.backend.schema = "unreviewed";
    reseal(&mut bad);
    assert_eq!(error(bad), ErrorCode::UnsupportedVersion);
}

#[test]
fn rejects_asset_aliases_escapes_and_network_without_opening_them() {
    for name in [
        "../escape.usda",
        "/root.usda",
        "C:/root.usda",
        "https://invalid.test/root.usda",
        "./root.usda",
        "a//root.usda",
        "a\\root.usda",
    ] {
        assert_eq!(
            error(request(vec![asset(name, model())])),
            ErrorCode::PermissionDenied
        );
    }
    for name in [
        "../foreign.usda",
        "https://invalid.test/f.usda",
        "C:\\f.usda",
    ] {
        let body = layer(&format!(
            "def \"Ref\" (references = @{name}@</Source>) {{}}"
        ));
        assert_eq!(
            error(request(vec![asset("root.usda", body)])),
            ErrorCode::PermissionDenied
        );
    }
}

#[test]
fn reference_provenance_overrides_and_foreign_write_refusal() {
    let root = layer(
        "def Xform \"World\" { def Scope \"Steel\" { def Scope \"Parts\" { def Xform \"PRT_0199e7a0000070008000000000000001\" (prepend references = @foreign.usda@</Source>) { custom double demo:value = 88 } } } }",
    );
    let foreign = layer(&format!(
        "def Xform \"Source\" {{ custom string bim:id = \"{ID}\"\ncustom double demo:value = 77.25 }}"
    ));
    let original = foreign.as_bytes().to_vec();
    let mut r = Reader::default();
    let opened = r
        .open(
            request(vec![
                asset("root.usda", root),
                asset("foreign.usda", foreign),
            ]),
            &control(),
        )
        .unwrap();
    let p = r.read(&opened.context, None, &control()).unwrap();
    assert_eq!(
        p.items
            .last()
            .unwrap()
            .properties
            .iter()
            .find(|p| p.name == "demo:value")
            .unwrap()
            .value,
        Some(Value::Number(88.0))
    );
    assert!(
        p.items
            .last()
            .unwrap()
            .sources
            .iter()
            .any(|s| s.layer == "foreign.usda"
                && s.path == "/Source"
                && !s.owned
                && s.digest == digest(&original))
    );
    assert_eq!(
        r.refuse_write(&opened.context, PATH).unwrap_err().code,
        ErrorCode::ForeignReadOnly
    );
    assert_eq!(opened.context.identity().generation, 42);
    assert_eq!(
        r.read(&opened.context, None, &control()).unwrap().items,
        p.items
    );
}

#[test]
fn contained_sublayers_strong_opinions_and_dependency_failures() {
    let root = "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\"\nsubLayers = [@model.usda@])\n";
    let mut r = Reader::default();
    let opened = r
        .open(
            request(vec![
                asset("dir/root.usda", root),
                asset("dir/model.usda", model()),
            ]),
            &control(),
        )
        .unwrap();
    assert_eq!(
        r.read(&opened.context, None, &control())
            .unwrap()
            .items
            .last()
            .unwrap()
            .path,
        PATH
    );
    assert_eq!(
        error(request(vec![asset("root.usda", root)])),
        ErrorCode::DependencyUnavailable
    );
    let self_cycle = root.replace("model.usda", "root.usda");
    assert_eq!(
        error(request(vec![asset("root.usda", self_cycle)])),
        ErrorCode::DependencyConflict
    );
    let a = root.replace("model.usda", "b.usda");
    let b = root.replace("model.usda", "a.usda");
    assert_eq!(
        error(request(vec![asset("a.usda", a), asset("b.usda", b)])),
        ErrorCode::DependencyConflict
    );
}

#[test]
fn nonfinite_and_finite_cap_failures_preserve_old_view() {
    let mut r = Reader::default();
    let opened = r
        .open(request(vec![asset("root.usda", model())]), &control())
        .unwrap();
    for literal in ["1e999999999", "nan", "inf", "-inf", "1e-99999"] {
        let invalid = model().replace("77.25", literal);
        assert_eq!(
            r.open(request(vec![asset("root.usda", invalid)]), &control())
                .unwrap_err()
                .code,
            ErrorCode::NonFiniteNumber
        );
        assert_eq!(
            r.read(&opened.context, None, &control())
                .unwrap()
                .items
                .last()
                .unwrap()
                .path,
            PATH
        );
    }
    assert!(
        Reader::default()
            .open(
                request(vec![asset(
                    "root.usda",
                    model().replace("77.25", "1000000000")
                )]),
                &control()
            )
            .is_ok()
    );
    assert_eq!(
        error(request(vec![asset(
            "root.usda",
            model().replace("77.25", "1000000000.1")
        )])),
        ErrorCode::LimitExceeded
    );
}

#[test]
fn paging_stale_cursors_and_cross_reader_contexts() {
    let text = layer("def Xform \"A\" {}\ndef Xform \"B\" {}\ndef Xform \"C\" {}");
    let mut input = request(vec![asset("root.usda", text.clone())]);
    input.budgets.page_items = 1;
    let mut r = Reader::default();
    let opened = r.open(input, &control()).unwrap();
    let mut cursor = None;
    let mut paths = Vec::new();
    let mut first = None;
    loop {
        let page = r
            .read(&opened.context, cursor.as_ref(), &control())
            .unwrap();
        paths.push(page.items[0].path.clone());
        if first.is_none() {
            first = page.next.clone();
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(paths, ["/A", "/B", "/C"]);
    let another = Reader::default();
    assert_eq!(
        another
            .read(&opened.context, None, &control())
            .unwrap_err()
            .code,
        ErrorCode::StaleWorkspace
    );
    let next = r
        .open(request(vec![asset("root.usda", text)]), &control())
        .unwrap();
    assert_eq!(
        r.read(&next.context, first.as_ref(), &control())
            .unwrap_err()
            .code,
        ErrorCode::StaleWorkspace
    );
}

#[test]
fn cancellation_and_backend_error_never_replace_old_view() {
    let mut r = Reader::default();
    let input = request(vec![asset("root.usda", model())]);
    let opened = r.open(input, &control()).unwrap();
    let cancelled = control();
    cancelled.cancellation.cancel();
    assert_eq!(
        r.open(request(vec![asset("root.usda", model())]), &cancelled)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        r.close(&opened.context, &cancelled).unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        r.read(&opened.context, None, &cancelled).unwrap_err().code,
        ErrorCode::Cancelled
    );
    let expired = Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now(),
    };
    assert_eq!(
        r.open(request(vec![asset("root.usda", model())]), &expired)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(
        r.open(
            request(vec![asset("root.usda", layer("def Xform {"))]),
            &control()
        )
        .is_err()
    );
    assert_eq!(
        r.read(&opened.context, None, &control())
            .unwrap()
            .items
            .last()
            .unwrap()
            .path,
        PATH
    );
}

#[test]
fn size_prim_value_array_depth_and_output_cap_plus_one() {
    let text = layer("def Xform \"A\" {}\ndef Xform \"B\" {}");
    let mut ok = request(vec![asset("root.usda", text.clone())]);
    ok.budgets.prims = 2;
    assert!(Reader::default().open(ok, &control()).is_ok());
    let mut bad = request(vec![asset("root.usda", text)]);
    bad.budgets.prims = 1;
    assert_eq!(error(bad), ErrorCode::LimitExceeded);
    let bytes = model().into_bytes();
    for delta in [0, 1] {
        let mut req = request(vec![asset("root.usda", bytes.clone())]);
        req.budgets.layer_bytes = bytes.len() - delta;
        assert_eq!(Reader::default().open(req, &control()).is_ok(), delta == 0);
    }
    let mut bad = request(vec![asset("root.usda", model())]);
    bad.budgets.total_bytes = 10;
    assert_eq!(error(bad), ErrorCode::LimitExceeded);
    let mut bad = request(vec![asset("root.usda", model())]);
    bad.budgets.values = 1;
    assert_eq!(error(bad), ErrorCode::LimitExceeded);
    let mut bad = request(vec![asset("root.usda", model())]);
    bad.budgets.snapshot_bytes = 1;
    assert_eq!(error(bad), ErrorCode::LimitExceeded);
    for n in [2, 3] {
        let arr = std::iter::repeat_n("1", n).collect::<Vec<_>>().join(",");
        let mut req = request(vec![asset(
            "root.usda",
            layer(&format!("def Xform \"A\" {{ double[] values = [{arr}] }}")),
        )]);
        req.budgets.array_items = 2;
        assert_eq!(Reader::default().open(req, &control()).is_ok(), n == 2);
    }
    let mut bad = request(vec![asset(
        "root.usda",
        layer("def Xform \"A\" { def Xform \"B\" {} }"),
    )]);
    bad.budgets.depth = 1;
    assert_eq!(error(bad), ErrorCode::LimitExceeded);
    let mut bad = request(vec![asset("root.usda", model())]);
    bad.budgets.layers = 0;
    assert_eq!(error(bad), ErrorCode::LimitExceeded);
}

#[test]
fn unsupported_arcs_and_full_id_faults_are_explicit() {
    for field in [
        "payload = @child.usda@</Source>",
        "inherits = </Source>",
        "specializes = </Source>",
        "instanceable = true",
    ] {
        let input = layer(&format!("def Xform \"A\" ({field}) {{}}"));
        assert_eq!(
            error(request(vec![asset("root.usda", input)])),
            ErrorCode::UnsupportedProperty
        );
    }
    assert_eq!(
        error(request(vec![asset(
            "root.usda",
            model().replace(ID, "short")
        )])),
        ErrorCode::InvalidId
    );
    assert_eq!(
        error(request(vec![asset(
            "root.usda",
            model().replace("000000000001\" {", "000000000003\" {")
        )])),
        ErrorCode::InvalidId
    );
    let mut req = request(vec![asset("root.usda", model())]);
    req.identity.project = "short".into();
    reseal(&mut req);
    assert_eq!(error(req), ErrorCode::InvalidId);
    let mut req = request(vec![asset("root.usda", model())]);
    req.owned_layers = BTreeSet::from(["absent.usda".into()]);
    reseal(&mut req);
    assert_eq!(error(req), ErrorCode::IntegrityError);
}

#[test]
fn metadata_unknown_content_blocks_and_absence_are_typed() {
    let text = "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\ncustomLayerData={string future=\"original\"\ndouble factor=1.25})\ndef Xform \"A\" { custom double absent\ncustom double blocked=None\ncustom string future:label=\"inert\"\n}";
    let mut reader = Reader::default();
    let opened = reader
        .open(request(vec![asset("root.usda", text)]), &control())
        .unwrap();
    assert_eq!(
        opened.metadata.get("customLayerData"),
        Some(&Value::Dictionary(
            [
                ("future".into(), Value::Text("original".into())),
                ("factor".into(), Value::Number(1.25))
            ]
            .into()
        ))
    );
    let page = reader.read(&opened.context, None, &control()).unwrap();
    let properties = &page.items[0].properties;
    assert_eq!(
        properties
            .iter()
            .find(|p| p.name == "absent")
            .unwrap()
            .value,
        None
    );
    assert_eq!(
        properties
            .iter()
            .find(|p| p.name == "blocked")
            .unwrap()
            .value,
        Some(Value::Block)
    );
    assert_eq!(
        properties
            .iter()
            .find(|p| p.name == "future:label")
            .unwrap()
            .value,
        Some(Value::Text("inert".into()))
    );
}

fn usdc_bytes() -> Vec<u8> {
    let data = openusd::usda::parse(&model()).unwrap();
    let mut buffer = IoCursor::new(Vec::new());
    openusd::usdc::CrateWriter::write(&data, &mut buffer).unwrap();
    buffer.into_inner()
}
fn word(bytes: &[u8], offset: usize) -> usize {
    u64::from_le_bytes(bytes.get(offset..offset + 8).unwrap().try_into().unwrap()) as usize
}
#[allow(clippy::unreachable)] // Original test fixture always has the six named sections.
fn section(bytes: &[u8], name: &[u8]) -> usize {
    let toc = word(bytes, 16);
    for n in 0..word(bytes, toc) {
        let at = toc + 8 + n * 32;
        let label = bytes
            .get(at..at + 16)
            .unwrap()
            .split(|c| *c == 0)
            .next()
            .unwrap();
        if label == name {
            return word(bytes, at + 16);
        }
    }
    unreachable!("original fixture section absent")
}

#[test]
fn usdc_declared_count_expansion_and_unsupported_nested_values_fail_closed() {
    let original = usdc_bytes();
    for name in [b"TOKENS".as_slice(), b"FIELDS", b"PATHS", b"SPECS"] {
        let mut bytes = original.clone();
        let at = section(&bytes, name);
        bytes
            .get_mut(at..at + 8)
            .unwrap()
            .copy_from_slice(&4097u64.to_le_bytes());
        assert_eq!(
            error(request(vec![asset("root.usdc", bytes)])),
            ErrorCode::LimitExceeded
        );
    }
    let mut bytes = original.clone();
    let at = section(&bytes, b"TOKENS") + 8;
    bytes
        .get_mut(at..at + 8)
        .unwrap()
        .copy_from_slice(&(4u64 * 1024 * 1024 + 1).to_le_bytes());
    assert_eq!(
        error(request(vec![asset("root.usdc", bytes)])),
        ErrorCode::LimitExceeded
    );
    let mut req = request(vec![asset("root.usdc", original)]);
    req.budgets.decoded_bytes = 1;
    assert_eq!(error(req), ErrorCode::LimitExceeded);
    let data=openusd::usda::parse("#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\ncustomLayerData={string nested=\"inert\"})\n").unwrap();
    let mut bytes = IoCursor::new(Vec::new());
    openusd::usdc::CrateWriter::write(&data, &mut bytes).unwrap();
    assert_eq!(
        error(request(vec![asset("root.usdc", bytes.into_inner())])),
        ErrorCode::UnsupportedProperty
    );
}

#[test]
fn layer_total_byte_and_authored_value_limits_are_inclusive() {
    let a = asset("a.usda", layer("def Xform \"A\" {}"));
    let z = asset("z.usda", layer("def Xform \"Z\" {}"));
    let total = a.bytes.len() + z.bytes.len();
    for n in [total - 1, total, total + 1] {
        let mut req = request(vec![a.clone(), z.clone()]);
        req.budgets.total_bytes = n;
        assert_eq!(Reader::default().open(req, &control()).is_ok(), n >= total);
    }
    for n in [1, 2, 3] {
        let mut req = request(vec![a.clone(), z.clone()]);
        req.budgets.layers = n;
        // Budget changes are independent of the committed manifest.
        assert_eq!(Reader::default().open(req, &control()).is_ok(), n >= 2);
    }
    // Pseudo-root: metres/unit, axis, child list. One prim: specifier and typeName = five.
    for n in [4, 5, 6] {
        let mut req = request(vec![a.clone()]);
        req.budgets.values = n;
        assert_eq!(Reader::default().open(req, &control()).is_ok(), n >= 5);
    }
}

#[test]
fn comment_forms_and_quoted_numbers_cannot_hide_underflow() {
    for literal in [
        "# note\r1e-99999\n",
        "/* # */ 1e-99999",
        "/* x */1e-99999",
        "\"1e-99999\"",
    ] {
        let invalid = model().replace("77.25", literal);
        let result = Reader::default().open(request(vec![asset("root.usda", invalid)]), &control());
        assert_eq!(
            result.map(|_| ()).unwrap_err().code,
            ErrorCode::NonFiniteNumber,
            "{literal:?}"
        );
    }
    // Ordinary strings, including numeric-looking ones that are finite, still open.
    let text = model().replace("\"Original\"", "\"12\"");
    assert!(
        Reader::default()
            .open(request(vec![asset("root.usda", text)]), &control())
            .is_ok()
    );
}

#[test]
fn usda_and_usdc_array_budgets_count_tuple_scalars_alike() {
    let body = "#usda 1.0
(metersPerUnit=0.001
upAxis=\"Z\")
def Xform \"A\" { custom double3[] v = [(1,2,3),(4,5,6)] }
";
    let data = openusd::usda::parse(body).unwrap();
    let mut bin = IoCursor::new(Vec::new());
    openusd::usdc::CrateWriter::write(&data, &mut bin).unwrap();
    for (name, bytes) in [
        ("root.usda", body.as_bytes().to_vec()),
        ("root.usdc", bin.into_inner()),
    ] {
        for (items, ok) in [(5, false), (6, true)] {
            let mut req = request(vec![asset(name, bytes.clone())]);
            req.budgets.array_items = items;
            let result = Reader::default().open(req, &control());
            if ok {
                assert!(result.is_ok(), "{name} {items}");
            } else {
                assert_eq!(result.unwrap_err().code, ErrorCode::LimitExceeded, "{name}");
            }
        }
    }
}

#[test]
fn kind_and_display_name_are_charged_to_the_snapshot_budget() {
    for key in ["kind", "displayName"] {
        let build = |value: &str| {
            let mut req = request(vec![asset(
                "root.usda",
                layer(&format!("def Xform \"A\" ({key} = \"{value}\") {{ }}")),
            )]);
            req.budgets.snapshot_bytes = 2000;
            req
        };
        assert!(
            Reader::default().open(build("x"), &control()).is_ok(),
            "{key}"
        );
        assert_eq!(
            error(build(&"x".repeat(5000))),
            ErrorCode::LimitExceeded,
            "{key}"
        );
    }
}

#[test]
fn raw_usd_text_has_an_inclusive_tightenable_budget_in_both_formats() {
    for cap in [1024, Budgets::MODEL.text_bytes] {
        for size in [cap, cap + 1] {
            let expected = "n".repeat(size);
            let text = layer(&format!(
                "def Xform \"A\" {{ custom string note = \"{expected}\" }}"
            ));
            let data = openusd::usda::parse(&text).unwrap();
            let mut binary = IoCursor::new(Vec::new());
            openusd::usdc::CrateWriter::write(&data, &mut binary).unwrap();
            for (name, bytes) in [
                ("root.usda", text.as_bytes().to_vec()),
                ("root.usdc", binary.into_inner()),
            ] {
                let mut req = request(vec![asset(name, bytes)]);
                req.budgets = Budgets::MODEL;
                req.budgets.text_bytes = cap;
                reseal(&mut req);
                let mut reader = Reader::default();
                let result = reader.open(req, &control());
                if size > cap {
                    assert_eq!(result.unwrap_err().code, ErrorCode::LimitExceeded);
                    continue;
                }
                let opened = result.unwrap();
                let page = reader.read(&opened.context, None, &control()).unwrap();
                assert_eq!(
                    page.items[0]
                        .properties
                        .iter()
                        .find(|p| p.name == "note")
                        .unwrap()
                        .value,
                    Some(Value::Text(expected.clone())),
                    "{name}, cap {cap}"
                );
            }
        }
    }
    for cap in [0, Budgets::MODEL.text_bytes + 1] {
        let mut req = request(vec![asset("root.usda", model())]);
        req.budgets.text_bytes = cap;
        assert_eq!(
            req.manifest_digest().unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }
}

#[test]
fn the_model_profile_is_the_default_ceiling_and_its_caps_hold_at_plus_one() {
    // The default ceiling is the model profile; nothing may be raised above it.
    let (d, m) = (Budgets::default(), Budgets::MODEL);
    assert_eq!(
        (
            d.layer_bytes,
            d.total_bytes,
            d.prims,
            d.values,
            d.array_items
        ),
        (
            m.layer_bytes,
            m.total_bytes,
            m.prims,
            m.values,
            m.array_items
        )
    );
    assert_eq!((m.layer_bytes, m.total_bytes), (33_554_432, 67_108_864));
    let mut above = request(vec![asset("root.usda", model())]);
    above.budgets = Budgets::MODEL;
    above.budgets.array_items += 1;
    assert_eq!(
        above.manifest_digest().unwrap_err().code,
        ErrorCode::LimitExceeded
    );
    // One element mesh at the engine blob caps fits; one scalar more does not.
    let at_cap = Budgets::MODEL.array_items;
    for n in [at_cap, at_cap + 1] {
        let arr = std::iter::repeat_n("1", n).collect::<Vec<_>>().join(",");
        let mut req = request(vec![asset(
            "root.usda",
            layer(&format!("def Xform \"A\" {{ int[] values = [{arr}] }}")),
        )]);
        req.budgets = Budgets::MODEL;
        reseal(&mut req);
        assert_eq!(Reader::default().open(req, &control()).is_ok(), n == at_cap);
    }
    // A layer one byte above the model ceiling is refused before it is parsed.
    let mut text = layer("def Xform \"A\" {}");
    text.push_str(&" ".repeat(Budgets::MODEL.layer_bytes + 1 - text.len()));
    let mut req = request(vec![asset("root.usda", text)]);
    req.budgets = Budgets::MODEL;
    reseal(&mut req);
    assert_eq!(error(req), ErrorCode::LimitExceeded);
}
