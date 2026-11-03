// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The append-only head log: one small complete record per revision, installed by a
//! non-replacing hard link. See the parent module for the guarantees.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use beamcraft_model::digest::{ContentDigest, Sha256};

use super::manifest::{BaseRef, Content, RevisionDraft};
use super::{MAX_REVISIONS, RevisionError, Uuid7, manifest::Manifest};

/// Directory below the store root that holds the head log.
pub(super) const DIR: &str = "revisions";
/// Longest record file the reader accepts.
pub(super) const MAX_RECORD_BYTES: usize = 512;
pub(super) const TEMP_PREFIX: &str = ".tmp-";
pub(super) const MAGIC: &str = "beamcraft-revision-root 1";
/// Directory entries inspected by one scan: the log plus a bounded allowance for leftovers.
const MAX_SCANNED: u64 = MAX_REVISIONS + 4096;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// One head-log record: which revision is the root at one sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRecord {
    sequence: u64,
    revision: Uuid7,
    manifest: ContentDigest,
}

impl RootRecord {
    pub(super) fn new(sequence: u64, revision: Uuid7, manifest: ContentDigest) -> Self {
        Self {
            sequence,
            revision,
            manifest,
        }
    }

    /// Revision number, starting at 1.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Revision id.
    pub fn revision(&self) -> &Uuid7 {
        &self.revision
    }

    /// Digest of the manifest blob (the U01 `Identity::revision` for this revision).
    pub const fn manifest(&self) -> ContentDigest {
        self.manifest
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        format!(
            "{MAGIC}\nsequence {}\nrevision {}\nmanifest {}\n",
            self.sequence, self.revision, self.manifest
        )
        .into_bytes()
    }

    pub(super) fn parse(bytes: &[u8]) -> Result<Self, RevisionError> {
        let bad = RevisionError::TornRoot;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(bad);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| bad)?;
        let body = text.strip_suffix('\n').ok_or(bad)?;
        let mut lines = body.split('\n');
        if lines.next() != Some(MAGIC) {
            return Err(bad);
        }
        let mut field = |key: &str| {
            lines
                .next()
                .and_then(|l| l.strip_prefix(key))
                .and_then(|r| r.strip_prefix(' '))
                .ok_or(bad)
        };
        let sequence = canonical_decimal(field("sequence")?).ok_or(bad)?;
        let revision = Uuid7::parse(field("revision")?).map_err(|_| bad)?;
        let manifest = field("manifest")?
            .parse::<ContentDigest>()
            .map_err(|_| bad)?;
        let record = Self {
            sequence,
            revision,
            manifest,
        };
        // The one canonical byte form: nothing may follow, and nothing may differ.
        if record.encode() != bytes {
            return Err(bad);
        }
        Ok(record)
    }
}

fn canonical_decimal(text: &str) -> Option<u64> {
    let ok = !text.is_empty()
        && text.len() <= 20
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if ok { text.parse().ok() } else { None }
}

/// A verified head: the record together with its parsed manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionInfo {
    pub(super) record: RootRecord,
    pub(super) manifest: Manifest,
}

impl RevisionInfo {
    /// The head-log record.
    pub fn record(&self) -> &RootRecord {
        &self.record
    }

    /// The parsed manifest, whose digest the record names.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Revision number.
    pub const fn sequence(&self) -> u64 {
        self.record.sequence
    }

    /// Revision id.
    pub fn revision(&self) -> &Uuid7 {
        &self.record.revision
    }

    /// Manifest digest: the identity a fresh U01 reader pins as `Identity::revision`.
    pub const fn manifest_digest(&self) -> ContentDigest {
        self.record.manifest
    }

    /// Builds the info of the revision a draft's manifest would become, without committing
    /// anything: the manifest is assembled from the draft with the given head-log sequence and
    /// no base or index evidence, and its digest is computed, never stored. The CLI session
    /// reports drafted batch commands from this; the revision the batch commit later
    /// publishes carries the batch's own operation, revision id, sequence and base, so only
    /// the layer set of a preview is the promised one. Nothing reaches the root, the blobs or
    /// the ledger.
    pub fn preview(
        draft: &RevisionDraft,
        sequence: u64,
        base: Option<BaseRef>,
    ) -> Result<Self, RevisionError> {
        let content = Content::from_draft(draft)?;
        let manifest = Manifest::assemble(content, sequence, base, None)?;
        // A short canonical manifest is far below the SHA-256 length domain; the store would
        // refuse it long before the digest could fail.
        let digest =
            Sha256::digest(&manifest.encode()).map_err(|_| RevisionError::LimitExceeded)?;
        Ok(Self {
            record: RootRecord::new(sequence, draft.revision.clone(), digest),
            manifest,
        })
    }
}

/// Record file name for a sequence: 20 decimal digits, so names sort like numbers.
pub(super) fn record_name(sequence: u64) -> String {
    format!("{sequence:020}")
}

fn parse_record_name(name: &str) -> Option<u64> {
    if name.len() != 20 || !name.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    name.parse()
        .ok()
        .filter(|s| (1..=MAX_REVISIONS).contains(s))
}

/// The names of a head-log directory: record sequences (ascending), whether the U14 floor marker
/// is present, and the `.tmp-*` leftovers that readers ignore.
pub(super) struct Listing {
    pub(super) sequences: Vec<u64>,
    pub(super) marker: bool,
}

/// Lists the head-log directory after checking that it holds only record names, the floor
/// marker and `.tmp-*` leftovers. Does not open the records.
pub(super) fn list(directory: &Path) -> Result<Listing, RevisionError> {
    let io_err = |e: io::Error| RevisionError::Io(e.kind());
    let mut inspected = 0u64;
    let mut listing = Listing {
        sequences: Vec::new(),
        marker: false,
    };
    for entry in fs::read_dir(directory).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        inspected = inspected
            .checked_add(1)
            .filter(|n| *n <= MAX_SCANNED)
            .ok_or(RevisionError::LimitExceeded)?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(RevisionError::TornRoot)?;
        if name.starts_with(TEMP_PREFIX) {
            continue;
        }
        if name == super::floor::FILE {
            listing.marker = true;
            continue;
        }
        listing
            .sequences
            .push(parse_record_name(name).ok_or(RevisionError::TornRoot)?);
    }
    listing.sequences.sort_unstable();
    Ok(listing)
}

/// A fully written, synced temporary record that is removed unless installed and dropped.
pub(super) struct TempRecord {
    path: PathBuf,
}

impl TempRecord {
    /// Writes and syncs the record bytes into a fresh uniquely named file of the directory.
    pub(super) fn create(directory: &Path, bytes: &[u8]) -> Result<Self, io::Error> {
        for _ in 0..1024 {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!("{TEMP_PREFIX}{:08x}-{id:016x}", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let temp = Self { path };
                    file.write_all(bytes)?;
                    file.flush()?;
                    file.sync_all()?;
                    return Ok(temp);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::from(io::ErrorKind::AlreadyExists))
    }

    /// Installs the complete record under its final name without replacing anything: the link
    /// fails with `AlreadyExists` when another committer already took this sequence.
    pub(super) fn install(&self, directory: &Path, sequence: u64) -> io::Result<()> {
        fs::hard_link(&self.path, directory.join(record_name(sequence)))
    }
}

impl Drop for TempRecord {
    fn drop(&mut self) {
        // Best effort: a leftover temporary file is ignored by readers and is U07's to clean.
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use beamcraft_model::digest::Sha256;

    fn record() -> RootRecord {
        RootRecord::new(
            3,
            Uuid7::parse("0199e7a0-0000-7000-8000-000000000003").unwrap(),
            Sha256::digest(b"manifest").unwrap(),
        )
    }

    #[test]
    fn record_round_trips_and_names_sort_numerically() {
        let r = record();
        assert_eq!(RootRecord::parse(&r.encode()).unwrap(), r);
        assert_eq!(record_name(3), "00000000000000000003");
        assert_eq!(
            parse_record_name(&record_name(MAX_REVISIONS)),
            Some(MAX_REVISIONS)
        );
        for bad in [
            "0",
            "00000000000000000000",
            "00000000000000000003x",
            "0000000000000000003",
            "00000000000000000003.bak",
            "00000000000001000001",
            "-0000000000000000003",
        ] {
            assert_eq!(parse_record_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn record_parser_rejects_garbage_and_truncation() {
        let good = record().encode();
        let text = String::from_utf8(good.clone()).unwrap();
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            good[..good.len() - 1].to_vec(),
            good[..good.len() / 2].to_vec(),
            [good.clone(), b"x\n".to_vec()].concat(),
            text.replace("sequence 3", "sequence 03").into_bytes(),
            text.replace("root 1", "root 2").into_bytes(),
            text.replace('\n', "\r\n").into_bytes(),
            text.replace("manifest ", "manifest A").into_bytes(),
            vec![0xff; 20],
            vec![b'a'; MAX_RECORD_BYTES + 1],
        ];
        for case in cases {
            assert_eq!(RootRecord::parse(&case), Err(RevisionError::TornRoot));
        }
    }
}
