// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The store handle and the verified reader of the published root.

use std::path::{Path, PathBuf};

use crate::storage::{BlobStore, StorageError};

use super::manifest::{LayerEntry, Manifest};
use super::root::{self, MAX_RECORD_BYTES, RevisionInfo, RootRecord};
use super::{MAX_MANIFEST_BYTES, RevisionError, Uuid7};

/// A revision store bound to one [`BlobStore`] root. Resource methods take no path or URI.
#[derive(Debug, Clone)]
pub struct RevisionStore {
    pub(crate) blobs: BlobStore,
}

/// A revision whose record, manifest, base link and every layer blob were verified by
/// [`RevisionStore::open`]. Later reads verify the bytes again.
#[derive(Debug, Clone)]
pub struct Revision {
    store: RevisionStore,
    info: RevisionInfo,
}

impl RevisionStore {
    /// Binds the revision log to the same protected root as the blob store.
    pub fn new(blobs: BlobStore) -> Self {
        Self { blobs }
    }

    /// The underlying immutable byte store.
    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// The current root without verifying layer blobs: the record, its manifest and the base
    /// link are checked. `Ok(None)` means nothing was ever published.
    pub fn head(&self) -> Result<Option<RevisionInfo>, RevisionError> {
        match self.head_dir()? {
            Some(directory) => self.load_head_in(&directory),
            None => Ok(None),
        }
    }

    /// Opens the current root and verifies every referenced layer blob (presence, full digest,
    /// declared size). A torn log, a missing or corrupt manifest, a broken base link, a
    /// missing (for example garbage-collected), corrupt or resized blob is a typed error and
    /// no partial revision is returned. [`RevisionError::NoRevision`] if nothing is published.
    pub fn open(&self) -> Result<Revision, RevisionError> {
        let info = self.head()?.ok_or(RevisionError::NoRevision)?;
        for (index, entry) in info.manifest.content().entries().iter().enumerate() {
            self.read_entry(index, entry)?;
        }
        Ok(Revision {
            store: self.clone(),
            info,
        })
    }

    /// The record and manifest of one earlier (or the current) revision by its 1-based log
    /// sequence, with the same record, manifest and base-link checks as [`Self::head`]. A
    /// sequence of zero or beyond the head is [`RevisionError::NoRevision`].
    pub fn info_at(&self, sequence: u64) -> Result<RevisionInfo, RevisionError> {
        let directory = self.head_dir()?.ok_or(RevisionError::NoRevision)?;
        match root::scan(&directory)? {
            Some(head) if sequence > 0 && sequence <= head => self.load_at(&directory, sequence),
            _ => Err(RevisionError::NoRevision),
        }
    }

    /// The log sequence of the revision with this id, found by reading each head-log record once
    /// from the head down (the directory is scanned a single time). `Ok(None)` when no committed
    /// revision has that id.
    pub fn sequence_of(&self, revision: &Uuid7) -> Result<Option<u64>, RevisionError> {
        let Some(directory) = self.head_dir()? else {
            return Ok(None);
        };
        let Some(head) = root::scan(&directory)? else {
            return Ok(None);
        };
        for sequence in (1..=head).rev() {
            if self.load_record(&directory, sequence)?.revision() == revision {
                return Ok(Some(sequence));
            }
        }
        Ok(None)
    }

    /// Opens one revision by its log sequence and verifies every layer blob it references, as
    /// [`Self::open`] does for the head. The blobs of an earlier revision stay readable as long as
    /// the store does not garbage-collect them.
    pub fn open_at(&self, sequence: u64) -> Result<Revision, RevisionError> {
        let info = self.info_at(sequence)?;
        for (index, entry) in info.manifest.content().entries().iter().enumerate() {
            self.read_entry(index, entry)?;
        }
        Ok(Revision {
            store: self.clone(),
            info,
        })
    }

    pub(crate) fn head_dir(&self) -> Result<Option<PathBuf>, RevisionError> {
        match self.blobs.namespace_dir(root::DIR, false) {
            Ok(path) => Ok(Some(path)),
            Err(StorageError::NotFound) => Ok(None),
            Err(error) => Err(RevisionError::Storage(error)),
        }
    }

    pub(crate) fn load_head_in(
        &self,
        directory: &Path,
    ) -> Result<Option<RevisionInfo>, RevisionError> {
        match root::scan(directory)? {
            Some(sequence) => self.load_at(directory, sequence).map(Some),
            None => Ok(None),
        }
    }

    pub(super) fn load_record(
        &self,
        directory: &Path,
        sequence: u64,
    ) -> Result<RootRecord, RevisionError> {
        let bytes = self
            .blobs
            .read_namespace_file(directory, &root::record_name(sequence), MAX_RECORD_BYTES)
            .map_err(|error| match error {
                StorageError::NotFound
                | StorageError::LimitExceeded
                | StorageError::UnsafeEntry => RevisionError::TornRoot,
                other => RevisionError::Storage(other),
            })?;
        let record = RootRecord::parse(&bytes)?;
        if record.sequence() != sequence {
            return Err(RevisionError::TornRoot);
        }
        Ok(record)
    }

    /// Loads one log record with its manifest and checks that the record, the manifest and the
    /// previous record agree.
    pub(crate) fn load_at(
        &self,
        directory: &Path,
        sequence: u64,
    ) -> Result<RevisionInfo, RevisionError> {
        let record = self.load_record(directory, sequence)?;
        let bytes = self
            .blobs
            .read_capped(&record.manifest(), MAX_MANIFEST_BYTES)
            .map_err(|error| match error {
                StorageError::NotFound => RevisionError::ManifestMissing,
                StorageError::Io(_) => RevisionError::Storage(error),
                _ => RevisionError::MalformedManifest,
            })?;
        let manifest = Manifest::parse(&bytes)?;
        if manifest.sequence() != sequence || manifest.content().revision() != record.revision() {
            return Err(RevisionError::RootMismatch);
        }
        match (sequence.checked_sub(1).filter(|s| *s > 0), manifest.base()) {
            (None, None) => {}
            (Some(previous), Some(base)) => {
                let before = self.load_record(directory, previous)?;
                if before.revision() != &base.revision || before.manifest() != base.manifest {
                    return Err(RevisionError::RootMismatch);
                }
            }
            _ => return Err(RevisionError::RootMismatch),
        }
        Ok(RevisionInfo { record, manifest })
    }

    /// Reads one referenced layer blob and checks its digest (by the blob store) and size.
    pub(super) fn read_entry(
        &self,
        index: usize,
        entry: &LayerEntry,
    ) -> Result<Vec<u8>, RevisionError> {
        let bytes = self
            .blobs
            .read(&entry.digest())
            .map_err(|error| match error {
                StorageError::NotFound => RevisionError::MissingBlob { entry: index },
                StorageError::DigestMismatch
                | StorageError::UnsafeEntry
                | StorageError::IntegrityConflict => RevisionError::CorruptBlob { entry: index },
                other => RevisionError::Storage(other),
            })?;
        if u64::try_from(bytes.len()) != Ok(entry.size()) {
            return Err(RevisionError::SizeMismatch { entry: index });
        }
        Ok(bytes)
    }
}

impl Revision {
    /// The verified head-log record and manifest.
    pub fn info(&self) -> &RevisionInfo {
        &self.info
    }

    /// The manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.info.manifest
    }

    /// Verified bytes of one layer by its virtual name. An unknown name is a storage miss
    /// (`NOT_FOUND`); the bytes are verified again against the manifest on every call.
    pub fn layer_bytes(&self, name: &str) -> Result<Vec<u8>, RevisionError> {
        let (index, entry) = self
            .info
            .manifest
            .content()
            .entries()
            .iter()
            .enumerate()
            .find(|(_, e)| e.name() == name)
            .ok_or(RevisionError::Storage(StorageError::NotFound))?;
        self.store.read_entry(index, entry)
    }
}
