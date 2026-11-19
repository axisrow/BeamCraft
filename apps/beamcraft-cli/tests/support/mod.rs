// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Mechanical real-binary fixtures; domain expectations stay in each integration suite.

// Suites that share only the JSON accessors never touch the fixture. Test-only setup code fails
// the calling test on any setup error, as each suite's own helpers do.
#![allow(dead_code, clippy::panic, clippy::unwrap_used)]

use beamcraft_json::{Json, Limits};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// One random value per process boot, for fixture directory names. A pid alone is not unique
/// across runs — the OS recycles pids, and a directory left behind by a killed process then
/// collides with a later run whose counter restarted at zero (a known collision class). A
/// process-wide random nonce cannot collide with any other boot.
pub(crate) fn boot_nonce() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    static NONCE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *NONCE.get_or_init(|| RandomState::new().hash_one(0u64))
}

pub(crate) struct StoreFixture {
    pub(crate) root: PathBuf,
}

impl StoreFixture {
    pub(crate) fn new(suite: &str) -> Self {
        steel::layer::install();
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(suite);
        fs::create_dir_all(&base).unwrap();
        for _ in 0..1024 {
            let root = base.join(format!(
                "e2e-{}-{}",
                boot_nonce(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let fixture = Self { root };
                    fs::create_dir(fixture.root.join("store")).unwrap();
                    return fixture;
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot reserve fixture directory: {error}"),
            }
        }
        panic!("fixture directory retry limit exhausted");
    }

    pub(crate) fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        Self::run_in(self.root.join("store").to_str().unwrap(), command, extra)
    }

    pub(crate) fn run_in(store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        let mut args = vec![command, "--store", store];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
}

impl Drop for StoreFixture {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One scripted command line: its tokens and the process exit the answer must carry
/// (`0` for an accepted command, `2` for a typed refusal, `130` for a cancellation).
#[derive(Clone)]
pub(crate) struct Scripted {
    args: Vec<String>,
    exit: i32,
}

impl Scripted {
    fn line(args: &[&str], exit: i32) -> Self {
        Self {
            args: args.iter().map(|token| (*token).to_owned()).collect(),
            exit,
        }
    }

    /// A command expected to succeed.
    pub(crate) fn ok(args: &[&str]) -> Self {
        Self::line(args, 0)
    }

    /// A command expected to be refused.
    pub(crate) fn refused(args: &[&str]) -> Self {
        Self::line(args, 2)
    }

    /// A command expected to answer `cancelled`.
    pub(crate) fn cancelled(args: &[&str]) -> Self {
        Self::line(args, 130)
    }
}

impl StoreFixture {
    /// Runs the lines as one `run-script` invocation over the store at `dir`, asserting
    /// the summary's outcome and every line's exit code; returns each command's reply in
    /// order. A script must not end with an open batch, so the summary carries exactly
    /// one reply entry per line.
    pub(crate) fn script(&self, dir: &Path, lines: &[Scripted]) -> Vec<Json> {
        script_lines(&self.root, dir.to_str().unwrap(), lines)
    }
}

/// The shared `run-script` driver: encodes the lines as one script file under `root`,
/// runs one real invocation over the store `store`, asserts the summary's outcome and
/// every line's exit code, and returns each command's reply in order.
pub(crate) fn script_lines(root: &Path, store: &str, lines: &[Scripted]) -> Vec<Json> {
    run_script(root, store, lines, &[])
}

/// The same driver for suites that strip the ambient `BEAMCRAFT_CONFIG_DIR` from every
/// spawn they make: the scripted invocation sees the same clean environment the suite's
/// direct spawns see.
pub(crate) fn script_lines_without_config(
    root: &Path,
    store: &str,
    lines: &[Scripted],
) -> Vec<Json> {
    run_script(root, store, lines, &["BEAMCRAFT_CONFIG_DIR"])
}

fn run_script(root: &Path, store: &str, lines: &[Scripted], strip_env: &[&str]) -> Vec<Json> {
    let file = root.join(format!("script-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    let entries = lines
        .iter()
        .map(|line| {
            Json::Array(
                line.args
                    .iter()
                    .map(|token| Json::text(token.as_str()))
                    .collect(),
            )
        })
        .collect();
    fs::write(
        &file,
        Json::Array(entries).encode(&Limits::DEFAULT).unwrap(),
    )
    .unwrap();
    let mut invocation = Command::new(env!("CARGO_BIN_EXE_beamcraft"));
    for name in strip_env {
        invocation.env_remove(name);
    }
    let output = invocation
        .args([
            "run-script",
            "--store",
            store,
            "--file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "run-script itself failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let summary = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    assert_eq!(text(&summary, "outcome"), "script_complete");
    assert_eq!(
        at(&summary, &["commands"]).unsigned().unwrap(),
        lines.len() as u64,
        "one reply entry per scripted line"
    );
    let replies = list(&summary, "replies");
    let exits: Vec<i32> = replies
        .iter()
        .map(|entry| at(entry, &["exit"]).unsigned().unwrap() as i32)
        .collect();
    let expected: Vec<i32> = lines.iter().map(|line| line.exit).collect();
    assert_eq!(exits, expected, "the summary's exits follow the script");
    replies
        .iter()
        .map(|entry| at(entry, &["reply"]).clone())
        .collect()
}

/// The string value at `key`, failing the test when the key is absent or not a string.
pub(crate) fn text(json: &Json, key: &str) -> String {
    json.get(key)
        .unwrap_or_else(|_| panic!("{key} missing from {json:?}"))
        .string()
        .unwrap()
        .to_owned()
}

/// The value at a `path` of nested keys, failing the test when a step is absent.
pub(crate) fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}

/// The array at `key`, failing the test when the key is absent or not an array.
pub(crate) fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
