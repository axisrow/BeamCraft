// SPDX-License-Identifier: MPL-2.0
//! Real Cargo/nextest stages, fresh inventories and exact-source verification.

use crate::{
    Result, capture, fail, git, process, read,
    report::{NEXTEST_VERSION, Os, Report, Scope, Stage, cli_paths, oid},
};
use quick_xml::{Reader, events::Event};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BUILD_LIMIT: Duration = Duration::from_secs(2700);
const DOC_LIMIT: Duration = Duration::from_secs(120);

/// Options for a final local gate. No arbitrary test filters or platform overrides are accepted.
pub struct Options {
    /// Full workspace or whole CLI package.
    pub scope: Scope,
    /// Integration base ref.
    pub base: String,
    /// Explicit CLI/UI contract assertion.
    pub reason: Option<String>,
    /// Optional output report path.
    pub report: Option<PathBuf>,
    /// The existing sync's staged, uncommitted merge rather than an ordinary clean HEAD.
    pub staged_merge: bool,
}

/// Source identity before and after a gate.
#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// HEAD at verification start.
    pub commit: String,
    /// Exact content tree.
    pub tree: String,
    /// Optional pending upstream parent.
    pub merge_parent: Option<String>,
}

/// Captures clean source, or a staged merge with neither unstaged nor untracked source changes.
pub fn snapshot(root: &Path, staged: bool) -> Result<Snapshot> {
    let commit = git(root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if !git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?.is_empty() {
        return fail("untracked source files: commit or remove them before a final gate");
    }
    let (tree, merge_parent) = if staged {
        git(root, &["diff", "--quiet"])?;
        let parent = git(root, &["rev-parse", "--verify", "MERGE_HEAD"])?;
        if !oid(&parent) {
            return fail("sync gate requires exactly one pending merge parent");
        }
        (git(root, &["write-tree"])?, Some(parent))
    } else {
        if !git(root, &["status", "--porcelain=v1", "--untracked-files=all"])?.is_empty() {
            return fail(
                "final gate requires committed, clean source; iteration commands remain available",
            );
        }
        (git(root, &["rev-parse", "HEAD^{tree}"])?, None)
    };
    Ok(Snapshot {
        commit,
        tree,
        merge_parent,
    })
}

/// Returns both old and new paths of changes; NUL delimiting handles spaces and renames safely.
pub fn changed_paths(root: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    if !oid(base) || !oid(head) {
        return fail("scope comparison requires object IDs");
    }
    let text = git(
        root,
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            base,
            head,
            "--",
        ],
    )?;
    Ok(text
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}

fn build_target(root: &Path) -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let base = if base.is_absolute() {
        base
    } else {
        root.join(base)
    };
    // Keep Cargo's running `cargo run` wrapper outside the artifacts it rebuilds.
    // The final `target` component also preserves archive paths on another runner.
    base.join("test-gate/target")
}

fn cargo(root: &Path) -> Command {
    let mut command = Command::new("cargo");
    // Match CI's compact test artifacts without disabling runtime assertions.
    command
        .current_dir(root)
        .env("CARGO_TARGET_DIR", build_target(root))
        .env("CARGO_PROFILE_TEST_DEBUG", "0")
        .env("CARGO_PROFILE_TEST_DEBUG_ASSERTIONS", "true")
        .env("CARGO_PROFILE_TEST_OVERFLOW_CHECKS", "true")
        .env("CARGO_INCREMENTAL", "0");
    command
}

fn config(root: &Path, output: &Path) -> Result<PathBuf> {
    let original = read(&root.join(".config/nextest.toml"))?;
    if !original.contains("path = \"junit.xml\"") {
        return fail("gate profiles must write JUnit reports");
    }
    let junit = serde_json::to_string(&output.join("junit.xml").to_string_lossy())?;
    let path = output.join("nextest.toml");
    std::fs::write(
        &path,
        original.replace("path = \"junit.xml\"", &format!("path = {junit}")),
    )?;
    Ok(path)
}

fn nextest(root: &Path, action: &str, profile: &str, config: &Path) -> Command {
    let mut command = cargo(root);
    command
        .args([
            "nextest",
            action,
            "--profile",
            profile,
            "--user-config-file",
            "none",
            "--config-file",
        ])
        .arg(config)
        .arg("--ignore-default-filter");
    command
}

fn stage(name: &str, command: &mut Command, output: &Path, limit: Duration) -> Stage {
    match process::run(name, command, output, limit) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("gate: {name}: {error}");
            Stage {
                name: name.into(),
                exit_code: 1,
                elapsed_ms: 0,
                timed_out: false,
            }
        }
    }
}

/// Counts the exact runnable tests selected by the nextest list JSON, not a log-line heuristic.
pub fn inventory(text: &str) -> Result<u64> {
    let value: Value = serde_json::from_str(text)?;
    let suites = value
        .get("rust-suites")
        .and_then(Value::as_object)
        .ok_or("missing nextest suite inventory")?;
    let mut count = 0_u64;
    for suite in suites.values() {
        let cases = suite
            .get("testcases")
            .and_then(Value::as_object)
            .ok_or("unlisted test suite")?;
        for case in cases.values() {
            let ignored = case
                .get("ignored")
                .and_then(Value::as_bool)
                .ok_or("missing ignored flag")?;
            let matched = case
                .pointer("/filter-match/status")
                .and_then(Value::as_str)
                .ok_or("missing filter result")?;
            if !ignored && matched == "matches" {
                count = count.checked_add(1).ok_or("too many tests")?;
            }
        }
    }
    if count == 0 {
        return fail("the gate selected zero runnable tests");
    }
    Ok(count)
}

fn ignored_inventory(text: &str) -> Result<Vec<String>> {
    let value: Value = serde_json::from_str(text)?;
    let suites = value
        .get("rust-suites")
        .and_then(Value::as_object)
        .ok_or("missing nextest suite inventory")?;
    let mut ignored = Vec::new();
    for (id, suite) in suites {
        let cases = suite
            .get("testcases")
            .and_then(Value::as_object)
            .ok_or("unlisted test suite")?;
        for (name, case) in cases {
            if case.get("ignored").and_then(Value::as_bool) == Some(true) {
                ignored.push(format!("{id}::{name}"));
            }
        }
    }
    Ok(ignored)
}

/// Validated execution counts and runner-only time from a complete JUnit report.
pub struct TestResult {
    /// Actually executed cases, excluding explicitly skipped cases.
    pub executed: u64,
    /// Runner execution duration, without Cargo/build/archive overhead.
    pub elapsed_ms: u64,
    /// Whether every executed case passed.
    pub passed: bool,
}

/// Reads fresh JUnit output and rejects incomplete XML or inconsistent testcase counts.
pub fn junit(text: &str) -> Result<TestResult> {
    let mut reader = Reader::from_str(text);
    let mut root = None;
    let mut entries = 0_u64;
    let mut closed = false;
    loop {
        match reader.read_event()? {
            Event::Start(e) if e.name().as_ref() == b"testsuites" => {
                if root.is_some() {
                    return fail("repeated JUnit root");
                }
                let mut tests = None;
                let mut skipped = None;
                let mut errors = None;
                let mut failures = None;
                let mut time = None;
                for attr in e.attributes() {
                    let a = attr?;
                    let value = std::str::from_utf8(a.value.as_ref())?;
                    match a.key.as_ref() {
                        b"tests" => tests = Some(value.parse::<u64>()?),
                        b"skipped" => skipped = Some(value.parse::<u64>()?),
                        b"errors" => errors = Some(value.parse::<u64>()?),
                        b"failures" => failures = Some(value.parse::<u64>()?),
                        b"time" => time = Some(Duration::try_from_secs_f64(value.parse()?)?),
                        _ => {}
                    }
                }
                root = Some((
                    tests.ok_or("missing test count")?,
                    skipped.ok_or("missing skip count")?,
                    errors.ok_or("missing error count")?,
                    failures.ok_or("missing failure count")?,
                    time.ok_or("missing runtime")?,
                ));
            }
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"testcase" => {
                entries = entries.checked_add(1).ok_or("too many JUnit cases")?;
            }
            Event::End(e) if e.name().as_ref() == b"testsuites" => closed = true,
            Event::Eof => break,
            _ => {}
        }
    }
    let (tests, skipped, errors, failures, time) = root.ok_or("missing JUnit root")?;
    if !closed || tests != entries {
        return fail("incomplete JUnit report");
    }
    Ok(TestResult {
        executed: tests.checked_sub(skipped).ok_or("invalid skipped count")?,
        elapsed_ms: u64::try_from(time.as_millis())?,
        passed: errors == 0 && failures == 0,
    })
}

fn finish_tests(step: &mut Stage, output: &Path, expected: u64, budget_ms: u64) -> u64 {
    match read(&output.join("junit.xml")).and_then(|text| junit(&text)) {
        Ok(result) => {
            step.elapsed_ms = result.elapsed_ms;
            if (!result.passed
                || expected == 0
                || result.executed != expected
                || result.elapsed_ms > budget_ms)
                && step.exit_code == 0
            {
                step.exit_code = 1;
            }
            result.executed
        }
        Err(error) => {
            eprintln!("gate: missing/incomplete test evidence: {error}");
            if step.exit_code == 0 {
                step.exit_code = 1;
            }
            0
        }
    }
}

/// Executes and records the full local gate, retaining failed reports for diagnosis.
pub fn check(root: &Path, options: Options) -> Result<PathBuf> {
    let _lock = process::lock()?;
    if options.scope == Scope::Full && std::env::var_os("BEAMCRAFT_THEME").is_some() {
        return fail("unset BEAMCRAFT_THEME for a full gate: it suppresses theme acceptance tests");
    }
    let before = snapshot(root, options.staged_merge)?;
    if options.base.starts_with('-') {
        return fail("base cannot be an option");
    }
    let base = if options.staged_merge {
        before.commit.clone()
    } else {
        git(
            root,
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", options.base),
            ],
        )?
    };
    git(
        root,
        &["merge-base", "--is-ancestor", &base, &before.commit],
    )?;
    if options.scope == Scope::Cli {
        if options.staged_merge
            || !cli_paths(
                changed_paths(root, &base, &before.commit)?
                    .iter()
                    .map(String::as_str),
            )
        {
            return fail(
                "CLI-only requires CLI source/test changes without shared components, manifests or contracts",
            );
        }
        if !options
            .reason
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty() && s.len() <= 2048)
        {
            return fail(
                "provide --ui-independent-reason explaining unchanged UI-consumed commands/options/JSON",
            );
        }
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let report_path = options.report.unwrap_or_else(|| {
        root.join(format!(
            "target/test-gate/{stamp}-{}/report.json",
            std::process::id()
        ))
    });
    let report_path = if report_path.is_absolute() {
        report_path
    } else {
        root.join(report_path)
    };
    let output = report_path
        .parent()
        .ok_or("report needs a parent directory")?;
    if output.exists() {
        return fail("report directory already exists; each gate needs fresh evidence");
    }
    std::fs::create_dir_all(output)?;
    let rustc = capture(Command::new("rustc").arg("--version"))?;
    let cargo_version = capture(cargo(root).arg("--version"))?;
    let nextest_version = capture(cargo(root).args(["nextest", "--version"]))?;
    if nextest_version.split_whitespace().nth(1) != Some(NEXTEST_VERSION) {
        return fail("install cargo-nextest 0.9.149 before running the gate");
    }
    let cfg = config(root, output)?;
    let selection = options.scope.cargo_args();
    let mut stages = vec![stage(
        "format",
        cargo(root).args(["fmt", "--all", "--", "--check"]),
        output,
        BUILD_LIMIT,
    )];
    stages.push(stage(
        "clippy",
        cargo(root).arg("clippy").args(selection).args([
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
        output,
        BUILD_LIMIT,
    ));
    let prepare = stage(
        "prepare",
        nextest(root, "list", options.scope.profile(), &cfg)
            .args(selection)
            .args(["--locked", "--message-format", "json"]),
        output,
        BUILD_LIMIT,
    );
    let prepared = prepare.exit_code == 0;
    stages.push(prepare);
    let expected = if prepared {
        read(&output.join("prepare.stdout.log"))
            .and_then(|text| inventory(&text))
            .unwrap_or_else(|error| {
                eprintln!("gate: {error}");
                0
            })
    } else {
        0
    };
    let mut tests = if expected > 0 {
        stage(
            "tests",
            nextest(root, "run", options.scope.profile(), &cfg)
                .args(selection)
                .args([
                    "--locked",
                    "--no-fail-fast",
                    "--retries",
                    "0",
                    "--no-tests",
                    "fail",
                ]),
            output,
            BUILD_LIMIT,
        )
    } else {
        Stage {
            name: "tests".into(),
            exit_code: 1,
            elapsed_ms: 0,
            timed_out: false,
        }
    };
    let executed = finish_tests(&mut tests, output, expected, options.scope.budget_ms());
    stages.push(tests);
    let docs = if prepared {
        stage(
            "doctests",
            cargo(root)
                .arg("test")
                .args(selection)
                .args(["--doc", "--locked", "--no-fail-fast"]),
            output,
            DOC_LIMIT,
        )
    } else {
        Stage {
            name: "doctests".into(),
            exit_code: 1,
            elapsed_ms: 0,
            timed_out: false,
        }
    };
    stages.push(docs);
    stages.push(stage(
        "docs",
        cargo(root)
            .arg("doc")
            .args(selection)
            .args(["--no-deps", "--locked"])
            .env("RUSTDOCFLAGS", "-D warnings"),
        output,
        BUILD_LIMIT,
    ));
    let ignored_tests = if prepared {
        ignored_inventory(&read(&output.join("prepare.stdout.log"))?)?
    } else {
        Vec::new()
    };
    eprintln!(
        "gate: {executed}/{expected} runnable tests executed; {} explicitly ignored cases remain listed in the report",
        ignored_tests.len()
    );
    let report = Report {
        version: 1,
        commit: before.commit.clone(),
        tree: before.tree.clone(),
        base,
        merge_parent: before.merge_parent.clone(),
        os: Os::current()?,
        arch: std::env::consts::ARCH.into(),
        scope: options.scope,
        ui_tested: options.scope == Scope::Full,
        ui_independent_reason: options.reason,
        rustc,
        cargo: cargo_version,
        nextest: nextest_version,
        expected_tests: expected,
        executed_tests: executed,
        ignored_tests,
        source_unchanged: snapshot(root, options.staged_merge).is_ok_and(|after| after == before),
        stages,
    };
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    eprintln!("gate: report {}", report_path.display());
    report.validate()?;
    Ok(report_path)
}

/// Builds a CI archive and checks doctests, preserving both failure statuses.
pub fn ci_archive(root: &Path, output: &Path) -> Result<()> {
    std::fs::create_dir_all(output)?;
    let build = stage(
        "archive",
        cargo(root)
            .args([
                "nextest",
                "archive",
                "--workspace",
                "--locked",
                "--archive-file",
            ])
            .arg(output.join("nextest-archive.tar.zst")),
        output,
        BUILD_LIMIT,
    );
    let docs = if build.exit_code == 0 {
        stage(
            "doctests",
            cargo(root).args(["test", "--workspace", "--doc", "--locked", "--no-fail-fast"]),
            output,
            DOC_LIMIT,
        )
    } else {
        Stage {
            name: "doctests".into(),
            exit_code: 1,
            elapsed_ms: 0,
            timed_out: false,
        }
    };
    std::fs::write(
        output.join("archive-report.json"),
        serde_json::to_vec_pretty(&[build.clone(), docs.clone()])?,
    )?;
    if build.exit_code != 0 || docs.exit_code != 0 {
        return fail("archive/doctest gate failed; see archive-report.json");
    }
    std::fs::copy(
        std::env::current_exe()?,
        output.join(format!(
            "beamcraft-test-gate{}",
            std::env::consts::EXE_SUFFIX
        )),
    )?;
    Ok(())
}

/// Executes a complete CI partition with a fresh inventory and its fixed 180-second budget.
pub fn ci_test(root: &Path, output: &Path, archive: &Path, partition: &str) -> Result<()> {
    let (index, total) = partition.split_once('/').ok_or("partition must be i/n")?;
    let index: u32 = index.parse()?;
    let total: u32 = total.parse()?;
    if index == 0 || index > total || total != Os::current()?.shards() {
        return fail("unexpected CI partition");
    }
    std::fs::create_dir_all(output)?;
    let cfg = config(root, output)?;
    let target = build_target(root);
    let extract = target
        .parent()
        .ok_or("missing archive extraction directory")?;
    std::fs::create_dir_all(extract)?;
    let part = format!("hash:{index}/{total}");
    let prepare = stage(
        "prepare",
        nextest(root, "list", "gate-ci", &cfg)
            .arg("--archive-file")
            .arg(archive)
            .args([
                "--extract-overwrite",
                "--partition",
                &part,
                "--message-format",
                "json",
            ])
            .arg("--extract-to")
            .arg(extract),
        output,
        BUILD_LIMIT,
    );
    let expected = if prepare.exit_code == 0 {
        inventory(&read(&output.join("prepare.stdout.log"))?)?
    } else {
        0
    };
    let mut tests = if expected > 0 {
        stage(
            "tests",
            nextest(root, "run", "gate-ci", &cfg)
                .arg("--archive-file")
                .arg(archive)
                .args([
                    "--extract-overwrite",
                    "--partition",
                    &part,
                    "--no-fail-fast",
                    "--retries",
                    "0",
                    "--no-tests",
                    "fail",
                ])
                .arg("--extract-to")
                .arg(extract),
            output,
            BUILD_LIMIT,
        )
    } else {
        Stage {
            name: "tests".into(),
            exit_code: 1,
            elapsed_ms: 0,
            timed_out: false,
        }
    };
    let executed = finish_tests(&mut tests, output, expected, 180_000);
    std::fs::write(
        output.join("partition-report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"expected_tests":expected,"executed_tests":executed,"stages":[prepare,tests.clone()]}),
        )?,
    )?;
    if tests.exit_code != 0 {
        return fail("CI partition failed or did not produce complete evidence");
    }
    Ok(())
}
