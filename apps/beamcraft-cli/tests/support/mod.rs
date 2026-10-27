// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Mechanical real-binary fixtures; domain expectations stay in each integration suite.

// Suites that share only the JSON accessors never touch the fixture.
#![allow(dead_code, clippy::panic)]

use beamcraft_json::{Json, Limits};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) struct StoreFixture {
    pub(crate) root: PathBuf,
}

impl StoreFixture {
    pub(crate) fn new(suite: &str) -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(suite);
        fs::create_dir_all(&base).unwrap();
        for _ in 0..1024 {
            let root = base.join(format!(
                "e2e-{}-{}",
                std::process::id(),
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
