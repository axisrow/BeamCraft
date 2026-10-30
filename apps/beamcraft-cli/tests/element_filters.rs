// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V14: the CLI query, the engine evaluator and the refusals over one pinned head.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use support::{StoreFixture, text};

fn ok(f: &StoreFixture, command: &str, options: &[&str]) -> Json {
    let (code, reply) = f.run(command, options);
    assert_eq!(code, 0, "{command}: {reply:?}");
    reply
}
fn ids(j: &Json) -> Vec<String> {
    j.get("elements")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|e| e.string().unwrap().to_owned())
        .collect()
}
fn rule_file(f: &StoreFixture, rule: &str) -> String {
    let path = f.root.join("rule.json");
    std::fs::write(&path, rule).unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn typed_rules_select_by_type_name_class_and_attribute_without_writing() {
    let f = StoreFixture::new("element-filters");
    let alpha = ok(
        &f,
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
    let a = text(alpha.get("plate").unwrap(), "element_id");
    ok(
        &f,
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
    ok(
        &f,
        "attribute-define",
        &[
            "--name",
            "Status",
            "--type",
            "choice",
            "--options",
            "New,Done",
        ],
    );
    ok(
        &f,
        "attribute-set",
        &["--element", &a, "--name", "Status", "--value", "New"],
    );
    let before = ok(&f, "open-revision", &[]);

    let query = |rule: &str| ids(&ok(&f, "query-elements", &["--rule", &rule_file(&f, rule)]));
    assert_eq!(
        query(r#"{"field":"type","op":"eq","value":"plate"}"#).len(),
        2
    );
    assert_eq!(
        query(r#"{"field":"class","op":"eq","value":7}"#),
        vec![a.clone()]
    );
    assert_eq!(
        query(r#"{"field":"name","op":"contains","value":"lph"}"#),
        vec![a.clone()]
    );
    // Beta carries no Status: a missing value never satisfies ne.
    assert_eq!(
        query(r#"{"field":"attribute","attribute":"Status","op":"ne","value":"Done"}"#),
        vec![a.clone()]
    );
    assert_eq!(
        query(
            r#"{"all":[{"field":"type","op":"eq","value":"plate"},{"any":[{"field":"class","op":"eq","value":7},{"field":"name","op":"starts-with","value":"Z"}]},{"field":"attribute","attribute":"Status","op":"ne","value":"Done"}]}"#,
        ),
        vec![a]
    );
    assert_eq!(ok(&f, "open-revision", &[]), before);
}

#[test]
fn unknown_fields_attributes_and_incompatible_operators_refuse_with_a_pinned_page() {
    let f = StoreFixture::new("filter-refusals");
    let empty_all = rule_file(&f, r#"{"all":[]}"#);
    // `none` pins a store without a revision and matches its absent digest.
    let empty = ok(
        &f,
        "query-elements",
        &["--rule", &empty_all, "--expected-model", "none"],
    );
    assert!(ids(&empty).is_empty());
    for class in ["1", "2"] {
        ok(
            &f,
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
    let all = rule_file(&f, r#"{"all":[]}"#);
    let (code, _) = f.run(
        "query-elements",
        &["--rule", &all, "--expected-model", "none"],
    );
    assert_eq!(code, 2, "a store with a revision is not pin none");
    let page = ok(&f, "query-elements", &["--rule", &all, "--limit", "1"]);
    let next = text(&page, "next");
    let pin = text(&page, "model_manifest");
    let second = ok(
        &f,
        "query-elements",
        &["--rule", &all, "--after", &next, "--expected-model", &pin],
    );
    assert_eq!(second.get("elements").unwrap().array().unwrap().len(), 1);
    let head = ok(&f, "open-revision", &[]);
    for rule in [
        r#"{"field":"typo","op":"eq","value":"plate"}"#,
        r#"{"field":"attribute","attribute":"Missing","op":"eq","value":1}"#,
        r#"{"field":"class","op":"contains","value":1}"#,
        r#"{"field":"class","op":"eq","value":"7"}"#,
    ] {
        let (code, _) = f.run("query-elements", &["--rule", &rule_file(&f, rule)]);
        assert_eq!(code, 2, "{rule}");
        assert_eq!(ok(&f, "open-revision", &[]), head);
    }
    ok(
        &f,
        "create-plate",
        &["--width", "20", "--length", "20", "--thickness", "2"],
    );
    let (code, _) = f.run(
        "query-elements",
        &["--rule", &all, "--expected-model", &pin],
    );
    assert_eq!(code, 2);
}

#[test]
fn numeric_filter_units_are_exact_against_their_definition() {
    let f = StoreFixture::new("filter-units");
    let plate = ok(
        &f,
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    let id = text(plate.get("plate").unwrap(), "element_id");
    ok(
        &f,
        "attribute-define",
        &["--name", "Length", "--type", "number", "--unit", "mm"],
    );
    ok(
        &f,
        "attribute-set",
        &["--element", &id, "--name", "Length", "--value", "12"],
    );
    let good = rule_file(
        &f,
        r#"{"field":"attribute","attribute":"Length","op":"gt","value":{"value":10,"unit":"mm"}}"#,
    );
    assert_eq!(ids(&ok(&f, "query-elements", &["--rule", &good])), vec![id]);
    let before = ok(&f, "open-revision", &[]);
    let bad = rule_file(
        &f,
        r#"{"field":"attribute","attribute":"Length","op":"gt","value":{"value":10,"unit":"m"}}"#,
    );
    let (code, _) = f.run("query-elements", &["--rule", &bad]);
    assert_eq!(code, 2);
    assert_eq!(ok(&f, "open-revision", &[]), before);
}

#[test]
fn a_stale_choice_option_never_matches_and_never_refuses_the_query() {
    let f = StoreFixture::new("filter-stale-choice");
    let plate = ok(
        &f,
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    let id = text(plate.get("plate").unwrap(), "element_id");
    ok(
        &f,
        "attribute-define",
        &[
            "--name",
            "Status",
            "--type",
            "choice",
            "--options",
            "New,Done",
        ],
    );
    ok(
        &f,
        "attribute-set",
        &["--element", &id, "--name", "Status", "--value", "New"],
    );
    // The value outlives the undefine (I04); the redefinition declares other options, so the
    // stored option is stale from here on.
    ok(&f, "attribute-undefine", &["--name", "Status"]);
    ok(
        &f,
        "attribute-define",
        &[
            "--name",
            "Status",
            "--type",
            "choice",
            "--options",
            "Open,Done",
        ],
    );
    let all = rule_file(&f, r#"{"all":[]}"#);
    assert_eq!(ids(&ok(&f, "query-elements", &["--rule", &all])), vec![id]);
    for option in ["Open", "Done"] {
        let rule = rule_file(
            &f,
            &format!(
                r#"{{"field":"attribute","attribute":"Status","op":"eq","value":"{option}"}}"#
            ),
        );
        assert!(
            ids(&ok(&f, "query-elements", &["--rule", &rule])).is_empty(),
            "{option}"
        );
    }
    let stale = rule_file(
        &f,
        r#"{"field":"attribute","attribute":"Status","op":"eq","value":"New"}"#,
    );
    let (code, _) = f.run("query-elements", &["--rule", &stale]);
    assert_eq!(code, 2, "a foreign option literal refuses at admission");
}
