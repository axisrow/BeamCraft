// SPDX-License-Identifier: MPL-2.0
//! Explicit report publication and fail-closed GitHub event planning.

use crate::{
    Result, capture, fail, git,
    local::{changed_paths, snapshot},
    read,
    report::{Report, Scope, cli_paths, from_body, oid, with_report},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Commit-message receipt used only by the existing no-PR sync flow.
pub const SYNC_TRAILER: &str = "BeamCraft-Test-Gate: ";

/// How long planning may wait for a receipt that lands after the triggering push.
const GRACE: Duration = Duration::from_secs(120);
/// Pause between live-description polls inside the grace window.
const POLL: Duration = Duration::from_secs(5);

fn api(repo: &str, path: &str) -> Result<Value> {
    check_repo(repo)?;
    let text = capture(Command::new("gh").args(["api", &format!("repos/{repo}/{path}")]))?;
    Ok(serde_json::from_str(&text)?)
}

fn check_repo(repo: &str) -> Result<()> {
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|s| {
            s.is_empty()
                || *s == "."
                || *s == ".."
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    {
        return fail("repository must be owner/name");
    }
    Ok(())
}

/// Gets owner/name from a GitHub HTTPS or SSH origin without discovering credential values.
pub fn repository(root: &Path) -> Result<String> {
    let remote = git(root, &["remote", "get-url", "origin"])?;
    let repo = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("git@github.com:"))
        .ok_or("supply --repo owner/name for this origin")?
        .trim_end_matches(".git");
    check_repo(repo)?;
    Ok(repo.into())
}

fn field<'a>(value: &'a Value, pointer: &str) -> Result<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| std::io::Error::other(format!("missing GitHub field: {pointer}")).into())
}

fn ensure_commit(root: &Path, sha: &str) -> Result<()> {
    if !oid(sha) {
        return fail("malformed commit ID");
    }
    let status = Command::new("git")
        .current_dir(root)
        .args(["cat-file", "-e", &format!("{sha}^{{commit}}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        git(root, &["fetch", "--no-tags", "origin", sha])?;
    }
    Ok(())
}

fn tree(root: &Path, sha: &str) -> Result<String> {
    ensure_commit(root, sha)?;
    git(root, &["rev-parse", &format!("{sha}^{{tree}}")])
}

fn scope(root: &Path, report: &Report) -> Result<()> {
    if report.scope == Scope::Cli {
        ensure_commit(root, &report.base)?;
        ensure_commit(root, &report.commit)?;
        if !cli_paths(
            changed_paths(root, &report.base, &report.commit)?
                .iter()
                .map(String::as_str),
        ) {
            return fail("CLI-only receipt includes changes outside CLI source/tests");
        }
    }
    Ok(())
}

/// Publishes only successful, current, clean-HEAD evidence, preserving the human description.
pub fn publish(root: &Path, repo: &str, number: u64, report_path: &Path) -> Result<()> {
    let report: Report = serde_json::from_str(&read(report_path)?)?;
    report.validate()?;
    let actual = snapshot(root, false)?;
    let pr = api(repo, &format!("pulls/{number}"))?;
    let head = field(&pr, "/head/sha")?;
    let base = field(&pr, "/base/sha")?;
    report.validate_pr(head, base, &actual.tree)?;
    if actual.commit != head {
        return fail("local checkout no longer matches the PR head");
    }
    scope(root, &report)?;
    let body = pr.get("body").and_then(Value::as_str).unwrap_or("");
    let patched = with_report(body, &report)?;
    let payload = serde_json::to_vec(&serde_json::json!({"body":patched}))?;
    let mut child = Command::new("gh")
        .args([
            "api",
            "--method",
            "PATCH",
            &format!("repos/{repo}/pulls/{number}"),
            "--input",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    use std::io::Write;
    // Stream the bounded payload: never overwrite a file next to a copied report.
    let written = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("missing publication stdin"))
        .and_then(|mut input| input.write_all(&payload));
    let result = child.wait()?;
    written?;
    if !result.success() {
        return fail("GitHub refused report publication");
    }
    Ok(())
}

/// Pure, serializable CI selection. Exactly one locally checked OS is excluded.
#[derive(Debug, Serialize, Deserialize)]
pub struct Plan {
    /// Stable hosted-runner labels of the two complete workspace lanes.
    pub oses: Vec<String>,
    /// The selected lane that also runs platform-independent fmt/docs checks.
    pub primary: String,
    /// Local report OS, displayed explicitly in CI summaries.
    pub local_os: crate::report::Os,
    /// Local UI coverage (false for independent CLI verification).
    pub local_ui_tested: bool,
}

/// Selects two platforms only after validating the complete local report.
pub fn select(report: &Report) -> Result<Plan> {
    report.validate()?;
    let oses: Vec<_> = report
        .os
        .others()
        .into_iter()
        .map(|os| os.runner().to_owned())
        .collect();
    let primary = oses.first().ok_or("empty CI selection")?.clone();
    Ok(Plan {
        oses,
        primary,
        local_os: report.os,
        local_ui_tested: report.ui_tested,
    })
}

/// Aggregates every required lane, rejecting failures, missing jobs and unexpected skips.
pub fn verdict(needs: &Value, oses: &[String], event: &str) -> Result<()> {
    let valid = ["ubuntu-latest", "macos-latest", "windows-latest"];
    if oses.len() != 2
        || oses.first() == oses.get(1)
        || oses.iter().any(|s| !valid.contains(&s.as_str()))
    {
        return fail("CI must select exactly two distinct supported platforms");
    }
    let require = |job: &str, expected: &str| -> Result<()> {
        if needs
            .get(job)
            .and_then(|j| j.get("result"))
            .and_then(Value::as_str)
            != Some(expected)
        {
            return fail(format!("CI job {job} did not finish as {expected}"));
        }
        Ok(())
    };
    for job in ["plan", "quality", "wasm", "deny"] {
        require(job, "success")?;
    }
    // Repository-specific policy checks (an attribution guard, for example) run as
    // separate workflow files beside this pipeline, so they are not required lanes.
    require(
        "build-timings",
        if event == "pull_request" {
            "skipped"
        } else {
            "success"
        },
    )?;
    for (os, suffix) in [
        ("ubuntu-latest", "ubuntu"),
        ("macos-latest", "macos"),
        ("windows-latest", "windows"),
    ] {
        let expected = if oses.iter().any(|s| s == os) {
            "success"
        } else {
            "skipped"
        };
        require(&format!("test-archive-{suffix}"), expected)?;
        require(&format!("test-{suffix}"), expected)?;
    }
    Ok(())
}

/// Rejects a verdict whose PR head or base moved while CI was executing.
pub fn current_pr_matches(event_pr: &Value, current_pr: &Value) -> Result<()> {
    for pointer in ["/head/sha", "/base/sha"] {
        if field(event_pr, pointer)? != field(current_pr, pointer)? {
            return fail("PR head/base changed during CI; run and publish a new local gate");
        }
    }
    Ok(())
}

/// Revalidates live PR metadata before the final verdict, not only the event payload.
pub fn refresh_pr(root: &Path, payload: &Value, repo: &str, oses: &[String]) -> Result<()> {
    let event_pr = payload.get("pull_request").ok_or("missing PR event")?;
    let number = event_pr
        .get("number")
        .and_then(Value::as_u64)
        .ok_or("missing PR number")?;
    let current = api(repo, &format!("pulls/{number}"))?;
    current_pr_matches(event_pr, &current)?;
    let current_payload = serde_json::json!({"pull_request":current});
    let selected = plan(
        root,
        "pull_request",
        &current_payload,
        field(event_pr, "/head/sha")?,
        repo,
    )?;
    if selected.oses != oses {
        return fail("local OS evidence changed during CI");
    }
    Ok(())
}

/// Checks a sync receipt against actual merge parents and tree, independently of its message.
pub fn validate_sync(report: &Report, actual_tree: &str, parents: &[String]) -> Result<()> {
    report.validate()?;
    let expected_parent = report
        .merge_parent
        .as_ref()
        .ok_or("not a staged-merge report")?;
    if report.scope != Scope::Full
        || report.tree != actual_tree
        || parents.len() != 2
        || parents.first() != Some(&report.commit)
        || parents.get(1) != Some(expected_parent)
    {
        return fail("sync receipt does not match the actual full merge tree and parents");
    }
    Ok(())
}

/// A merged/squashed/rebased commit can reuse a PR receipt only for identical tested content.
pub fn validate_merged(
    report: &Report,
    original_head: &str,
    original_tree: &str,
    integrated_tree: &str,
) -> Result<()> {
    report.validate()?;
    if report.merge_parent.is_some()
        || report.commit != original_head
        || report.tree != original_tree
        || report.tree != integrated_tree
    {
        return fail("integrated source differs from the tested PR tree");
    }
    Ok(())
}

/// Formats a one-line sync receipt only while the exact tested staged merge still exists.
pub fn sync_trailer(root: &Path, path: &Path) -> Result<String> {
    let report: Report = serde_json::from_str(&read(path)?)?;
    report.validate()?;
    let actual = snapshot(root, true)?;
    if actual.tree != report.tree
        || actual.commit != report.commit
        || actual.merge_parent != report.merge_parent
    {
        return fail("staged merge changed after its gate");
    }
    Ok(format!("{SYNC_TRAILER}{}", serde_json::to_string(&report)?))
}

/// Verifies the newly created sync commit before it can be pushed.
pub fn verify_sync(root: &Path, path: &Path) -> Result<()> {
    let report: Report = serde_json::from_str(&read(path)?)?;
    let actual = snapshot(root, false)?;
    let parents: Vec<_> = git(root, &["show", "-s", "--format=%P", "HEAD"])?
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    validate_sync(&report, &actual.tree, &parents)
}

/// Parses a description snapshot and binds it to this run's exact head, base and tree.
fn receipt_for(body: &str, head: &str, base: &str, tested_tree: &str) -> Result<Report> {
    let report = from_body(body)?;
    report.validate_pr(head, base, tested_tree)?;
    Ok(report)
}

/// Completes PR planning once a receipt is bound: source topology, scope, platforms.
fn finalize_pr(root: &Path, pr: &Value, report: &Report) -> Result<Plan> {
    let head = field(pr, "/head/sha")?;
    let base = field(pr, "/base/sha")?;
    ensure_commit(root, base)?;
    git(root, &["merge-base", "--is-ancestor", base, head])?;
    scope(root, report)?;
    select(report)
}

/// Reads the live PR description; the seam tests substitute for a fake publisher.
fn live_pr(repo: &str, number: u64) -> Result<Value> {
    api(repo, &format!("pulls/{number}"))
}

/// Reads an event and validates its receipt against Git/PR facts before producing a matrix.
pub fn plan(root: &Path, event: &str, payload: &Value, sha: &str, repo: &str) -> Result<Plan> {
    plan_with(root, event, payload, sha, repo, &mut live_pr, GRACE)
}

/// [`plan`] with an injectable live-description source and grace window for tests.
pub fn plan_with(
    root: &Path,
    event: &str,
    payload: &Value,
    sha: &str,
    repo: &str,
    fetch: &mut dyn FnMut(&str, u64) -> Result<Value>,
    grace: Duration,
) -> Result<Plan> {
    check_repo(repo)?;
    if !matches!(event, "pull_request" | "push" | "workflow_dispatch") {
        return fail("unsupported CI event");
    }
    if event == "pull_request" {
        let pr = payload
            .get("pull_request")
            .ok_or("missing pull_request event")?;
        let head = field(pr, "/head/sha")?;
        let base = field(pr, "/base/sha")?;
        let tested_tree = tree(root, head)?;
        if let Ok(report) = receipt_for(
            pr.get("body").and_then(Value::as_str).unwrap_or(""),
            head,
            base,
            &tested_tree,
        ) {
            return finalize_pr(root, pr, &report);
        }
        // Startup race: publication follows the push that started this run, so
        // the event snapshot usually predates the receipt. Poll the live
        // description for a bounded grace window; every attempt stays bound to
        // this run's head and base, and a window without a receipt still fails.
        let number = pr
            .get("number")
            .and_then(Value::as_u64)
            .ok_or("missing PR number")?;
        let deadline = Instant::now() + grace;
        loop {
            let live = fetch(repo, number)?;
            if let Ok(report) = receipt_for(
                live.get("body").and_then(Value::as_str).unwrap_or(""),
                head,
                base,
                &tested_tree,
            ) {
                return finalize_pr(root, pr, &report);
            }
            if Instant::now() >= deadline {
                return fail(
                    "no eligible local gate receipt within the grace window; \
                     publish a current report and rerun CI",
                );
            }
            std::thread::sleep(grace.min(POLL));
        }
    }
    let actual_tree = tree(root, sha)?;
    let message = git(root, &["show", "-s", "--format=%B", sha])?;
    let receipts: Vec<_> = message
        .lines()
        .filter_map(|line| line.strip_prefix(SYNC_TRAILER))
        .collect();
    if !receipts.is_empty() {
        if receipts.len() != 1 {
            return fail("multiple sync receipts");
        }
        let text = receipts.first().ok_or("missing sync receipt")?;
        let report: Report = serde_json::from_str(text)?;
        let parents: Vec<_> = git(root, &["show", "-s", "--format=%P", sha])?
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        validate_sync(&report, &actual_tree, &parents)?;
        return select(&report);
    }
    // A dispatch may identify an open PR; a push must identify a PR that was actually merged.
    let dispatch = payload
        .pointer("/inputs/pr_number")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::parse::<u64>)
        .transpose()?;
    let prs = if let Some(number) = dispatch {
        vec![api(repo, &format!("pulls/{number}"))?]
    } else {
        api(repo, &format!("commits/{sha}/pulls?per_page=100"))?
            .as_array()
            .ok_or("missing associated PR collection")?
            .clone()
    };
    let mut matched = None;
    for entry in prs {
        let number = entry
            .get("number")
            .and_then(Value::as_u64)
            .ok_or("missing PR number")?;
        let pr = api(repo, &format!("pulls/{number}"))?;
        if dispatch.is_none()
            && (pr.get("merged").and_then(Value::as_bool) != Some(true)
                || field(&pr, "/base/ref")? != "master")
        {
            continue;
        }
        let head = field(&pr, "/head/sha")?;
        let body = pr.get("body").and_then(Value::as_str).unwrap_or("");
        let Ok(report) = from_body(body) else {
            continue;
        };
        report.validate()?;
        if validate_merged(&report, head, &tree(root, head)?, &actual_tree).is_err() {
            continue;
        }
        if dispatch.is_some() && sha == head {
            report.validate_pr(head, field(&pr, "/base/sha")?, &actual_tree)?;
        }
        scope(root, &report)?;
        if matched.is_some() {
            return fail("ambiguous source PR receipts");
        }
        matched = Some(select(&report)?);
    }
    matched.ok_or_else(|| {
        std::io::Error::other(
            "no eligible local receipt for this source tree; direct pushes without evidence fail",
        )
        .into()
    })
}
