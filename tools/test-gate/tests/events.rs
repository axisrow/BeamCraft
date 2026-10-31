// SPDX-License-Identifier: MPL-2.0
//! Real Git events through the executable: no GitHub API or writes outside isolated fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use beamcraft_test_gate::{
    git, github, process,
    report::{self, Os, Report, STEPS, Scope, Stage},
};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Repo {
    temp: PathBuf,
    root: PathBuf,
}
impl Repo {
    fn new() -> Self {
        let temp = std::env::temp_dir().join(format!(
            "beamcraft-gate-events-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = temp.join("repo");
        std::fs::create_dir_all(root.join("apps/beamcraft-cli/src")).unwrap();
        let repo = Self { temp, root };
        repo.git(&["init", "-b", "master"]);
        repo.git(&["config", "user.name", "Gate test"]);
        repo.git(&["config", "user.email", "gate@example.invalid"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(
            repo.root.join("apps/beamcraft-cli/src/one.rs"),
            "// first\n",
        )
        .unwrap();
        repo.commit("initial");
        repo
    }
    fn git(&self, args: &[&str]) -> String {
        git(&self.root, args).unwrap()
    }
    fn commit(&self, message: &str) {
        self.git(&["add", "."]);
        self.git(&["commit", "-m", message]);
    }
    fn report(&self, base: &str) -> Report {
        Report {
            version: 1,
            commit: self.git(&["rev-parse", "HEAD"]),
            tree: self.git(&["rev-parse", "HEAD^{tree}"]),
            base: base.into(),
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
                .map(|name| Stage {
                    name: (*name).into(),
                    exit_code: 0,
                    elapsed_ms: 10,
                    timed_out: false,
                })
                .collect(),
        }
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp);
    }
}

fn plan(repo: &Repo, event: &str, payload: serde_json::Value) -> (Stage, String, String) {
    let path = repo.temp.join("event.json");
    std::fs::write(&path, payload.to_string()).unwrap();
    let output = repo
        .temp
        .join(format!("run-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_beamcraft-test-gate"));
    cmd.current_dir(&repo.root)
        .arg("ci-plan")
        .env("GITHUB_EVENT_NAME", event)
        .env("GITHUB_EVENT_PATH", path)
        .env("GITHUB_SHA", repo.git(&["rev-parse", "HEAD"]))
        .env("GITHUB_REPOSITORY", "test/repo")
        .env_remove("GITHUB_OUTPUT");
    let result = process::run("plan", &mut cmd, &output, Duration::from_secs(10)).unwrap();
    let out = std::fs::read_to_string(output.join("plan.stdout.log")).unwrap();
    let err = std::fs::read_to_string(output.join("plan.stderr.log")).unwrap();
    (result, out, err)
}

#[test]
fn pr_event_requires_current_head_base_source_tree_and_eligible_scope() {
    let r = Repo::new();
    let base = r.git(&["rev-parse", "HEAD"]);
    std::fs::write(
        r.root.join("apps/beamcraft-cli/src/one.rs"),
        "// changed CLI\n",
    )
    .unwrap();
    r.commit("CLI change");
    let report = r.report(&base);
    let body = report::with_report("Human text", &report).unwrap();
    let payload = serde_json::json!({"pull_request":{"head":{"sha":report.commit},"base":{"sha":base},"body":body}});
    let (status, out, err) = plan(&r, "pull_request", payload.clone());
    assert_eq!(status.exit_code, 0, "{err}");
    let selected: github::Plan = serde_json::from_str(&out).unwrap();
    assert_eq!(selected.oses, vec!["macos-latest", "windows-latest"]);
    let mut stale = payload.clone();
    stale["pull_request"]["base"]["sha"] = report.commit.clone().into();
    assert_ne!(plan(&r, "pull_request", stale).0.exit_code, 0);
    let mut missing = payload.clone();
    missing["pull_request"]["body"] = "no machine result".into();
    assert_ne!(plan(&r, "pull_request", missing).0.exit_code, 0);
    r.git(&["commit", "--allow-empty", "-m", "new head"]);
    let mut stale = payload;
    stale["pull_request"]["head"]["sha"] = r.git(&["rev-parse", "HEAD"]).into();
    assert_ne!(plan(&r, "pull_request", stale).0.exit_code, 0);
    std::fs::write(r.root.join("Cargo.toml"), "[workspace]\n").unwrap();
    r.commit("shared manifest change");
    let mut report = r.report(&base);
    report.scope = Scope::Cli;
    report.ui_tested = false;
    report.ui_independent_reason = Some("declared independent but paths disagree".into());
    let payload = serde_json::json!({"pull_request":{"head":{"sha":report.commit},"base":{"sha":base},"body":report::with_report("",&report).unwrap()}});
    assert_ne!(plan(&r, "pull_request", payload).0.exit_code, 0);
}

#[test]
fn sync_transport_accepts_the_tested_merge_and_rejects_a_different_committed_tree() {
    let r = Repo::new();
    r.git(&["checkout", "-b", "upstream"]);
    std::fs::write(
        r.root.join("apps/beamcraft-cli/src/one.rs"),
        "// upstream change\n",
    )
    .unwrap();
    r.commit("upstream");
    let upstream = r.git(&["rev-parse", "HEAD"]);
    r.git(&["checkout", "master"]);
    std::fs::write(r.root.join("README.md"), "fork\n").unwrap();
    r.commit("fork change");
    let base = r.git(&["rev-parse", "HEAD"]);
    r.git(&["merge", "--no-commit", "--no-ff", "upstream"]);
    let mut report = r.report(&base);
    report.tree = r.git(&["write-tree"]);
    report.merge_parent = Some(upstream);
    let path = r.temp.join("sync.json");
    std::fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    let trailer = github::sync_trailer(&r.root, &path).unwrap();
    r.git(&["commit", "-m", "sync", "-m", &trailer]);
    github::verify_sync(&r.root, &path).unwrap();
    let (status, out, err) = plan(&r, "push", serde_json::json!({}));
    assert_eq!(status.exit_code, 0, "{err}");
    assert_eq!(
        serde_json::from_str::<github::Plan>(&out)
            .unwrap()
            .oses
            .len(),
        2
    );
    std::fs::write(r.root.join("README.md"), "changed after the gate\n").unwrap();
    r.git(&["add", "."]);
    r.git(&["commit", "--amend", "--no-edit"]);
    assert!(github::verify_sync(&r.root, &path).is_err());
    assert_ne!(plan(&r, "push", serde_json::json!({})).0.exit_code, 0);
}

type FakeFetch<'a> = &'a mut dyn FnMut(&str, u64) -> beamcraft_test_gate::Result<serde_json::Value>;

fn plan_with_grace(
    r: &Repo,
    payload: serde_json::Value,
    grace: Duration,
    fetch: FakeFetch<'_>,
) -> beamcraft_test_gate::Result<github::Plan> {
    github::plan_with(
        &r.root,
        "pull_request",
        &payload,
        r.git(&["rev-parse", "HEAD"]).trim(),
        "test/repo",
        fetch,
        grace,
    )
}

fn opened_payload(body: &str, head: &str, base: &str) -> serde_json::Value {
    serde_json::json!({"pull_request":{"number":7,"head":{"sha":head},"base":{"sha":base},"body":body}})
}

#[test]
fn a_receipt_published_after_the_push_is_found_within_the_grace_window() {
    let r = Repo::new();
    let base = r.git(&["rev-parse", "HEAD"]);
    std::fs::write(
        r.root.join("apps/beamcraft-cli/src/one.rs"),
        "// changed CLI\n",
    )
    .unwrap();
    r.commit("CLI change");
    let report = r.report(&base);
    let good = report::with_report("Human text", &report).unwrap();
    let payload = opened_payload("no receipt yet", &report.commit, &base);
    let mut calls = 0_u32;
    let mut fetch = |_repo: &str, number: u64| {
        calls += 1;
        assert_eq!(number, 7);
        if calls == 1 {
            Ok(serde_json::json!({"body":"still no receipt"}))
        } else {
            Ok(serde_json::json!({"body":good.clone()}))
        }
    };
    let selected =
        plan_with_grace(&r, payload, Duration::from_secs(1), &mut fetch).expect("grace receipt");
    assert_eq!(selected.oses, vec!["macos-latest", "windows-latest"]);
    assert_eq!(calls, 2);
}

#[test]
fn a_window_without_any_receipt_fails_bounded() {
    let r = Repo::new();
    let base = r.git(&["rev-parse", "HEAD"]);
    let payload = opened_payload("no receipt ever", &r.git(&["rev-parse", "HEAD"]), &base);
    let mut fetch = |_repo: &str, _number: u64| Ok(serde_json::json!({"body":"never"}));
    let start = Instant::now();
    let error = plan_with_grace(&r, payload, Duration::from_secs(1), &mut fetch)
        .expect_err("an absent receipt must still fail");
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "must stay bounded"
    );
    assert!(error.to_string().contains("grace window"), "{error}");
}

#[test]
fn a_live_receipt_for_another_head_never_satisfies_this_run() {
    let r = Repo::new();
    let base = r.git(&["rev-parse", "HEAD"]);
    let report = r.report(&base);
    std::fs::write(r.root.join("README.md"), "advanced\n").unwrap();
    r.commit("newer head, receipt not for this run");
    let newer = r.report(&base);
    let live = report::with_report("", &newer).unwrap();
    let payload = opened_payload("stale snapshot", &report.commit, &base);
    let mut fetch = |_repo: &str, _number: u64| Ok(serde_json::json!({"body":live.clone()}));
    let error = plan_with_grace(&r, payload, Duration::from_secs(1), &mut fetch)
        .expect_err("a newer head's receipt is not this run's evidence");
    assert!(error.to_string().contains("grace window"), "{error}");
}

#[test]
fn a_current_payload_receipt_never_reaches_the_live_poll() {
    let r = Repo::new();
    let base = r.git(&["rev-parse", "HEAD"]);
    let report = r.report(&base);
    let body = report::with_report("Human text", &report).unwrap();
    let payload = opened_payload(&body, &report.commit, &base);
    let mut fetch = |_repo: &str, _number: u64| -> beamcraft_test_gate::Result<serde_json::Value> {
        panic!("the fast path must not consult the API")
    };
    let selected = plan_with_grace(&r, payload, Duration::from_secs(30), &mut fetch).unwrap();
    assert_eq!(selected.oses, vec!["macos-latest", "windows-latest"]);
}
