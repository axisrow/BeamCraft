// SPDX-License-Identifier: MPL-2.0
//! Command-line entry point for local checks, publication and CI execution.

use beamcraft_test_gate::{Result, fail, git, github, local, process, read, report::Scope};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

fn usage() {
    eprintln!(
        "beamcraft-test-gate check [--scope full|cli] [--base REF] [--ui-independent-reason TEXT] [--report PATH]\n\
beamcraft-test-gate publish --report PATH --pr NUMBER [--repo OWNER/NAME]\n\
beamcraft-test-gate ci-plan\n\
beamcraft-test-gate ci-archive --output DIR\n\
beamcraft-test-gate ci-test --archive PATH --partition I/N --output DIR\n\
beamcraft-test-gate sync-trailer|verify-sync --report PATH"
    );
}

fn absolute(root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let action = args.next().ok_or("missing gate command")?;
    if action == "--help" || action == "help" {
        usage();
        return Ok(());
    }
    let cwd = std::env::current_dir()?;
    let root = PathBuf::from(git(&cwd, &["rev-parse", "--show-toplevel"])?);
    let mut scope = Scope::Full;
    let mut base = "origin/master".to_owned();
    let mut reason = None;
    let mut report = None;
    let mut pr = None;
    let mut repo = None;
    let mut output = None;
    let mut archive = None;
    let mut partition = None;
    let mut staged_merge = false;
    let mut seen = std::collections::BTreeSet::new();
    while let Some(key) = args.next() {
        if !seen.insert(key.clone()) {
            return fail(format!("repeated option: {key}"));
        }
        if key == "--staged-merge" && action == "check" {
            staged_merge = true;
            continue;
        }
        let value = args.next().ok_or("option requires a value")?;
        match (action.as_str(), key.as_str()) {
            ("check", "--scope") => {
                scope = match value.as_str() {
                    "full" => Scope::Full,
                    "cli" => Scope::Cli,
                    _ => return fail("scope must be full or cli"),
                }
            }
            ("check", "--base") => base = value,
            ("check", "--ui-independent-reason") => reason = Some(value),
            ("check" | "publish" | "sync-trailer" | "verify-sync", "--report") => {
                report = Some(absolute(&root, &value))
            }
            ("publish", "--pr") => pr = Some(value.parse::<u64>()?),
            ("publish", "--repo") => repo = Some(value),
            ("ci-archive" | "ci-test", "--output") => output = Some(absolute(&root, &value)),
            ("ci-test", "--archive") => archive = Some(absolute(&root, &value)),
            ("ci-test", "--partition") => partition = Some(value),
            _ => {
                return fail(format!(
                    "unknown option {key} for {action}; test filters/platform overrides are not allowed"
                ));
            }
        }
    }
    match action.as_str() {
        "check" => {
            let path = local::check(
                &root,
                local::Options {
                    scope,
                    base,
                    reason,
                    report,
                    staged_merge,
                },
            )?;
            println!("{}", path.display());
        }
        "publish" => {
            let repo = match repo {
                Some(value) => value,
                None => github::repository(&root)?,
            };
            github::publish(
                &root,
                &repo,
                pr.ok_or("--pr is required")?,
                &report.ok_or("--report is required")?,
            )?;
        }
        "ci-plan" => {
            let event = std::env::var("GITHUB_EVENT_NAME")?;
            let payload =
                serde_json::from_str(&read(Path::new(&std::env::var("GITHUB_EVENT_PATH")?))?)?;
            let plan = github::plan(
                &root,
                &event,
                &payload,
                &std::env::var("GITHUB_SHA")?,
                &std::env::var("GITHUB_REPOSITORY")?,
            )?;
            let serialized = serde_json::to_string(&plan.oses)?;
            if let Some(path) = std::env::var_os("GITHUB_OUTPUT") {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
                writeln!(file, "oses={serialized}\nprimary={}", plan.primary)?;
            }
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        "ci-verdict" => {
            let needs = serde_json::from_str(&std::env::var("GATE_NEEDS")?)?;
            let oses = serde_json::from_str::<Vec<String>>(&std::env::var("GATE_OSES")?)?;
            github::verdict(&needs, &oses, &std::env::var("GITHUB_EVENT_NAME")?)?;
            println!("CI verdict: required checks passed on both selected operating systems");
        }
        "ci-archive" => local::ci_archive(&root, &output.ok_or("--output is required")?)?,
        "ci-test" => local::ci_test(
            &root,
            &output.ok_or("--output is required")?,
            &archive.ok_or("--archive is required")?,
            &partition.ok_or("--partition is required")?,
        )?,
        "sync-trailer" => println!(
            "{}",
            github::sync_trailer(&root, &report.ok_or("--report is required")?)?
        ),
        "verify-sync" => github::verify_sync(&root, &report.ok_or("--report is required")?)?,
        _ => {
            usage();
            return fail("unknown gate command");
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = ctrlc::set_handler(|| process::CANCELLED.store(true, Ordering::Relaxed)) {
        eprintln!("gate: cannot install cancellation handler: {error}");
        std::process::exit(1);
    }
    if let Err(error) = run() {
        eprintln!("gate: {error}");
        std::process::exit(if process::CANCELLED.load(Ordering::Relaxed) {
            130
        } else {
            1
        });
    }
}
