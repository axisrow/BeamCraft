// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Store-maintenance primitives of [`BlobStore`] for U14 history compaction: sized blob listing,
//! blob removal, atomically replaced small files and the store lock file. Every path is built from
//! a validated digest or an internal literal, and every entry gets the link and containment checks
//! of the readers; nothing here follows a link or removes anything but a regular file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use beamcraft_model::digest::ContentDigest;

use super::{BlobStore, MAX_SCANNED_ENTRIES, NEXT_TEMP, StorageError, io_error, is_link};

/// Internal lock names; no caller-supplied path can become a lock file.
pub(crate) enum LockFile {
    Maintenance,
    WorkspaceState,
    Publication,
}

/// One installed blob as found on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlobInfo {
    /// The digest the file name states (the bytes are not verified by a listing).
    pub(crate) digest: ContentDigest,
    /// File size in bytes.
    pub(crate) size: u64,
    /// Seconds since the file was last modified, zero when unknown.
    pub(crate) age_secs: u64,
}

/// One regular file of a namespace directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileInfo {
    /// File name.
    pub(crate) name: String,
    /// File size in bytes.
    pub(crate) size: u64,
}

impl BlobStore {
    /// Lists installed blob files with their size and age, ascending by digest, without reading
    /// their bytes. More than `limit` blobs is [`StorageError::LimitExceeded`]. An absent `blobs`
    /// directory lists nothing; unrelated names are ignored.
    pub(crate) fn list_blobs(&self, limit: usize) -> Result<Vec<BlobInfo>, StorageError> {
        self.check_root()?;
        let blobs = self.root.join("blobs");
        match self.check_directory(&blobs, false) {
            Ok(()) => {}
            Err(StorageError::NotFound) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        }
        let now = SystemTime::now();
        let mut found = Vec::new();
        let mut scanned = 0usize;
        for shard in fs::read_dir(&blobs).map_err(io_error)? {
            let shard = shard.map_err(io_error)?;
            let Some(prefix) = shard.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let metadata = fs::symlink_metadata(shard.path()).map_err(io_error)?;
            if prefix.len() != 2 || is_link(&metadata) || !metadata.is_dir() {
                continue;
            }
            for blob in fs::read_dir(shard.path()).map_err(io_error)? {
                let blob = blob.map_err(io_error)?;
                scanned = scanned
                    .checked_add(1)
                    .filter(|n| *n <= MAX_SCANNED_ENTRIES)
                    .ok_or(StorageError::LimitExceeded)?;
                let Some(name) = blob.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if !name.starts_with(&prefix) {
                    continue;
                }
                let Ok(digest) = name.parse::<ContentDigest>() else {
                    continue;
                };
                let metadata = fs::symlink_metadata(blob.path()).map_err(io_error)?;
                if is_link(&metadata) || !metadata.is_file() {
                    return Err(StorageError::UnsafeEntry);
                }
                if found.len() >= limit {
                    return Err(StorageError::LimitExceeded);
                }
                let age_secs = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .map_or(0, |age| age.as_secs());
                found.push(BlobInfo {
                    digest,
                    size: metadata.len(),
                    age_secs,
                });
            }
        }
        found.sort_by_key(|info| info.digest);
        Ok(found)
    }

    /// Removes one installed blob file. `Ok(false)` when it is already absent; an entry that is a
    /// link or not a regular file is [`StorageError::UnsafeEntry`] and is left alone.
    pub(crate) fn remove_blob(&self, digest: &ContentDigest) -> Result<bool, StorageError> {
        let path = match self.location(digest, false) {
            Ok(path) => path,
            Err(StorageError::NotFound) => return Ok(false),
            Err(error) => return Err(error),
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(io_error(e)),
        };
        if is_link(&metadata) || !metadata.is_file() {
            return Err(StorageError::UnsafeEntry);
        }
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io_error(e)),
        }
    }

    /// Lists the regular files of a checked namespace directory, ascending by name. Directories
    /// and other entries are ignored; a link is [`StorageError::UnsafeEntry`].
    pub(crate) fn list_namespace_files(
        &self,
        directory: &Path,
        limit: usize,
    ) -> Result<Vec<FileInfo>, StorageError> {
        self.check_directory(directory, false)?;
        let mut found = Vec::new();
        for entry in fs::read_dir(directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
            if is_link(&metadata) {
                return Err(StorageError::UnsafeEntry);
            }
            if !metadata.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                return Err(StorageError::UnsafeEntry);
            };
            if found.len() >= limit {
                return Err(StorageError::LimitExceeded);
            }
            found.push(FileInfo {
                name,
                size: metadata.len(),
            });
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }

    /// Writes `bytes` to a fresh uniquely named `<prefix>...` file of a checked namespace
    /// directory, flushed and synced, and returns its name. Nothing is replaced.
    pub(crate) fn stage_namespace_file(
        &self,
        directory: &Path,
        prefix: &str,
        bytes: &[u8],
    ) -> Result<String, StorageError> {
        self.check_directory(directory, false)?;
        for _ in 0..1024 {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let name = format!("{prefix}{:08x}-{id:016x}", std::process::id());
            let path = directory.join(&name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let written = file
                        .write_all(bytes)
                        .and_then(|()| file.flush())
                        .and_then(|()| file.sync_all());
                    if let Err(e) = written {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(io_error(e));
                    }
                    return Ok(name);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_error(e)),
            }
        }
        Err(StorageError::Io(io::ErrorKind::AlreadyExists))
    }

    /// Renames a staged file over `name` in one step: a reader sees the old file or the new one.
    /// The destination, when it exists, must be a regular file.
    pub(crate) fn install_over(
        &self,
        directory: &Path,
        staged: &str,
        name: &str,
    ) -> Result<(), StorageError> {
        self.check_directory(directory, false)?;
        let destination = directory.join(name);
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if is_link(&metadata) || !metadata.is_file() => {
                return Err(StorageError::UnsafeEntry);
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
        fs::rename(directory.join(staged), destination).map_err(io_error)
    }

    /// Removes one regular file of a checked namespace directory. `Ok(false)` when absent.
    pub(crate) fn remove_namespace_file(
        &self,
        directory: &Path,
        name: &str,
    ) -> Result<bool, StorageError> {
        self.check_directory(directory, false)?;
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if is_link(&metadata) || !metadata.is_file() => {
                Err(StorageError::UnsafeEntry)
            }
            Ok(_) => match fs::remove_file(&path) {
                Ok(()) => Ok(true),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(io_error(e)),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io_error(e)),
        }
    }

    /// Opens the store lock file `maintenance.lock` in the store root, creating it when `create`
    /// is set; `Ok(None)` when it is absent and not to be created. The file holds no data; its
    /// advisory lock is the state (see [`crate::lease`]). A link or a non-regular entry under
    /// that name is [`StorageError::UnsafeEntry`].
    pub(crate) fn open_lock_file(&self, create: bool) -> Result<Option<File>, StorageError> {
        self.open_named_lock_file(LockFile::Maintenance, create)
    }

    pub(crate) fn open_named_lock_file(
        &self,
        kind: LockFile,
        create: bool,
    ) -> Result<Option<File>, StorageError> {
        self.check_root()?;
        let name = match kind {
            LockFile::Maintenance => "maintenance.lock",
            LockFile::WorkspaceState => "workspace-state.lock",
            LockFile::Publication => "publication.lock",
        };
        let path: PathBuf = self.root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if is_link(&metadata) || !metadata.is_file() => {
                return Err(StorageError::UnsafeEntry);
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
            }
            Err(e) => return Err(io_error(e)),
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(create)
            .truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
            Err(e) => return Err(io_error(e)),
        };
        let opened = file.metadata().map_err(io_error)?;
        if is_link(&opened) || !opened.is_file() {
            return Err(StorageError::UnsafeEntry);
        }
        Ok(Some(file))
    }
}
