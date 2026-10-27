// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V14 direct-query/presentation parity, typed predicates, missing data and personal sets.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn model(dir: &std::path::Path, c: &str, a: &[&str]) -> Json {
    let (code, j) = StoreFixture::run_in(dir.to_str().unwrap(), c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn personal(dir: &std::path::Path, c: &str, a: &[&str]) -> (i32, Json) {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg(c)
        .arg("--workspace-dir")
        .arg(dir)
        .args(a)
        .output()
        .unwrap();
    (
        o.status.code().unwrap(),
        beamcraft_json::decode(&o.stdout, &beamcraft_json::Limits::DEFAULT).unwrap(),
    )
}
fn ok(dir: &std::path::Path, c: &str, a: &[&str]) -> Json {
    let (code, j) = personal(dir, c, a);
    assert_eq!(code, 0, "{c}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
fn file(f: &StoreFixture, s: &str) -> String {
    let p = f.root.join("rule.json");
    std::fs::write(&p, s).unwrap();
    p.to_str().unwrap().to_owned()
}
const STATUS: &str = "019a7bf0-0000-7000-8000-000000e00001";
const FILTER: &str = "019a7bf0-0000-7000-8000-000000e00002";
#[test]
fn direct_queries_and_bound_view_use_identical_rules_without_model_writes() {
    let f = StoreFixture::new("element-filters");
    let root = f.root.join("store");
    let (code, p) = f.run(
        "create-plate",
        &[
            "--width",
            "100",
            "--length",
            "50",
            "--thickness",
            "10",
            "--name",
            "Alpha",
            "--class",
            "7",
        ],
    );
    assert_eq!(code, 0, "{p:?}");
    let a = text(p.get("plate").unwrap(), "element_id");
    model(
        &root,
        "create-plate",
        &[
            "--width",
            "100",
            "--length",
            "50",
            "--thickness",
            "10",
            "--name",
            "Beta",
            "--class",
            "2",
        ],
    );
    model(
        &root,
        "define-attribute",
        &[
            "--name",
            "Status",
            "--type",
            "choice",
            "--choices",
            "New|Done",
            "--new-id",
            STATUS,
        ],
    );
    let values = file(&f, r#"{"Status":"New"}"#);
    model(&root, "set-attributes", &["--id", &a, "--values", &values]);
    let dir = f.root.join("draft");
    model(
        &root,
        "create-workspace",
        &["--workspace-dir", dir.to_str().unwrap()],
    );
    let before = model(&dir, "inspect-elements", &[]);
    let rule = file(
        &f,
        r#"{"all":[{"field":"type","op":"eq","value":"plate"},{"any":[{"field":"class","op":"eq","value":7},{"field":"name","op":"starts-with","value":"Z"}]},{"field":"attribute","attribute":"Status","op":"ne","value":"Done"}]}"#,
    );
    let query = model(&dir, "query-elements", &["--rule", &rule]);
    assert_eq!(
        query.get("elements").unwrap().array().unwrap(),
        &[Json::text(&a)]
    );
    ok(
        &dir,
        "save-filter",
        &["--id", FILTER, "--name", "Open", "--rule", &rule],
    );
    let view = ok(&dir, "create-view", &["--kind", "3d", "--name", "Filtered"]);
    let vid = text(view.get("view").unwrap(), "id");
    ok(&dir, "update-view", &["--id", &vid, "--filter", FILTER]);
    assert_eq!(
        model(&dir, "query-elements", &["--filter", FILTER]).get("elements"),
        query.get("elements")
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg("export-view")
        .arg("--store")
        .arg(&dir)
        .args(["--view", &vid, "--out", "-"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let mut offset = 8;
    let mut display = Json::Null;
    for i in 0..4 {
        let n = u32::from_le_bytes(out.stdout[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if i == 3 {
            display = beamcraft_json::decode(
                &out.stdout[offset..offset + n],
                &beamcraft_json::Limits::DEFAULT,
            )
            .unwrap();
        }
        offset += n;
    }
    assert_eq!(display.get("visible_elements"), query.get("elements"));
    assert_eq!(model(&dir, "inspect-elements", &[]), before);
    let bytes = std::fs::read(dir.join("views.usda")).unwrap();
    assert_eq!(personal(&dir, "delete-filter", &["--id", FILTER]).0, 2);
    assert_eq!(std::fs::read(dir.join("views.usda")).unwrap(), bytes);
    ok(&dir, "update-view", &["--id", &vid, "--filter", "none"]);
    ok(&dir, "delete-filter", &["--id", FILTER]);
    assert!(
        ok(&dir, "list-filters", &[])
            .get("filters")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn unknown_fields_attributes_and_incompatible_operators_refuse_with_a_pinned_page() {
    let f = StoreFixture::new("filter-refusals");
    let dir = f.root.join("store");
    for class in ["1", "2"] {
        model(
            &dir,
            "create-plate",
            &[
                "--width",
                "100",
                "--length",
                "50",
                "--thickness",
                "10",
                "--class",
                class,
            ],
        );
    }
    let all = file(&f, r#"{"all":[]}"#);
    let page = model(&dir, "query-elements", &["--rule", &all, "--limit", "1"]);
    let next = text(&page, "next");
    let pin = text(&page, "model_manifest");
    let second = model(
        &dir,
        "query-elements",
        &["--rule", &all, "--after", &next, "--expected-model", &pin],
    );
    assert_eq!(second.get("elements").unwrap().array().unwrap().len(), 1);
    let head = model(&dir, "open-revision", &[]);
    for rule in [
        r#"{"field":"typo","op":"eq","value":"plate"}"#,
        r#"{"field":"attribute","attribute":"Missing","op":"eq","value":1}"#,
        r#"{"field":"class","op":"contains","value":1}"#,
    ] {
        let path = file(&f, rule);
        assert_eq!(
            StoreFixture::run_in(dir.to_str().unwrap(), "query-elements", &["--rule", &path]).0,
            2
        );
        assert_eq!(model(&dir, "open-revision", &[]), head);
    }
    model(
        &dir,
        "create-plate",
        &["--width", "20", "--length", "20", "--thickness", "2"],
    );
    let all = file(&f, r#"{"all":[]}"#);
    assert_eq!(
        StoreFixture::run_in(
            dir.to_str().unwrap(),
            "query-elements",
            &["--rule", &all, "--expected-model", &pin]
        )
        .0,
        2
    );
}

#[test]
fn numeric_filter_units_and_localized_attribute_names_are_exact() {
    let f = StoreFixture::new("filter-units");
    let dir = f.root.join("store");
    let (code, p) = f.run(
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    assert_eq!(code, 0);
    let id = text(p.get("plate").unwrap(), "element_id");
    model(
        &dir,
        "define-attribute",
        &["--name", "Длина", "--type", "number", "--unit", "mm"],
    );
    let values = file(&f, r#"{"Длина":12}"#);
    model(&dir, "set-attributes", &["--id", &id, "--values", &values]);
    let good = file(
        &f,
        r#"{"field":"attribute","attribute":"Длина","op":"gt","value":{"value":10,"unit":"mm"}}"#,
    );
    assert_eq!(
        model(&dir, "query-elements", &["--rule", &good])
            .get("elements")
            .unwrap()
            .array()
            .unwrap(),
        &[Json::text(id)]
    );
    let before = model(&dir, "open-revision", &[]);
    let bad = file(
        &f,
        r#"{"field":"attribute","attribute":"Длина","op":"gt","value":{"value":10,"unit":"m"}}"#,
    );
    assert_eq!(
        StoreFixture::run_in(dir.to_str().unwrap(), "query-elements", &["--rule", &bad]).0,
        2
    );
    assert_eq!(model(&dir, "open-revision", &[]), before);
}
