// SPDX-License-Identifier: MPL-2.0
//! Report validation and conservative scope/platform selection.

use crate::{Result, fail};
use serde::{Deserialize, Serialize};

/// Pinned test runner version.
pub const NEXTEST_VERSION: &str = "0.9.149";
/// Required recorded steps, each occurring exactly once.
pub const STEPS: [&str; 6] = ["format", "clippy", "prepare", "tests", "doctests", "docs"];
/// Delimiter of the machine block in a PR description.
pub const BEGIN: &str = "<!-- beamcraft-test-gate:v1\n";
/// Closing delimiter of the machine block.
pub const END: &str = "\n-->";

/// A supported local operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    /// Native Linux.
    Linux,
    /// Native macOS.
    Macos,
    /// Native Windows.
    Windows,
}
impl Os {
    /// Detects the host OS, refusing unsupported hosts.
    pub fn current() -> Result<Self> {
        match std::env::consts::OS {
            "linux" => Ok(Self::Linux),
            "macos" => Ok(Self::Macos),
            "windows" => Ok(Self::Windows),
            _ => fail("the test gate supports Linux, macOS and Windows"),
        }
    }
    /// GitHub hosted runner label.
    pub const fn runner(self) -> &'static str {
        match self {
            Self::Linux => "ubuntu-latest",
            Self::Macos => "macos-latest",
            Self::Windows => "windows-latest",
        }
    }
    /// Existing partition count on that OS.
    pub const fn shards(self) -> u32 {
        match self {
            Self::Linux => 4,
            Self::Macos => 6,
            Self::Windows => 12,
        }
    }
    /// The two CI operating systems, in stable order.
    pub fn others(self) -> Vec<Self> {
        [Self::Linux, Self::Macos, Self::Windows]
            .into_iter()
            .filter(|os| *os != self)
            .collect()
    }
}

/// Whole-workspace or explicitly UI-independent CLI verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Every workspace test and its documentation.
    Full,
    /// All targets of the CLI package, without building UI.
    Cli,
}
impl Scope {
    /// Cargo package-selection arguments; never a test-name filter.
    pub const fn cargo_args(self) -> &'static [&'static str] {
        match self {
            Self::Full => &["--workspace"],
            Self::Cli => &["-p", "beamcraft-cli"],
        }
    }
    /// The mandatory unfiltered local profile.
    pub const fn profile(self) -> &'static str {
        match self {
            Self::Full => "gate-full",
            Self::Cli => "gate-cli",
        }
    }
    /// Local execution limit in milliseconds, excluding preparation.
    pub const fn budget_ms(self) -> u64 {
        match self {
            Self::Full => 600_000,
            Self::Cli => 300_000,
        }
    }
}

/// A single supervised command, with its original status and separate execution time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    /// Stable step name.
    pub name: String,
    /// Actual process exit code; 124 means watchdog expiry, 130 cancellation.
    pub exit_code: i32,
    /// Monotonic elapsed command time.
    pub elapsed_ms: u64,
    /// Whether the watchdog terminated the process tree.
    pub timed_out: bool,
}

/// A successful report is eligible only after all required steps and snapshot validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// Wire format version.
    pub version: u32,
    /// Source HEAD before the gate (first merge parent for sync).
    pub commit: String,
    /// Exact tested source tree.
    pub tree: String,
    /// Tested integration base.
    pub base: String,
    /// Second parent for a staged upstream sync; absent for ordinary PRs.
    pub merge_parent: Option<String>,
    /// Native host OS, never supplied by a command-line override.
    pub os: Os,
    /// Host architecture, retained for inspection.
    pub arch: String,
    /// Verification scope.
    pub scope: Scope,
    /// Required explanation for CLI-only verification.
    pub ui_independent_reason: Option<String>,
    /// Explicit statement of local UI coverage.
    pub ui_tested: bool,
    /// Rust compiler version.
    pub rustc: String,
    /// Cargo version.
    pub cargo: String,
    /// Nextest version.
    pub nextest: String,
    /// Number of runnable selected tests in the prepared inventory.
    pub expected_tests: u64,
    /// Executed test cases in the fresh JUnit report.
    pub executed_tests: u64,
    /// Explicitly ignored cases, retained as visible coverage limits rather than counted passes.
    pub ignored_tests: Vec<String>,
    /// Whether the source remained exactly the same after verification.
    pub source_unchanged: bool,
    /// Required command outcomes.
    pub stages: Vec<Stage>,
}

/// Whether a string is a Git SHA-1/SHA-256 object ID, not an option or ref expression.
pub fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Report {
    /// Validates completeness, counters, time budgets and successful command outcomes.
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !oid(&self.commit)
            || !oid(&self.tree)
            || !oid(&self.base)
            || self.merge_parent.as_deref().is_some_and(|p| !oid(p))
        {
            return fail("unsupported report version or malformed source object ID");
        }
        if self.arch.is_empty()
            || self.arch.len() > 64
            || self.rustc.is_empty()
            || self.cargo.is_empty()
            || self.rustc.len() > 256
            || self.cargo.len() > 256
            || self.nextest.split_whitespace().nth(1) != Some(NEXTEST_VERSION)
        {
            return fail("missing tool/architecture identity or wrong nextest version");
        }
        if !self.source_unchanged
            || self.expected_tests == 0
            || self.executed_tests != self.expected_tests
        {
            return fail("changed source, empty inventory, or incomplete test execution");
        }
        if self.ignored_tests.len() > 4096 || self.ignored_tests.iter().any(|s| s.len() > 1024) {
            return fail("oversized ignored-test inventory");
        }
        if self.ui_tested != (self.scope == Scope::Full)
            || (self.merge_parent.is_some() && self.scope != Scope::Full)
        {
            return fail("inconsistent UI scope or non-full sync report");
        }
        if self.scope == Scope::Cli
            && !self
                .ui_independent_reason
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty() && s.len() <= 2048)
        {
            return fail("CLI-only verification needs a UI-independence explanation");
        }
        if self.stages.len() != STEPS.len() {
            return fail("missing or duplicate gate steps");
        }
        for name in STEPS {
            let mut steps = self.stages.iter().filter(|s| s.name == name);
            let Some(step) = steps.next() else {
                return fail(format!("missing gate step: {name}"));
            };
            if steps.next().is_some() || step.exit_code != 0 || step.timed_out {
                return fail(format!("unsuccessful gate step: {name}"));
            }
            let limit = match name {
                "tests" => self.scope.budget_ms(),
                "doctests" => 120_000,
                _ => 2_700_000,
            };
            if step.elapsed_ms > limit {
                return fail(format!("gate step exceeded its budget: {name}"));
            }
        }
        Ok(())
    }
    /// Validates a PR snapshot independently of the report's own eligibility claim.
    pub fn validate_pr(&self, head: &str, base: &str, tree: &str) -> Result<()> {
        self.validate()?;
        if self.merge_parent.is_some()
            || self.commit != head
            || self.base != base
            || self.tree != tree
        {
            return fail("local report is stale for this PR head, base or tree");
        }
        Ok(())
    }
}

/// Conservative CLI-only eligibility, including both sides of renames/deletions.
pub fn cli_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> bool {
    let mut any_code = false;
    for path in paths {
        if path.starts_with("apps/beamcraft-cli/src/")
            || path.starts_with("apps/beamcraft-cli/tests/")
        {
            any_code = true;
        } else if !path.ends_with(".md")
            || path.starts_with("docs/contracts/")
            || matches!(path, "AGENTS.md" | "CLAUDE.md")
        {
            return false;
        }
    }
    any_code
}

/// Extracts exactly one bounded machine block, rejecting partial or repeated blocks.
pub fn from_body(body: &str) -> Result<Report> {
    if body.len() > 65_536 || body.matches(BEGIN).count() != 1 {
        return fail("missing or repeated local gate report");
    }
    let (_, rest) = body.split_once(BEGIN).ok_or("missing report marker")?;
    let (json, _) = rest.split_once(END).ok_or("unterminated gate report")?;
    Ok(serde_json::from_str(json)?)
}

/// Inserts/replaces the machine block without discarding any human description.
pub fn with_report(body: &str, report: &Report) -> Result<String> {
    report.validate()?;
    let json = serde_json::to_string_pretty(report)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    let block = format!("{BEGIN}{json}{END}");
    let result = if body.contains(BEGIN) {
        from_body(body)?;
        let (before, rest) = body.split_once(BEGIN).ok_or("missing marker")?;
        let (_, after) = rest.split_once(END).ok_or("missing end marker")?;
        format!("{before}{block}{after}")
    } else {
        format!("{body}\n\n{block}\n")
    };
    if result.len() > 65_536 {
        return fail("PR description plus report exceeds 64 KiB");
    }
    Ok(result)
}
