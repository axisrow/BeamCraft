// SPDX-License-Identifier: MPL-2.0
//! Real subprocess failures, descendant cleanup and an isolated CLI-only workspace.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_test_gate::{capture, local, process, report::Report};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "beamcraft-gate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(role: &str) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--ignored", "--exact", "process_fixture", "--nocapture"])
        .env("GATE_FIXTURE_ROLE", role);
    cmd
}

#[test]
#[ignore = "subprocess fixture, invoked explicitly by the watchdog tests"]
fn process_fixture() {
    match std::env::var("GATE_FIXTURE_ROLE").unwrap().as_str() {
        "fail" => panic!("intentional ordinary-test failure"),
        "pass" => {}
        "lock" => {
            use std::io::{Read, Write};
            println!("GATE-REQUEST");
            std::io::stdout().flush().unwrap();
            let _held = process::lock().unwrap();
            println!("GATE-LOCKED");
            std::io::stdout().flush().unwrap();
            std::io::stdin().read_exact(&mut [0_u8]).unwrap();
        }
        "parent" => {
            let mut cmd = fixture("child");
            #[cfg(unix)]
            if std::env::var_os("GATE_FIXTURE_DETACH").is_some() {
                use std::os::unix::process::CommandExt;
                cmd.process_group(0);
            }
            let mut child = cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            child.wait().unwrap();
        }
        "child" => {
            std::fs::write(
                std::env::var_os("GATE_FIXTURE_PID").unwrap(),
                std::process::id().to_string(),
            )
            .unwrap();
            loop {
                std::thread::park();
            }
        }
        _ => panic!("unknown fixture role"),
    }
}

fn running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let result = capture(Command::new("ps").args(["-p", &pid.to_string(), "-o", "stat="]));
        result.is_ok_and(|s| !s.trim().is_empty() && !s.trim_start().starts_with('Z'))
    }
    #[cfg(windows)]
    {
        capture(Command::new("tasklist").args([
            "/FI",
            &format!("PID eq {pid}"),
            "/FO",
            "CSV",
            "/NH",
        ]))
        .is_ok_and(|s| s.contains(&format!("\",\"{pid}\",")))
    }
}

fn assert_descendant_stopped(path: &Path) {
    let pid: u32 = std::fs::read_to_string(path)
        .expect("fixture child reached its ready point")
        .parse()
        .unwrap();
    let leaked = running(pid);
    if leaked {
        // Keep a failing regression from leaking its own fixture.
        #[cfg(unix)]
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
        #[cfg(windows)]
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();
    }
    assert!(!leaked, "descendant {pid} survived the gate timeout");
}

#[test]
fn watchdog_kills_the_parent_and_a_separately_grouped_descendant() {
    let temp = Temp::new();
    let ready = temp.0.join("pid");
    let mut cmd = fixture("parent");
    cmd.env("GATE_FIXTURE_PID", &ready)
        .env("GATE_FIXTURE_DETACH", "1");
    let result = process::run("deadline", &mut cmd, &temp.0, Duration::from_secs(2)).unwrap();
    assert!(result.timed_out);
    assert_eq!(result.exit_code, 124);
    assert_descendant_stopped(&ready);
    assert!(temp.0.join("deadline.stderr.log").is_file());
}

#[test]
fn a_later_success_does_not_change_an_earlier_real_failure() {
    let temp = Temp::new();
    let failed = process::run(
        "ordinary",
        &mut fixture("fail"),
        &temp.0,
        Duration::from_secs(15),
    )
    .unwrap();
    let passed = process::run(
        "doctests",
        &mut fixture("pass"),
        &temp.0,
        Duration::from_secs(15),
    )
    .unwrap();
    assert_ne!(failed.exit_code, 0);
    assert_eq!(passed.exit_code, 0);
    assert!(
        std::fs::read_to_string(temp.0.join("ordinary.stdout.log"))
            .unwrap()
            .contains("FAILED")
    );
    assert!(
        std::fs::read_to_string(temp.0.join("doctests.stdout.log"))
            .unwrap()
            .contains("ok")
    );
}

#[test]
fn two_worktrees_share_a_crash_safe_native_file_lock() {
    use std::io::{BufRead, Write};
    fn line_until(reader: &mut impl BufRead, marker: &str) {
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).unwrap() > 0,
                "missing handshake {marker}"
            );
            if line.contains(marker) {
                break;
            }
        }
    }
    let temp = Temp::new();
    let a = temp.0.join("worktree-a");
    let b = temp.0.join("worktree-b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let mut first = fixture("lock")
        .current_dir(&a)
        .env("XDG_CACHE_HOME", temp.0.join("cache"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut first_out = std::io::BufReader::new(first.stdout.take().unwrap());
    line_until(&mut first_out, "GATE-LOCKED");
    let mut second = fixture("lock")
        .current_dir(&b)
        .env("XDG_CACHE_HOME", temp.0.join("cache"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut second_out = std::io::BufReader::new(second.stdout.take().unwrap());
    line_until(&mut second_out, "GATE-REQUEST");
    assert!(second.try_wait().unwrap().is_none());
    first.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(first.wait().unwrap().success());
    line_until(&mut second_out, "GATE-LOCKED");
    second.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(second.wait().unwrap().success());
}

#[test]
fn a_broken_real_doctest_returns_a_failed_stage() {
    let temp = Temp::new();
    std::fs::create_dir(temp.0.join("src")).unwrap();
    std::fs::write(temp.0.join("Cargo.toml"), "[package]\nname=\"broken-gate-doctest\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[workspace]\n").unwrap();
    std::fs::write(
        temp.0.join("src/lib.rs"),
        "/// ```\n/// a_missing_function();\n/// ```\npub fn example() {}\n",
    )
    .unwrap();
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&temp.0)
        .args(["test", "--doc", "--offline", "--no-fail-fast"])
        .env("CARGO_TARGET_DIR", temp.0.join("target"));
    let result = process::run("doctests", &mut cmd, &temp.0, Duration::from_secs(180)).unwrap();
    assert_ne!(result.exit_code, 0);
    assert!(!result.timed_out);
    assert!(
        std::fs::read_to_string(temp.0.join("doctests.stdout.log"))
            .unwrap()
            .contains("FAILED")
    );
    let stderr = std::fs::read_to_string(temp.0.join("doctests.stderr.log")).unwrap();
    // A real doctest failure is never the vacuous no-library-targets refusal.
    assert!(!local::no_library_targets(&stderr));
}

#[test]
fn a_bin_only_selection_has_no_doctests_to_run() {
    let temp = Temp::new();
    std::fs::create_dir(temp.0.join("src")).unwrap();
    std::fs::write(temp.0.join("Cargo.toml"), "[package]\nname=\"bin-only-gate-doctest\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[workspace]\n").unwrap();
    std::fs::write(temp.0.join("src/main.rs"), "fn main() {}\n").unwrap();
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&temp.0)
        .args(["test", "--doc", "--offline", "--no-fail-fast"])
        .env("CARGO_TARGET_DIR", temp.0.join("target"));
    let result = process::run("doctests", &mut cmd, &temp.0, Duration::from_secs(180)).unwrap();
    assert_ne!(result.exit_code, 0);
    assert!(!result.timed_out);
    let stderr = std::fs::read_to_string(temp.0.join("doctests.stderr.log")).unwrap();
    assert!(local::no_library_targets(&stderr));
}

#[test]
fn nextest_individual_and_global_timeouts_stop_real_descendants() {
    let temp = Temp::new();
    let root = temp.0.join("fixture");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"nextest-timeout-fixture\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[workspace]\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), r#"
#[cfg(test)]
mod tests {
    #[test]
    fn hangs() {
        let _child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "tests::child"])
            .spawn().unwrap();
        loop { std::thread::park(); }
    }
    #[test]
    #[ignore]
    fn child() {
        std::fs::write(std::env::var_os("GATE_FIXTURE_PID").unwrap(), std::process::id().to_string()).unwrap();
        loop { std::thread::park(); }
    }
}
"#).unwrap();
    capture(
        Command::new("cargo")
            .current_dir(&root)
            .args(["generate-lockfile", "--offline"]),
    )
    .unwrap();
    for (name, slow, global) in [("individual", "1s", "30s"), ("global", "30s", "1s")] {
        let cfg = temp.0.join(format!("{name}.toml"));
        std::fs::write(&cfg, format!("[profile.default]\nslow-timeout={{period=\"{slow}\",terminate-after=1,grace-period=\"0s\"}}\nglobal-timeout=\"{global}\"\n")).unwrap();
        let ready = temp.0.join(format!("{name}.pid"));
        let mut cmd = Command::new("cargo");
        cmd.current_dir(&root)
            .args([
                "nextest",
                "run",
                "--locked",
                "--offline",
                "--profile",
                "default",
                "--user-config-file",
                "none",
                "-E",
                "test(/^tests::hangs$/)",
                "--no-fail-fast",
                "--config-file",
            ])
            .arg(cfg)
            .env("NEXTEST_PROFILE", "deliberately-unknown-parent-profile")
            .env("GATE_FIXTURE_PID", &ready)
            .env("CARGO_TARGET_DIR", root.join("target"));
        let result = process::run(name, &mut cmd, &temp.0, Duration::from_secs(180)).unwrap();
        assert_ne!(result.exit_code, 0);
        assert!(!result.timed_out, "nextest's own budget must fire first");
        assert_descendant_stopped(&ready);
    }
}

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn git(root: &Path, args: &[&str]) -> String {
    beamcraft_test_gate::git(root, args).unwrap()
}

/// Waits bounded for a directory written by the fixture gate's grandchild cargo
/// processes before handing it out. Hosted shards once read it absent for the
/// whole grace after a fully successful gate; `None` means the directory never
/// became listable within `limit`, which stays a failure.
fn dir_built_within(path: &Path, limit: Duration) -> Option<std::fs::ReadDir> {
    let deadline = Instant::now() + limit;
    loop {
        match std::fs::read_dir(path) {
            Ok(entries) => return Some(entries),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return None,
        }
    }
}

/// Reports whether any file under the fixture gate's isolated target tree is
/// named after the UI crate. The walk covers whatever layout the running
/// cargo produced: hosted cargos place test and doctest executables under
/// per-package build directories (`debug/build/<pkg>/<hash>/out`), so a fixed
/// `debug/deps` listing never sees the artifacts it must judge. The budget
/// bounds the walk for a hostile-looking tree; the fixture's own gate builds
/// a few hundred entries at most.
fn ui_artifacts_under(root: &Path, budget: usize) -> bool {
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > budget {
                return true;
            }
            let name = entry.file_name();
            if name.to_string_lossy().contains("beamcraft_ui") {
                return true;
            }
            if entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
                stack.push(dir.join(name));
            }
        }
    }
    false
}

#[test]
fn the_fixture_artifact_reader_waits_out_a_late_directory_and_reports_an_absent_one() {
    let temp = Temp::new();
    let late = temp.0.join("late").join("deps");
    std::fs::create_dir_all(late.parent().unwrap()).unwrap();
    let writer = std::thread::spawn({
        let late = late.clone();
        move || {
            std::thread::sleep(Duration::from_millis(300));
            std::fs::create_dir_all(&late).unwrap();
        }
    });
    assert!(dir_built_within(&late, Duration::from_secs(10)).is_some());
    writer.join().unwrap();
    assert!(
        dir_built_within(&temp.0.join("absent"), Duration::from_millis(300)).is_none(),
        "a directory that never appears must stay a failure"
    );
}

#[test]
fn the_ui_artifact_walk_finds_a_buried_name_and_a_clean_tree_stays_clean() {
    let temp = Temp::new();
    let buried = temp.0.join("debug/build/beamcraft-cli/hash/out");
    std::fs::create_dir_all(&buried).unwrap();
    std::fs::write(buried.join("beamcraft_ui-abc123.rlib"), b"").unwrap();
    assert!(
        ui_artifacts_under(&temp.0.join("debug"), 10_000),
        "a UI artifact buried in a per-package build directory must be found"
    );
    let clean = temp.0.join("clean/debug/build/beamcraft-cli/hash/out");
    std::fs::create_dir_all(&clean).unwrap();
    std::fs::write(clean.join("beamcraft_cli-abc123"), b"").unwrap();
    assert!(
        !ui_artifacts_under(&temp.0.join("clean"), 10_000),
        "CLI-named artifacts must not trip the UI check"
    );
}

#[test]
fn cli_gate_runs_real_tests_and_doctests_without_building_the_ui() {
    let temp = Temp::new();
    let root = temp.0.join("repo");
    std::fs::create_dir(&root).unwrap();
    write(
        &root,
        "Cargo.toml",
        "[workspace]\nresolver=\"3\"\nmembers=[\"apps/beamcraft-cli\",\"apps/beamcraft-ui\"]\n",
    );
    write(&root, ".gitignore", "/target\n");
    write(&root, "README.md", "source snapshot fixture\n");
    write(
        &root,
        "apps/beamcraft-cli/Cargo.toml",
        "[package]\nname=\"beamcraft-cli\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[[bin]]\nname=\"beamcraft-test-gate\"\npath=\"src/main.rs\"\n",
    );
    write(&root, "apps/beamcraft-cli/src/main.rs", "fn main() {}\n");
    write(
        &root,
        "apps/beamcraft-cli/src/lib.rs",
        "/// Double a number.\n/// ```\n/// assert_eq!(beamcraft_cli::twice(2), 4);\n/// ```\npub fn twice(value: i32) -> i32 {\n    value * 2\n}\n",
    );
    write(
        &root,
        "apps/beamcraft-cli/tests/behaviour.rs",
        "#[test]\nfn doubles() {\n    assert_eq!(beamcraft_cli::twice(3), 6);\n}\n",
    );
    write(
        &root,
        "apps/beamcraft-ui/Cargo.toml",
        "[package]\nname=\"beamcraft-ui\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    );
    write(
        &root,
        "apps/beamcraft-ui/src/lib.rs",
        "pub fn ui_marker() {}\n",
    );
    write(
        &root,
        "apps/beamcraft-ui/build.rs",
        "fn main() {\n    panic!(\"CLI gate must never build the UI\");\n}\n",
    );
    let config = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.config/nextest.toml"),
    )
    .unwrap();
    write(
        &root,
        ".config/nextest.toml",
        config.split("[profile.fast]").next().unwrap(),
    );
    capture(
        Command::new("cargo")
            .current_dir(&root)
            .args(["generate-lockfile", "--offline"]),
    )
    .unwrap();
    git(&root, &["init", "-b", "master"]);
    git(&root, &["config", "user.name", "Gate test"]);
    git(&root, &["config", "user.email", "gate@example.invalid"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial fixture"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    write(
        &root,
        "apps/beamcraft-cli/tests/additional.rs",
        "#[test]\nfn another_cli_case() {\n    assert_eq!(beamcraft_cli::twice(4), 8);\n}\n",
    );
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "CLI-only fixture change"]);
    // A workspace binary with the running wrapper's name must not replace or lock it.
    let wrapper = root.join(format!(
        "target/debug/beamcraft-test-gate{}",
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_beamcraft-test-gate"), &wrapper).unwrap();
    let mut cmd = Command::new(&wrapper);
    cmd.current_dir(&root)
        .args([
            "check",
            "--scope",
            "cli",
            "--base",
            &base,
            "--ui-independent-reason",
            "Only a CLI case was added; the UI contract is unchanged.",
            "--report",
            "target/run/report.json",
        ])
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("XDG_CACHE_HOME", temp.0.join("cache"));
    let result = process::run("cli-gate", &mut cmd, &temp.0, Duration::from_secs(180)).unwrap();
    let stderr = std::fs::read_to_string(temp.0.join("cli-gate.stderr.log")).unwrap();
    assert_eq!(result.exit_code, 0, "{stderr}");
    assert_eq!(
        std::fs::read(&wrapper).unwrap(),
        std::fs::read(env!("CARGO_BIN_EXE_beamcraft-test-gate")).unwrap()
    );
    let report: Report = serde_json::from_str(
        &std::fs::read_to_string(root.join("target/run/report.json")).unwrap(),
    )
    .unwrap();
    report.validate().unwrap();
    assert!(!report.ui_tested);
    assert_eq!(report.executed_tests, 2);
    assert!(
        std::fs::read_to_string(root.join("target/run/doctests.stdout.log"))
            .unwrap()
            .contains("1 passed")
    );
    let isolated_target = root.join("target/test-gate/target");
    dir_built_within(&isolated_target, Duration::from_secs(10))
        .expect("the fixture gate built its isolated target directory");
    assert!(
        !ui_artifacts_under(&isolated_target, 100_000),
        "the fixture gate's isolated target must hold no beamcraft_ui artifacts"
    );
    assert!(local::snapshot(&root, false).is_ok());
    // A green test process must not certify source changed by one of its tests.
    write(
        &root,
        "apps/beamcraft-cli/tests/mutation.rs",
        "#[test]\nfn mutation() {\n    let path = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\"));\n    std::fs::write(path.join(\"../../README.md\"), \"changed\").unwrap();\n}\n",
    );
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "source mutation fixture"]);
    let mut cmd = Command::new(&wrapper);
    cmd.current_dir(&root)
        .args([
            "check",
            "--scope",
            "cli",
            "--base",
            &base,
            "--ui-independent-reason",
            "Only CLI tests changed; this fixture must detect a mutated source snapshot.",
            "--report",
            "target/mutating/report.json",
        ])
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("XDG_CACHE_HOME", temp.0.join("cache"));
    let failed = process::run("mutated-gate", &mut cmd, &temp.0, Duration::from_secs(180)).unwrap();
    assert_ne!(failed.exit_code, 0);
    let changed: Report = serde_json::from_str(
        &std::fs::read_to_string(root.join("target/mutating/report.json")).unwrap(),
    )
    .unwrap();
    assert!(!changed.source_unchanged);
    assert!(changed.validate().is_err());
}

#[test]
fn a_cli_gate_passes_vacuously_when_the_package_has_no_library_target() {
    let temp = Temp::new();
    let root = temp.0.join("repo");
    std::fs::create_dir(&root).unwrap();
    write(
        &root,
        "Cargo.toml",
        "[workspace]\nresolver=\"3\"\nmembers=[\"apps/beamcraft-cli\"]\n",
    );
    write(&root, ".gitignore", "/target\n");
    write(&root, "README.md", "source snapshot fixture\n");
    write(
        &root,
        "apps/beamcraft-cli/Cargo.toml",
        "[package]\nname=\"beamcraft-cli\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[[bin]]\nname=\"fixture-cli\"\npath=\"src/main.rs\"\n",
    );
    write(&root, "apps/beamcraft-cli/src/main.rs", "fn main() {}\n");
    write(
        &root,
        "apps/beamcraft-cli/tests/behaviour.rs",
        "#[test]\nfn doubles() {\n    assert_eq!(2 + 2, 4);\n}\n",
    );
    let config = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.config/nextest.toml"),
    )
    .unwrap();
    write(
        &root,
        ".config/nextest.toml",
        config.split("[profile.fast]").next().unwrap(),
    );
    capture(
        Command::new("cargo")
            .current_dir(&root)
            .args(["generate-lockfile", "--offline"]),
    )
    .unwrap();
    git(&root, &["init", "-b", "master"]);
    git(&root, &["config", "user.name", "Gate test"]);
    git(&root, &["config", "user.email", "gate@example.invalid"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial fixture"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    write(
        &root,
        "apps/beamcraft-cli/tests/additional.rs",
        "#[test]\nfn another_cli_case() {\n    assert_eq!(3 * 3, 9);\n}\n",
    );
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "CLI-only fixture change"]);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_beamcraft-test-gate"));
    cmd.current_dir(&root)
        .args([
            "check",
            "--scope",
            "cli",
            "--base",
            &base,
            "--ui-independent-reason",
            "Only a CLI case was added; the UI contract is unchanged.",
            "--report",
            "target/run/report.json",
        ])
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("XDG_CACHE_HOME", temp.0.join("cache"));
    let result = process::run("cli-gate", &mut cmd, &temp.0, Duration::from_secs(180)).unwrap();
    let stderr = std::fs::read_to_string(temp.0.join("cli-gate.stderr.log")).unwrap();
    assert_eq!(result.exit_code, 0, "{stderr}");
    assert!(stderr.contains("no doctests to run"), "{stderr}");
    let report: Report = serde_json::from_str(
        &std::fs::read_to_string(root.join("target/run/report.json")).unwrap(),
    )
    .unwrap();
    report.validate().unwrap();
    assert!(!report.ui_tested);
    assert_eq!(report.executed_tests, 2);
    let refusal = std::fs::read_to_string(root.join("target/run/doctests.stderr.log")).unwrap();
    assert!(local::no_library_targets(&refusal), "{refusal}");
}
