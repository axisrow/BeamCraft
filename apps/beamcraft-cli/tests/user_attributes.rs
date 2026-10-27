// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! I04 typed units, exact integers, replay, local copy values and unknown preservation.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn ok(f: &StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c} {a:?}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
fn id(n: u8) -> String {
    format!("019a7bf0-0000-7000-8000-000000d000{n:02x}")
}
fn setup() -> (StoreFixture, String) {
    let f = StoreFixture::new("user-attributes");
    let p = text(
        ok(
            &f,
            "create-plate",
            &["--width", "100", "--length", "50", "--thickness", "10"],
        )
        .get("plate")
        .unwrap(),
        "element_id",
    );
    for (n, name, kind, extra) in [
        (1, "Text", "text", vec![]),
        (2, "Length", "number", vec!["--unit", "mm"]),
        (3, "Count", "integer", vec![]),
        (4, "Flag", "boolean", vec![]),
        (
            5,
            "Status",
            "choice",
            vec!["--choices", "New|Done", "--sets", "Review,Production"],
        ),
    ] {
        let i = id(n);
        let mut a = vec!["--new-id", &i, "--name", name, "--type", kind];
        a.extend(extra);
        ok(&f, "define-attribute", &a);
    }
    (f, p)
}
fn input(f: &StoreFixture, text: &str) -> String {
    let path = f.root.join("values.json");
    std::fs::write(&path, text).unwrap();
    path.to_str().unwrap().to_owned()
}
#[test]
fn every_type_roundtrips_and_replay_never_reapplies_after_later_edits() {
    let (f, p) = setup();
    let before = ok(&f, "inspect-elements", &[]);
    let path = input(
        &f,
        r#"{"Text":"hello","Length":{"value":12.5,"unit":"mm"},"Count":9223372036854775807,"Flag":false,"Status":"New"}"#,
    );
    let op = id(20);
    let a = ["--id", &p, "--values", &path, "--op-id", &op];
    ok(&f, "set-attributes", &a);
    let attrs = ok(&f, "inspect-attributes", &["--id", &p]);
    assert_eq!(attrs.get("attributes").unwrap().array().unwrap().len(), 5);
    let count = attrs
        .get("attributes")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .find(|v| text(v.get("definition").unwrap(), "name") == "Count")
        .unwrap();
    assert_eq!(
        count.get("value").unwrap(),
        &Json::Number(i64::MAX.to_string())
    );
    let head = ok(&f, "open-revision", &[]);
    assert_eq!(
        text(&ok(&f, "set-attributes", &a), "outcome"),
        "attributes_replayed"
    );
    assert_eq!(ok(&f, "open-revision", &[]), head);
    let after = ok(&f, "inspect-elements", &[]);
    let first = |j: &Json| j.get("plates").unwrap().array().unwrap()[0].clone();
    assert_eq!(
        first(&before).get("mesh_digest"),
        first(&after).get("mesh_digest")
    );
    assert_eq!(
        first(&before).get("quantities"),
        first(&after).get("quantities")
    );
    assert_eq!(
        after
            .get("other_element_count")
            .unwrap()
            .unsigned()
            .unwrap(),
        0
    );
    let instance = ok(&f, "copy-instance", &["--id", &p, "--offset", "100,0,0"]);
    let copy = text(instance.get("element").unwrap(), "element_id");
    assert_eq!(
        ok(&f, "inspect-attributes", &["--id", &copy]).get("attributes"),
        attrs.get("attributes")
    );
    let change = input(&f, r#"{"Status":"Done"}"#);
    ok(&f, "set-attributes", &["--id", &copy, "--values", &change]);
    assert_eq!(
        ok(&f, "inspect-attributes", &["--id", &p]).get("attributes"),
        attrs.get("attributes")
    );
    ok(
        &f,
        "remove-attributes",
        &["--id", &copy, "--attributes", "Status"],
    );
    assert_eq!(
        ok(&f, "inspect-attributes", &["--id", &copy])
            .get("attributes")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        4
    );
    let latest = input(&f, r#"{"Status":"Done"}"#);
    ok(&f, "set-attributes", &["--id", &p, "--values", &latest]);
    let held = ok(&f, "open-revision", &[]);
    input(
        &f,
        r#"{"Text":"hello","Length":{"value":12.5,"unit":"mm"},"Count":9223372036854775807,"Flag":false,"Status":"New"}"#,
    );
    assert_eq!(
        text(&ok(&f, "set-attributes", &a), "outcome"),
        "attributes_replayed"
    );
    assert_eq!(ok(&f, "open-revision", &[]), held);
}
#[test]
fn wrong_units_types_choices_and_reuse_are_refused_atomically() {
    let (f, p) = setup();
    let before = ok(&f, "open-revision", &[]);
    for value in [
        r#"{"Length":{"value":1,"unit":"m"}}"#,
        r#"{"Flag":"false"}"#,
        r#"{"Status":"Other"}"#,
        r#"{"Count":1.5}"#,
        r#"{"Missing":1}"#,
    ] {
        let path = input(&f, value);
        assert_eq!(
            f.run("set-attributes", &["--id", &p, "--values", &path]).0,
            2
        );
        assert_eq!(ok(&f, "open-revision", &[]), before);
    }
    let path = input(&f, r#"{"Status":"New"}"#);
    let op = id(21);
    ok(
        &f,
        "set-attributes",
        &["--id", &p, "--values", &path, "--op-id", &op],
    );
    let head = ok(&f, "open-revision", &[]);
    input(&f, r#"{"Status":"Done"}"#);
    assert_eq!(
        f.run(
            "set-attributes",
            &["--id", &p, "--values", &path, "--op-id", &op]
        )
        .0,
        2
    );
    assert_eq!(ok(&f, "open-revision", &[]), head);
}
#[test]
fn unknown_user_properties_survive_known_edits_and_are_not_silently_lost_on_copy() {
    use beamcraft_host::{
        ids::IdSource,
        revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
        storage::BlobStore,
    };
    let (f, p) = setup();
    let store = RevisionStore::new(BlobStore::new(f.root.join("store")).unwrap());
    let head = store.open().unwrap();
    let content = head.manifest().content();
    let mut ids = IdSource::system();
    let mut entries = Vec::new();
    for e in content.entries() {
        let mut data = String::from_utf8(head.layer_bytes(e.name()).unwrap()).unwrap();
        if e.name() == "authored.usda" {
            let marker = format!("def Xform \"PRT_{}\"", p.replace('-', ""));
            let at = data.find(&marker).unwrap();
            let (before, part) = data.split_at(at);
            data = before.to_owned()+&part.replacen(
                "custom token bim:geometryMode",
                "custom string user:legacy = \"keep\"\n            custom token bim:geometryMode",
                1,
            );
        }
        store.blobs().publish(data.as_bytes()).unwrap();
        entries.push(LayerEntry::from_bytes(e.name(), e.role(), data.as_bytes()).unwrap());
    }
    store
        .commit_with(
            &RevisionDraft {
                project: content.project().clone(),
                workspace: content.workspace().clone(),
                revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
                op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
                workspace_generation: content.workspace_generation() + 1,
                schema: content.schema(),
                root_layer: content.root_layer().to_owned(),
                entries,
            },
            &ExpectedBase::Revision(head.info().revision().clone()),
            None,
            &mut NoFault,
        )
        .unwrap();
    let path = input(&f, r#"{"Status":"New"}"#);
    ok(&f, "set-attributes", &["--id", &p, "--values", &path]);
    let inspect = ok(&f, "inspect-attributes", &["--id", &p]);
    assert!(
        inspect
            .get("attributes")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .any(|v| text(v, "property") == "user:legacy"
                && v.get("value").unwrap().string().unwrap() == "keep")
    );
    let before = ok(&f, "open-revision", &[]);
    let (code, result) = f.run("copy-element", &["--id", &p]);
    assert_eq!(code, 2, "{result:?}");
    assert_eq!(text(&result, "code"), "DEPENDENCY_UNAVAILABLE");
    assert_eq!(ok(&f, "open-revision", &[]), before);
}

#[test]
fn one_attribute_patch_is_one_durable_undo_and_redo_unit() {
    let (f, p) = setup();
    let dir = f.root.join("draft");
    ok(
        &f,
        "create-workspace",
        &["--workspace-dir", dir.to_str().unwrap()],
    );
    let path = input(&f, r#"{"Status":"Done","Count":7}"#);
    let (code, j) = StoreFixture::run_in(
        dir.to_str().unwrap(),
        "set-attributes",
        &["--id", &p, "--values", &path],
    );
    assert_eq!(code, 0, "{j:?}");
    let inspect =
        || StoreFixture::run_in(dir.to_str().unwrap(), "inspect-attributes", &["--id", &p]).1;
    assert_eq!(
        inspect().get("attributes").unwrap().array().unwrap().len(),
        2
    );
    for (command, count) in [("undo-workspace", 0), ("redo-workspace", 2)] {
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .arg(command)
            .arg("--workspace-dir")
            .arg(&dir)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
        assert_eq!(
            inspect().get("attributes").unwrap().array().unwrap().len(),
            count
        );
    }
}
