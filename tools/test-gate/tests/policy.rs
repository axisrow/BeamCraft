// SPDX-License-Identifier: MPL-2.0
//! Policy regressions: stale evidence, partial runs, scope and CI aggregation.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_test_gate::{
    github, local,
    report::{self, Os, Report, STEPS, Scope, Stage},
};

fn success() -> Report {
    Report {
        version: 1,
        commit: "1".repeat(40),
        tree: "2".repeat(40),
        base: "3".repeat(40),
        merge_parent: None,
        os: Os::Linux,
        arch: "x86_64".into(),
        scope: Scope::Full,
        ui_tested: true,
        ui_independent_reason: None,
        rustc: "rustc 1.99.1".into(),
        cargo: "cargo 1.99.1".into(),
        nextest: "cargo-nextest 0.9.149".into(),
        expected_tests: 2,
        executed_tests: 2,
        ignored_tests: Vec::new(),
        source_unchanged: true,
        stages: STEPS
            .iter()
            .map(|s| Stage {
                name: (*s).into(),
                exit_code: 0,
                elapsed_ms: 10,
                timed_out: false,
            })
            .collect(),
    }
}

#[test]
fn each_native_os_excludes_itself_and_selects_two_complete_lanes() {
    for os in [Os::Linux, Os::Macos, Os::Windows] {
        let mut r = success();
        r.os = os;
        let p = github::select(&r).unwrap();
        assert_eq!(p.oses.len(), 2);
        assert!(!p.oses.iter().any(|s| s == os.runner()));
        assert!(p.oses.contains(&p.primary));
        for other in os.others() {
            assert!(p.oses.iter().any(|s| s == other.runner()));
        }
    }
}

#[test]
fn ordinary_failure_and_doctest_failure_cannot_be_erased_by_later_success() {
    for name in ["tests", "doctests"] {
        let mut r = success();
        r.stages
            .iter_mut()
            .find(|s| s.name == name)
            .unwrap()
            .exit_code = 101;
        assert!(r.validate().is_err());
        assert_eq!(r.stages.last().unwrap().exit_code, 0);
    }
}

#[test]
fn empty_partial_timed_out_and_changed_source_reports_are_ineligible() {
    let mut r = success();
    r.expected_tests = 0;
    r.executed_tests = 0;
    assert!(r.validate().is_err());
    let mut r = success();
    r.executed_tests = 1;
    assert!(r.validate().is_err());
    let mut r = success();
    r.source_unchanged = false;
    assert!(r.validate().is_err());
    let mut r = success();
    r.stages.pop();
    assert!(r.validate().is_err());
    let mut r = success();
    r.stages[0].timed_out = true;
    assert!(r.validate().is_err());
    let mut r = success();
    r.stages[0].name = "tests".into();
    assert!(r.validate().is_err());
    let mut r = success();
    r.stages[3].elapsed_ms = 600_001;
    assert!(r.validate().is_err());
    let mut r = success();
    r.stages[4].elapsed_ms = 120_001;
    assert!(r.validate().is_err());
}

#[test]
fn a_new_head_base_or_tree_needs_new_local_evidence() {
    let r = success();
    r.validate_pr(&r.commit, &r.base, &r.tree).unwrap();
    assert!(r.validate_pr(&"4".repeat(40), &r.base, &r.tree).is_err());
    assert!(r.validate_pr(&r.commit, &"4".repeat(40), &r.tree).is_err());
    assert!(r.validate_pr(&r.commit, &r.base, &"4".repeat(40)).is_err());
}

#[test]
fn integration_receipt_matches_content_not_the_merge_method_commit_id() {
    let r = success();
    github::validate_merged(&r, &r.commit, &r.tree, &r.tree).unwrap();
    assert!(github::validate_merged(&r, &r.commit, &r.tree, &"9".repeat(40)).is_err());
    assert!(github::validate_merged(&r, &"9".repeat(40), &r.tree, &r.tree).is_err());
}

#[test]
fn cli_selection_is_whole_package_and_requires_explicit_ui_independence() {
    assert_eq!(Scope::Cli.cargo_args(), &["-p", "beamcraft-cli"]);
    assert_eq!(Scope::Full.cargo_args(), &["--workspace"]);
    assert!(report::cli_paths([
        "apps/beamcraft-cli/src/new_command.rs",
        "apps/beamcraft-cli/tests/new.rs",
        "docs/design/new.md"
    ]));
    for shared in [
        "Cargo.lock",
        "Cargo.toml",
        "apps/beamcraft-cli/Cargo.toml",
        "crates/model/src/lib.rs",
        "apps/beamcraft-ui/src/app.rs",
        ".config/nextest.toml",
        "docs/contracts/protocol.md",
        "CLAUDE.md",
    ] {
        assert!(
            !report::cli_paths(["apps/beamcraft-cli/src/new.rs", shared]),
            "{shared}"
        );
    }
    assert!(!report::cli_paths(["docs/development.md"]));
    let mut r = success();
    r.scope = Scope::Cli;
    r.ui_tested = false;
    assert!(r.validate().is_err());
    r.ui_independent_reason =
        Some("Only an independent console command changed; UI replies are unchanged.".into());
    r.validate().unwrap();
    assert!(!github::select(&r).unwrap().local_ui_tested);
}

#[test]
fn publish_preserves_human_text_escapes_delimiters_and_replaces_one_block() {
    let mut r = success();
    r.scope = Scope::Cli;
    r.ui_tested = false;
    r.ui_independent_reason = Some("text --> <script> is data".into());
    let body = report::with_report("Human description", &r).unwrap();
    assert_eq!(
        report::from_body(&body).unwrap().ui_independent_reason,
        r.ui_independent_reason
    );
    let updated = report::with_report(&format!("{body}\nHuman footer"), &r).unwrap();
    assert!(updated.starts_with("Human description"));
    assert!(updated.ends_with("Human footer"));
    assert_eq!(updated.matches(report::BEGIN).count(), 1);
    assert!(report::from_body("").is_err());
    assert!(report::from_body(&format!("{body}{body}")).is_err());
    assert!(report::from_body(report::BEGIN).is_err());
}

#[test]
fn sync_report_must_bind_full_scope_tree_and_both_actual_parents() {
    let mut r = success();
    r.merge_parent = Some("4".repeat(40));
    let parents = vec![r.commit.clone(), r.merge_parent.clone().unwrap()];
    github::validate_sync(&r, &r.tree, &parents).unwrap();
    assert!(github::validate_sync(&r, &"5".repeat(40), &parents).is_err());
    assert!(github::validate_sync(&r, &r.tree, &["6".repeat(40), parents[1].clone()]).is_err());
    r.scope = Scope::Cli;
    r.ui_tested = false;
    r.ui_independent_reason = Some("not sufficient for sync".into());
    assert!(github::validate_sync(&r, &r.tree, &parents).is_err());
}

#[test]
fn required_ci_jobs_cannot_be_skipped_or_failed_while_the_other_os_is_intentionally_skipped() {
    let oses = vec!["macos-latest".into(), "windows-latest".into()];
    let mut needs = serde_json::json!({});
    for name in [
        "plan",
        "quality",
        "wasm",
        "deny",
        "attribution",
        "test-archive-macos",
        "test-macos",
        "test-archive-windows",
        "test-windows",
    ] {
        needs[name] = serde_json::json!({"result":"success"});
    }
    for name in ["test-archive-ubuntu", "test-ubuntu", "build-timings"] {
        needs[name] = serde_json::json!({"result":"skipped"});
    }
    github::verdict(&needs, &oses, "pull_request").unwrap();
    for bad in ["failure", "cancelled", "skipped"] {
        let mut changed = needs.clone();
        changed["test-windows"]["result"] = bad.into();
        assert!(github::verdict(&changed, &oses, "pull_request").is_err());
    }
    needs["plan"]["result"] = "failure".into();
    assert!(github::verdict(&needs, &oses, "pull_request").is_err());
}

#[test]
fn inventory_and_junit_are_structured_not_green_log_grep() {
    let inventory = serde_json::json!({"rust-suites":{"cli":{"testcases":{
        "one":{"ignored":false,"filter-match":{"status":"matches"}},
        "ignored":{"ignored":true,"filter-match":{"status":"matches"}},
        "other_shard":{"ignored":false,"filter-match":{"status":"mismatch"}}
    }}}});
    assert_eq!(local::inventory(&inventory.to_string()).unwrap(), 1);
    assert!(local::inventory(r#"{"rust-suites":{}}"#).is_err());
    let xml = r#"<testsuites tests="1" skipped="0" errors="0" failures="0" time="0.125"><testsuite><testcase name="one"/></testsuite></testsuites>"#;
    let result = local::junit(xml).unwrap();
    assert!(result.passed);
    assert_eq!(result.executed, 1);
    assert_eq!(result.elapsed_ms, 125);
    assert!(local::junit(&xml.replace("</testsuites>", "")).is_err());
    assert!(local::junit(&xml.replace("tests=\"1\"", "tests=\"2\"")).is_err());
    assert!(
        !local::junit(&xml.replace("failures=\"0\"", "failures=\"1\""))
            .unwrap()
            .passed
    );
}
