// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Original synthetic test fixtures for BeamCraft (task F07).
//!
//! This development tool generates a deterministic set of contributor-original fixture
//! descriptions (rectangular plates and straight rectangular members, plus negative inputs),
//! writes them with a provenance manifest that records full SHA-256 digests, verifies a fixture
//! directory byte for byte, and validates single descriptions.
//!
//! It never computes expected quantities. Analytical expectations are hand-derived in
//! `docs/fixtures/expectations.md`, stored in `tests/fixtures/expectations/f07.json` and checked
//! independently by this package's `tests/expectations.rs`, which recomputes them from the
//! fixture inputs with its own closed-form vector algebra.
//!
//! The command-line front end is the `beamcraft-fixtures` binary.

pub mod catalog;
pub mod fixture;
pub mod json;
pub mod sha256;

use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use catalog::{Case, Expected};
use json::{Json, ParseLimits};

/// Format identifier of the manifest.
pub const MANIFEST_FORMAT: &str = "beamcraft.fixture-manifest/1";
/// Version of the F07 fixture set. Increase it whenever a case is added, removed or changed.
pub const FIXTURE_SET_VERSION: u32 = 1;
/// Manifest file name inside the fixture directory.
pub const MANIFEST_PATH: &str = "manifest.json";
/// Repository location of the committed fixture set.
pub const REPOSITORY_DIR: &str = "tests/fixtures/f07";
/// Repository location of the hand-derived expectations.
pub const EXPECTATIONS_PATH: &str = "tests/fixtures/expectations/f07.json";
/// Repository location of the written derivations.
pub const DERIVATION_PATH: &str = "docs/fixtures/expectations.md";
/// Author recorded for every fixture.
pub const AUTHOR: &str = "BeamCraft contributors";
/// SPDX licence of every fixture.
pub const LICENCE: &str = "MPL-2.0";

/// Largest file the verifier reads, in bytes.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// Most files the generator may produce.
const MAX_FILES: usize = 256;
/// Most bytes the generator may produce in total.
const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
/// Most directory entries examined before giving up.
const MAX_DIR_ENTRIES: usize = 1024;
/// Most problems reported by one verification.
const MAX_PROBLEMS: usize = 256;

const MANIFEST_LIMITS: ParseLimits = ParseLimits {
    max_bytes: MAX_FILE_BYTES,
    max_depth: 8,
    max_values: 16_384,
    max_string_bytes: 512,
    max_number_chars: 32,
    max_members: 64,
};

/// A failure of the generator or verifier itself (not a fixture rejection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A filesystem operation failed.
    Io {
        /// The path involved.
        path: String,
        /// The operating-system message.
        message: String,
    },
    /// The output directory holds entries the generator does not own.
    ForeignEntries(Vec<String>),
    /// The output would exceed a generator bound.
    Limit(String),
    /// The catalog could not be built.
    Catalog(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io { path, message } => write!(f, "{path}: {message}"),
            Error::ForeignEntries(entries) => write!(
                f,
                "refusing to write into a directory with entries the generator does not own: {}",
                entries.join(", ")
            ),
            Error::Limit(what) => write!(f, "generator limit exceeded: {what}"),
            Error::Catalog(what) => write!(f, "catalog error: {what}"),
        }
    }
}

impl std::error::Error for Error {}

fn io_error(path: &Path, e: &std::io::Error) -> Error {
    Error::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    }
}

/// The complete generated fixture set: every case plus the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureSet {
    /// Cases in catalog order.
    pub cases: Vec<Case>,
    /// The manifest bytes.
    pub manifest: Vec<u8>,
}

impl FixtureSet {
    /// Every output file as (relative path, bytes): the cases in order, then the manifest.
    pub fn files(&self) -> Vec<(&str, &[u8])> {
        self.cases
            .iter()
            .map(|c| (c.path, c.bytes.as_slice()))
            .chain(std::iter::once((MANIFEST_PATH, self.manifest.as_slice())))
            .collect()
    }
}

/// Generates the whole fixture set in memory. The result is identical on every run and platform.
pub fn generate() -> Result<FixtureSet, Error> {
    let cases = catalog::cases().map_err(Error::Catalog)?;
    if cases.len() >= MAX_FILES {
        return Err(Error::Limit(format!("{} cases", cases.len())));
    }
    let manifest = json::to_pretty_string(&manifest(&cases)).into_bytes();
    let total = cases
        .iter()
        .map(|c| c.bytes.len())
        .fold(manifest.len(), usize::saturating_add);
    if total > MAX_TOTAL_BYTES {
        return Err(Error::Limit(format!("{total} bytes of output")));
    }
    Ok(FixtureSet { cases, manifest })
}

fn s(v: &str) -> Json {
    Json::String(v.to_owned())
}

fn obj(members: Vec<(&str, Json)>) -> Json {
    Json::Object(
        members
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
    )
}

fn manifest(cases: &[Case]) -> Json {
    let entries = cases
        .iter()
        .map(|case| {
            let mut m = vec![
                ("id", s(case.id)),
                ("path", s(case.path)),
                // Fixture files are far below 2^53 bytes, so the conversion is exact.
                ("bytes", Json::Number(case.bytes.len() as f64)),
                ("sha256", s(&sha256::hex_digest(&case.bytes))),
                ("support", s(case.support.as_str())),
            ];
            match case.expected {
                Expected::Accept => m.push(("expected_outcome", s("accept"))),
                Expected::Reject(code) => {
                    m.push(("expected_outcome", s("reject")));
                    m.push(("expected_error", s(code.as_str())));
                }
            }
            m.extend([
                (
                    "expectation",
                    s(&format!("{EXPECTATIONS_PATH}#{}", case.id)),
                ),
                ("author", s(AUTHOR)),
                ("origin", s("contributor-original synthetic data")),
                ("licence", s(LICENCE)),
                ("summary", s(case.summary)),
            ]);
            obj(m)
        })
        .collect();

    obj(vec![
        ("format", s(MANIFEST_FORMAT)),
        ("fixture_set", s("f07")),
        (
            "fixture_set_version",
            Json::Number(f64::from(FIXTURE_SET_VERSION)),
        ),
        ("fixture_format", s(fixture::FORMAT)),
        ("location", s(REPOSITORY_DIR)),
        (
            "generator",
            obj(vec![
                ("name", s(env!("CARGO_PKG_NAME"))),
                ("version", s(env!("CARGO_PKG_VERSION"))),
                ("source", s("tools/fixtures")),
                (
                    "command",
                    s("cargo run -p beamcraft-fixtures -- generate --out tests/fixtures/f07"),
                ),
            ]),
        ),
        (
            "units",
            obj(vec![
                ("length", s("mm")),
                ("angle", s("deg")),
                ("density", s("kg/m3")),
                ("volume", s("mm3")),
                ("mass", s("kg")),
            ]),
        ),
        (
            "provenance",
            obj(vec![
                ("author", s(AUTHOR)),
                ("origin", s("contributor-original synthetic data, F07")),
                ("licence", s(LICENCE)),
                (
                    "excluded_sources",
                    s("no vendor, product, catalogue, sample-model or observation-note data"),
                ),
            ]),
        ),
        (
            "expected_results",
            obj(vec![
                ("data", s(EXPECTATIONS_PATH)),
                ("derivation", s(DERIVATION_PATH)),
                (
                    "method",
                    s("hand-derived closed-form analytical values; not produced by this generator"),
                ),
                (
                    "oracle",
                    s("cargo test -p beamcraft-fixtures --test expectations"),
                ),
            ]),
        ),
        ("fixtures", Json::Array(entries)),
    ])
}

fn join(dir: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(dir.to_path_buf(), |p, part| p.join(part))
}

/// True for a relative path made of `[a-z0-9._-]` segments separated by `/`, without empty,
/// `.` or `..` segments.
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 128
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        })
}

/// Lists files (as `/`-separated relative paths) and non-file entries (with a trailing `/` for
/// directories, `@` for anything else) under `dir`, two levels deep at most.
fn list_entries(dir: &Path) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    let mut pending = vec![(dir.to_path_buf(), String::new(), 0usize)];
    while let Some((path, prefix, depth)) = pending.pop() {
        let entries = std::fs::read_dir(&path).map_err(|e| io_error(&path, &e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_error(&path, &e))?;
            if out.len().saturating_add(pending.len()) >= MAX_DIR_ENTRIES {
                return Err(Error::Limit(format!(
                    "more than {MAX_DIR_ENTRIES} entries under {}",
                    dir.display()
                )));
            }
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            let kind = entry.file_type().map_err(|e| io_error(&entry.path(), &e))?;
            if kind.is_file() {
                out.push(name);
            } else if kind.is_dir() && depth == 0 {
                out.push(format!("{name}/"));
                pending.push((entry.path(), format!("{name}/"), depth.saturating_add(1)));
            } else if kind.is_dir() {
                out.push(format!("{name}/"));
            } else {
                out.push(format!("{name}@"));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn owned_entries(set: &FixtureSet) -> BTreeSet<String> {
    let mut owned: BTreeSet<String> = set.files().iter().map(|(p, _)| (*p).to_owned()).collect();
    for (path, _) in set.files() {
        if let Some((parent, _)) = path.rsplit_once('/') {
            owned.insert(format!("{parent}/"));
        }
    }
    owned
}

/// Writes `set` into `dir`, creating it if needed and replacing files the generator owns.
///
/// Refuses, before writing anything, if `dir` holds any entry that is not part of the set.
pub fn write(set: &FixtureSet, dir: &Path) -> Result<(), Error> {
    if dir.exists() {
        if !dir.is_dir() {
            return Err(Error::Io {
                path: dir.display().to_string(),
                message: "not a directory".to_owned(),
            });
        }
        let owned = owned_entries(set);
        let foreign: Vec<String> = list_entries(dir)?
            .into_iter()
            .filter(|e| !owned.contains(e))
            .collect();
        if !foreign.is_empty() {
            return Err(Error::ForeignEntries(foreign));
        }
    }
    for (relative, bytes) in set.files() {
        let path = join(dir, relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_error(parent, &e))?;
        }
        std::fs::write(&path, bytes).map_err(|e| io_error(&path, &e))?;
    }
    Ok(())
}

/// Why [`read_bounded`] failed.
#[derive(Debug)]
pub enum ReadError {
    /// The file is larger than the limit.
    TooLarge {
        /// The limit, in bytes.
        limit: usize,
    },
    /// The file could not be read.
    Io(std::io::Error),
}

/// Reads at most `limit` bytes from `path`; a longer file is an error rather than truncated.
pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, ReadError> {
    let file = std::fs::File::open(path).map_err(ReadError::Io)?;
    let mut bytes = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    file.take(cap)
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() > limit {
        return Err(ReadError::TooLarge { limit });
    }
    Ok(bytes)
}

/// Result of [`verify`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerifyReport {
    /// Files compared byte for byte with the generator output.
    pub files_compared: usize,
    /// Manifest digests recomputed from the bytes on disk.
    pub digests_checked: usize,
    /// Every mismatch found; empty when the directory is current.
    pub problems: Vec<String>,
}

impl VerifyReport {
    fn problem(&mut self, text: String) {
        if self.problems.len() < MAX_PROBLEMS {
            self.problems.push(text);
        }
    }
}

/// Verifies that `dir` holds exactly the generated set, byte for byte, and that every digest
/// and size in the manifest on disk matches the file bytes on disk.
pub fn verify(dir: &Path) -> Result<VerifyReport, Error> {
    if !dir.is_dir() {
        return Err(Error::Io {
            path: dir.display().to_string(),
            message: "not a directory".to_owned(),
        });
    }
    let set = generate()?;
    let on_disk: BTreeSet<String> = list_entries(dir)?.into_iter().collect();
    let owned = owned_entries(&set);
    let mut report = VerifyReport::default();

    for (relative, expected) in set.files() {
        if !on_disk.contains(relative) {
            report.problem(format!("missing {relative}"));
            continue;
        }
        match read_bounded(&join(dir, relative), MAX_FILE_BYTES) {
            Ok(actual) if actual == expected => report.files_compared += 1,
            Ok(_) => {
                report.files_compared += 1;
                report.problem(format!(
                    "{relative}: content differs from the generator output"
                ));
            }
            Err(ReadError::TooLarge { limit }) => {
                report.problem(format!("{relative}: larger than {limit} bytes"));
            }
            Err(ReadError::Io(e)) => report.problem(format!("{relative}: {e}")),
        }
    }
    for entry in on_disk.difference(&owned) {
        report.problem(format!("unexpected entry {entry}"));
    }
    if on_disk.contains(MANIFEST_PATH) {
        check_manifest_digests(dir, &mut report);
    }
    Ok(report)
}

fn check_manifest_digests(dir: &Path, report: &mut VerifyReport) {
    let manifest = match read_bounded(&join(dir, MANIFEST_PATH), MAX_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(ReadError::TooLarge { limit }) => {
            return report.problem(format!("{MANIFEST_PATH}: larger than {limit} bytes"));
        }
        Err(ReadError::Io(e)) => return report.problem(format!("{MANIFEST_PATH}: {e}")),
    };
    let parsed = match json::parse(&manifest, &MANIFEST_LIMITS) {
        Ok(v) => v,
        Err(e) => return report.problem(format!("{MANIFEST_PATH}: {e}")),
    };
    let Some(entries) = parsed.get("fixtures").and_then(Json::as_array) else {
        return report.problem(format!("{MANIFEST_PATH}: no fixtures array"));
    };
    for (index, entry) in entries.iter().enumerate() {
        let path = entry.get("path").and_then(Json::as_str);
        let digest = entry.get("sha256").and_then(Json::as_str);
        let size = entry.get("bytes").and_then(Json::as_f64);
        let (Some(path), Some(digest), Some(size)) = (path, digest, size) else {
            report.problem(format!(
                "{MANIFEST_PATH}: fixtures[{index}] lacks path/sha256/bytes"
            ));
            continue;
        };
        if !is_safe_relative_path(path) {
            report.problem(format!(
                "{MANIFEST_PATH}: fixtures[{index}] has unsafe path {path:?}"
            ));
            continue;
        }
        let bytes = match read_bounded(&join(dir, path), MAX_FILE_BYTES) {
            Ok(b) => b,
            Err(ReadError::TooLarge { limit }) => {
                report.problem(format!("{path}: larger than {limit} bytes"));
                continue;
            }
            Err(ReadError::Io(e)) => {
                report.problem(format!("{path}: {e}"));
                continue;
            }
        };
        report.digests_checked += 1;
        let actual = sha256::hex_digest(&bytes);
        if actual != digest {
            report.problem(format!(
                "{path}: sha256 {actual} does not match manifest {digest}"
            ));
        }
        // Sizes are far below 2^53, so the comparison is exact.
        if bytes.len() as f64 != size {
            report.problem(format!(
                "{path}: {} bytes but the manifest records {size}",
                bytes.len()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(generate().unwrap(), generate().unwrap());
    }

    #[test]
    fn manifest_lists_every_case_with_a_full_digest() {
        let set = generate().unwrap();
        let manifest = json::parse(&set.manifest, &MANIFEST_LIMITS).unwrap();
        let entries = manifest.get("fixtures").unwrap().as_array().unwrap();
        assert_eq!(entries.len(), set.cases.len());
        for (entry, case) in entries.iter().zip(&set.cases) {
            assert_eq!(entry.get("id").unwrap().as_str(), Some(case.id));
            let digest = entry.get("sha256").unwrap().as_str().unwrap();
            assert_eq!(digest.len(), 64);
            assert!(
                digest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
            assert_eq!(digest, sha256::hex_digest(&case.bytes));
            assert_eq!(entry.get("licence").unwrap().as_str(), Some(LICENCE));
        }
    }

    #[test]
    fn rejects_unsafe_relative_paths() {
        for bad in ["", "/a", "a//b", "../a", "a/./b", "A.json", "a\\b", "a/"] {
            assert!(!is_safe_relative_path(bad), "{bad:?}");
        }
        assert!(is_safe_relative_path("valid/plate-1.json"));
    }
}
