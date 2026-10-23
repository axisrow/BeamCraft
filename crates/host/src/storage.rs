// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Immutable full-SHA-256 byte resources (U05; F04 sections 2 and 8).
//!
//! The authorized root must already exist. Only the host may mutate its namespace while
//! operations run: safe portable `std` has no directory-handle-relative, no-follow traversal.
//! Every existing component is checked for links/reparse points (on macOS, any ancestor
//! link is canonicalised and asserted as a resolved directory — sanctioned system links
//! such as /var -> /private/var make this necessary, and a user-created ancestor link
//! passes too), and canonical containment is checked before opening. These checks reject
//! static hostile entries, but cannot exclude a hostile concurrent directory replacement
//! between a check and a filesystem call. Restrict filesystem permissions accordingly; this
//! API is not a sandbox against a hostile local writer.
//!
//! Installation uses a same-filesystem hard link, which fails if a destination already exists.
//! Filesystems without hard-link support return an error; there is no replacing fallback.
//! Temp bytes are flushed and synced before installation. No directory-sync durability,
//! crash-atomic multi-file commit or collision impossibility is claimed; garbage collection is
//! U14's compaction (`crate::compaction`), which uses the maintenance primitives of the private
//! `gc` submodule.
//! A crash can leave a temporary file; U07 recovery (`crate::ledger`) removes abandoned ones.

use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use beamcraft_model::ErrorCode;
use beamcraft_model::digest::{ContentDigest, Sha256};

mod gc;
pub(crate) use gc::LockFile;

pub(crate) use gc::BlobInfo;

/// Maximum blob and returned byte-buffer length: 32 MiB (33,554,432 bytes). The layer engine's
/// model profile sizes one layer by this cap.
pub const MAX_BLOB_BYTES: usize = 33_554_432;

/// Directory entries one namespace scan inspects before it gives up with `LimitExceeded`.
const MAX_SCANNED_ENTRIES: usize = 1_100_000;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// True when the last byte of a non-empty regular file is a newline.
fn ends_with_newline(path: &Path) -> Result<bool, StorageError> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(path).map_err(io_error)?;
    file.seek(SeekFrom::End(-1)).map_err(io_error)?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last).map_err(io_error)?;
    Ok(last == *b"\n")
}

/// Successful publish-if-absent outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Published {
    /// A complete new file was installed.
    Created,
    /// All existing bytes equal the requested bytes; the blob was not rewritten.
    AlreadyPresent,
}

/// Stable storage failure names with safe messages (no absolute paths or raw OS errors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    /// Root is not an existing absolute directory, or the requested cap is invalid.
    InvalidArgument,
    /// A link, reparse point, non-regular entry or containment violation was found.
    UnsafeEntry,
    /// Blob is absent.
    NotFound,
    /// Stored bytes do not hash to their filename.
    DigestMismatch,
    /// Existing bytes differ from the full proposed bytes; nothing was overwritten.
    IntegrityConflict,
    /// Blob/read cap or checked arithmetic was exceeded.
    LimitExceeded,
    /// Native I/O failed; only its safe category is retained.
    Io(io::ErrorKind),
}

impl StorageError {
    /// Maps stable storage failures to the existing F04 section 8 vocabulary.
    pub const fn code(self) -> ErrorCode {
        match self {
            Self::InvalidArgument => ErrorCode::InvalidArgument,
            Self::UnsafeEntry | Self::DigestMismatch | Self::IntegrityConflict => {
                ErrorCode::IntegrityError
            }
            Self::NotFound => ErrorCode::NotFound,
            Self::LimitExceeded => ErrorCode::LimitExceeded,
            Self::Io(io::ErrorKind::PermissionDenied) => ErrorCode::PermissionDenied,
            Self::Io(_) => ErrorCode::BackendUnavailable,
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidArgument => "invalid storage root or size cap",
            Self::UnsafeEntry => "unsafe storage entry or containment violation",
            Self::NotFound => "blob not found",
            Self::DigestMismatch => "blob bytes do not match the content digest",
            Self::IntegrityConflict => "existing blob differs from proposed bytes",
            Self::LimitExceeded => "blob or memory size cap exceeded",
            Self::Io(_) => "storage I/O failed",
        })
    }
}

impl std::error::Error for StorageError {}

pub(crate) fn io_error(error: io::Error) -> StorageError {
    if error.kind() == io::ErrorKind::NotFound {
        StorageError::NotFound
    } else {
        StorageError::Io(error.kind())
    }
}

pub(crate) fn is_link(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT, including junctions and non-symlink reparse types.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn directory(path: &Path) -> Result<(), StorageError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if is_link(&metadata) || !metadata.is_dir() {
        return Err(StorageError::UnsafeEntry);
    }
    Ok(())
}

fn absolute_directory(path: &Path) -> Result<(), StorageError> {
    // Walk from the filesystem root down so an ancestor link is checked before a child
    // lookup can follow it. The root argument is host configuration, never an asset path.
    // macOS places ordinary roots (temp dir, home) behind system links such as
    // /var -> /private/var, so any ancestor link there is canonicalised first and asserted
    // as a resolved directory (a user-created ancestor link passes too; a dangling or
    // non-directory ancestor still fails); elsewhere the plain component is asserted
    // unchanged. The supplied root itself must always be a plain directory.
    let resolve_ancestors = cfg!(target_os = "macos");
    let mut prefix = PathBuf::new();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        prefix.push(component);
        if !prefix.is_absolute() {
            continue;
        }
        let checked = if components.peek().is_none() || !resolve_ancestors {
            prefix.clone()
        } else {
            fs::canonicalize(&prefix).map_err(io_error)?
        };
        directory(&checked)?;
    }
    Ok(())
}

/// A store bound to one authorized root; resource methods accept only [`ContentDigest`].
/// Clones and independent instances may publish concurrently under the namespace ownership
/// requirement in the module documentation.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
    canonical_root: PathBuf,
    cap: usize,
}

impl BlobStore {
    /// Opens an existing absolute directory with the [`MAX_BLOB_BYTES`] cap.
    /// Does not create the root or accept an asset path/URI.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::with_limit(root, MAX_BLOB_BYTES)
    }

    /// The canonical form of the store's root directory, for deciding whether a path lies inside
    /// the store.
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    /// Opens a store with a smaller blob/read memory cap (zero permits only empty blobs).
    /// Caps above [`MAX_BLOB_BYTES`] are invalid; errors never include the root path.
    pub fn with_limit(root: impl AsRef<Path>, cap: usize) -> Result<Self, StorageError> {
        let root = root.as_ref();
        if !root.is_absolute() || cap > MAX_BLOB_BYTES {
            return Err(StorageError::InvalidArgument);
        }
        #[cfg(windows)]
        {
            // Reject UNC/network and device namespaces before any filesystem operation.
            // Only ordinary local drive roots (including canonical verbatim disks) qualify.
            use std::path::{Component, Prefix};
            if !matches!(root.components().next(), Some(Component::Prefix(p))
                if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
            {
                return Err(StorageError::InvalidArgument);
            }
        }
        // Reject links in the supplied root itself, and in its ancestors except on macOS,
        // where any ancestor link is canonicalised instead (system links such as
        // /var -> /private/var sit before ordinary roots; a user-created ancestor link
        // passes too). Parent components are ambiguous.
        if root
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(StorageError::InvalidArgument);
        }
        match absolute_directory(root) {
            Err(StorageError::NotFound) => return Err(StorageError::InvalidArgument),
            result => result?,
        }
        let canonical_root = fs::canonicalize(root).map_err(io_error)?;
        Ok(Self {
            root: root.to_owned(),
            canonical_root,
            cap,
        })
    }

    fn check_root(&self) -> Result<(), StorageError> {
        absolute_directory(&self.root)?;
        if fs::canonicalize(&self.root).map_err(io_error)? != self.canonical_root {
            return Err(StorageError::UnsafeEntry);
        }
        Ok(())
    }

    fn check_directory(&self, path: &Path, create: bool) -> Result<(), StorageError> {
        if create {
            match fs::create_dir(path) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(io_error(e)),
            }
        }
        directory(path)?;
        let canonical = fs::canonicalize(path).map_err(io_error)?;
        if !canonical.starts_with(&self.canonical_root) {
            return Err(StorageError::UnsafeEntry);
        }
        Ok(())
    }

    fn location(&self, digest: &ContentDigest, create: bool) -> Result<PathBuf, StorageError> {
        self.check_root()?;
        let blobs = self.root.join("blobs");
        self.check_directory(&blobs, create)?;
        // Both components are generated from a validated digest, never caller path text.
        let name = digest.to_string();
        let prefix: String = name.chars().take(2).collect();
        let shard = blobs.join(prefix);
        self.check_directory(&shard, create)?;
        Ok(shard.join(name))
    }

    fn read_file(&self, path: &Path, digest: &ContentDigest) -> Result<Vec<u8>, StorageError> {
        let bytes = self.read_bounded(path, self.cap)?;
        let actual = Sha256::digest(&bytes).map_err(|_| StorageError::LimitExceeded)?;
        if actual != *digest {
            return Err(StorageError::DigestMismatch);
        }
        Ok(bytes)
    }

    /// Reads one regular, non-link file below the canonical root without digest verification.
    fn read_bounded(&self, path: &Path, cap: usize) -> Result<Vec<u8>, StorageError> {
        let metadata = fs::symlink_metadata(path).map_err(io_error)?;
        if is_link(&metadata) || !metadata.is_file() {
            return Err(StorageError::UnsafeEntry);
        }
        if !fs::canonicalize(path)
            .map_err(io_error)?
            .starts_with(&self.canonical_root)
        {
            return Err(StorageError::UnsafeEntry);
        }
        let length = usize::try_from(metadata.len()).map_err(|_| StorageError::LimitExceeded)?;
        if length > cap {
            return Err(StorageError::LimitExceeded);
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Open the reparse point itself if the last component changes after preflight.
            options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let mut file = options.open(path).map_err(io_error)?;
        let opened = file.metadata().map_err(io_error)?;
        if is_link(&opened) || !opened.is_file() {
            return Err(StorageError::UnsafeEntry);
        }
        let mut bytes = Vec::new();
        // The metadata length was converted and capped above; reserve it once. A changing
        // file is still bounded on every read below, including growth after this metadata.
        bytes
            .try_reserve_exact(length)
            .map_err(|_| StorageError::Io(io::ErrorKind::OutOfMemory))?;
        let mut buffer = [0u8; 8192];
        loop {
            let count = file.read(&mut buffer).map_err(io_error)?;
            if count == 0 {
                break;
            }
            let next = bytes
                .len()
                .checked_add(count)
                .filter(|n| *n <= cap)
                .ok_or(StorageError::LimitExceeded)?;
            bytes
                .try_reserve_exact(next - bytes.len())
                .map_err(|_| StorageError::Io(io::ErrorKind::OutOfMemory))?;
            let chunk = buffer.get(..count).ok_or(StorageError::LimitExceeded)?;
            bytes.extend_from_slice(chunk);
        }
        Ok(bytes)
    }

    /// Resolves a fixed host-owned namespace directory below the checked root (U06 revision
    /// records). `name` is an internal literal, never caller text. Same link/containment checks
    /// and the same unprotected-concurrent-writer limitation as blob access.
    pub(crate) fn namespace_dir(&self, name: &str, create: bool) -> Result<PathBuf, StorageError> {
        self.check_root()?;
        let path = self.root.join(name);
        self.check_directory(&path, create)?;
        Ok(path)
    }

    /// Reads a small regular file of a checked namespace directory with a tight cap and the
    /// blob reader's entry checks. The bytes are not digest-addressed; the caller validates them.
    pub(crate) fn read_namespace_file(
        &self,
        directory: &Path,
        name: &str,
        cap: usize,
    ) -> Result<Vec<u8>, StorageError> {
        self.check_directory(directory, false)?;
        self.read_bounded(&directory.join(name), self.cap.min(cap))
    }

    /// Reads one regular file of a checked namespace directory under an exact caller cap that
    /// is independent of this store's blob cap (the U07 ledger). `Ok(None)` means the file does
    /// not exist. The bytes are not digest-addressed; the caller validates them.
    pub(crate) fn read_namespace_exact(
        &self,
        directory: &Path,
        name: &str,
        cap: usize,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        self.check_directory(directory, false)?;
        match self.read_bounded(&directory.join(name), cap) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(StorageError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Appends `record` to a regular file of a checked namespace directory in one write, then
    /// flushes and syncs the file. When the file is non-empty and does not end in a newline (a
    /// torn earlier append), a newline is written first so the torn bytes can never merge with
    /// this record. Refuses to grow the file beyond `max_len` with
    /// [`StorageError::LimitExceeded`] before writing anything. The file is created when
    /// absent; its directory entry is not synced.
    pub(crate) fn append_namespace_file(
        &self,
        directory: &Path,
        name: &str,
        record: &[u8],
        max_len: u64,
    ) -> Result<(), StorageError> {
        self.check_directory(directory, false)?;
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if is_link(&metadata) || !metadata.is_file() => {
                return Err(StorageError::UnsafeEntry);
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let mut file = options.open(&path).map_err(io_error)?;
        let opened = file.metadata().map_err(io_error)?;
        if is_link(&opened) || !opened.is_file() {
            return Err(StorageError::UnsafeEntry);
        }
        if !fs::canonicalize(&path)
            .map_err(io_error)?
            .starts_with(&self.canonical_root)
        {
            return Err(StorageError::UnsafeEntry);
        }
        let length = opened.len();
        let mut buffer = Vec::new();
        if length > 0 && !ends_with_newline(&path)? {
            buffer.push(b'\n');
        }
        buffer
            .try_reserve_exact(record.len())
            .map_err(|_| StorageError::Io(io::ErrorKind::OutOfMemory))?;
        buffer.extend_from_slice(record);
        let added = u64::try_from(buffer.len()).map_err(|_| StorageError::LimitExceeded)?;
        if length
            .checked_add(added)
            .is_none_or(|total| total > max_len)
        {
            return Err(StorageError::LimitExceeded);
        }
        file.write_all(&buffer).map_err(io_error)?;
        file.flush().map_err(io_error)?;
        file.sync_all().map_err(io_error)
    }

    /// Removes regular files named `prefix*` directly inside a checked namespace directory
    /// (`blobs` itself is also one) whose last modification is at least `min_age` old; a zero
    /// age removes them all. Links, directories and everything else are left alone. Returns the
    /// number removed. This deletes only abandoned temporary names, never installed content:
    /// a name that is also a hard link of an installed file merely loses the extra name.
    pub(crate) fn remove_stale_temps(
        &self,
        directory: &Path,
        prefix: &str,
        min_age: std::time::Duration,
    ) -> Result<usize, StorageError> {
        self.check_directory(directory, false)?;
        let now = std::time::SystemTime::now();
        let (mut scanned, mut removed) = (0usize, 0usize);
        for entry in fs::read_dir(directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            scanned = scanned
                .checked_add(1)
                .filter(|n| *n <= MAX_SCANNED_ENTRIES)
                .ok_or(StorageError::LimitExceeded)?;
            if !entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(prefix))
            {
                continue;
            }
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if is_link(&metadata) || !metadata.is_file() {
                continue;
            }
            let age = metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok());
            let old_enough = min_age.is_zero() || age.is_some_and(|age| age >= min_age);
            if old_enough {
                match fs::remove_file(&path) {
                    Ok(()) => removed += 1,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_error(e)),
                }
            }
        }
        Ok(removed)
    }

    /// Lists the digests of installed blob files (`blobs/<2 hex>/<64 hex>`) without reading
    /// their bytes; unrelated names are ignored. At most `limit` blobs are listed, more is
    /// [`StorageError::LimitExceeded`]. An absent `blobs` directory lists nothing.
    pub(crate) fn list_digests(&self, limit: usize) -> Result<Vec<ContentDigest>, StorageError> {
        self.check_root()?;
        let blobs = self.root.join("blobs");
        match self.check_directory(&blobs, false) {
            Ok(()) => {}
            Err(StorageError::NotFound) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        }
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
                if found.len() >= limit {
                    return Err(StorageError::LimitExceeded);
                }
                found.push(digest);
            }
        }
        found.sort();
        Ok(found)
    }

    fn compare(
        &self,
        path: &Path,
        digest: &ContentDigest,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        let existing = self.read_file(path, digest).map_err(|error| match error {
            // Proposed bytes already fit the cap: an oversized existing file necessarily
            // differs, so publication reports the conflict without reading beyond the cap.
            StorageError::DigestMismatch | StorageError::LimitExceeded => {
                StorageError::IntegrityConflict
            }
            other => other,
        })?;
        if existing != bytes {
            return Err(StorageError::IntegrityConflict);
        }
        Ok(())
    }

    /// Publishes exact bytes using a synced temporary file and non-replacing hard link.
    /// Existing bytes are compared in full even if their digest matches. A failed cleanup
    /// or post-install verification can return an error after installation; retry is safe.
    pub fn publish(&self, bytes: &[u8]) -> Result<(ContentDigest, Published), StorageError> {
        if bytes.len() > self.cap {
            return Err(StorageError::LimitExceeded);
        }
        let digest = Sha256::digest(bytes).map_err(|_| StorageError::LimitExceeded)?;
        let destination = self.location(&digest, true)?;
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                self.compare(&destination, &digest, bytes)?;
                return Ok((digest, Published::AlreadyPresent));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(io_error(e)),
        }
        let (mut file, mut temp) = self.temporary()?;
        file.write_all(bytes).map_err(io_error)?;
        file.flush().map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        // Repeat containment preflight immediately before installation.
        let destination = self.location(&digest, false)?;
        let outcome = match fs::hard_link(&temp.path, &destination) {
            Ok(()) => Published::Created,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Published::AlreadyPresent,
            Err(e) => return Err(io_error(e)),
        };
        self.compare(&destination, &digest, bytes)?;
        temp.remove().map_err(io_error)?;
        Ok((digest, outcome))
    }

    fn temporary(&self) -> Result<(File, Temporary), StorageError> {
        self.check_root()?;
        let blobs = self.root.join("blobs");
        self.check_directory(&blobs, false)?;
        for _ in 0..1024 {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = blobs.join(format!(".publish-{id:016x}"));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((file, Temporary::new(path))),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_error(e)),
            }
        }
        Err(StorageError::Io(io::ErrorKind::AlreadyExists))
    }

    /// Returns complete bytes only after size, entry, containment and SHA-256 verification.
    pub fn read(&self, digest: &ContentDigest) -> Result<Vec<u8>, StorageError> {
        let path = self.location(digest, false)?;
        self.read_file(&path, digest)
    }

    /// Reads a verified resource with an operation-specific cap checked before reservation.
    /// The cap can only tighten this store's bound; this adds no new storage mechanism.
    pub fn read_capped(&self, digest: &ContentDigest, cap: usize) -> Result<Vec<u8>, StorageError> {
        let scoped = Self {
            cap: self.cap.min(cap),
            ..self.clone()
        };
        scoped.read(digest)
    }

    /// Checks presence with complete verification; corrupt/unsafe entries are errors, not false.
    pub fn contains(&self, digest: &ContentDigest) -> Result<bool, StorageError> {
        match self.verify(digest) {
            Ok(()) => Ok(true),
            Err(StorageError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Verifies all stored bytes, discarding the capped returned buffer.
    pub fn verify(&self, digest: &ContentDigest) -> Result<(), StorageError> {
        self.read(digest).map(|_| ())
    }
}

/// A temporary file name owned by one publisher until it is removed.
struct Temporary {
    path: PathBuf,
    armed: bool,
}

impl Temporary {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    /// Removes the file and disarms the guard, so the name is never removed a second time. Another
    /// publisher process may create the same name as soon as it is free, and its file must survive.
    fn remove(&mut self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.armed = false;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        // Best-effort cleanup on error paths only; a successful `remove` disarms the guard.
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    // This module contains only synthetic test setup and assertions.
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn matching_digest_still_requires_full_byte_equality() {
        // Synthetic collision injection at the private comparison boundary: the existing
        // file verifies under this digest, but the proposed bytes are deliberately different.
        // This exercises collision handling without claiming a real SHA-256 collision.
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("u05-tests");
        fs::create_dir_all(&base).unwrap();
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = base.join(format!("collision-{time}"));
        fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let base = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("target")
                    .join("u05-tests");
                assert!(self.0.is_absolute() && self.0.starts_with(&base) && self.0 != base);
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let cleanup = Cleanup(root);
        let store = BlobStore::new(&cleanup.0).unwrap();
        let (digest, _) = store.publish(b"verified existing bytes").unwrap();
        let path = store.location(&digest, false).unwrap();
        assert_eq!(
            store.compare(&path, &digest, b"synthetic collision bytes"),
            Err(StorageError::IntegrityConflict)
        );
        assert_eq!(store.read(&digest).unwrap(), b"verified existing bytes");
    }

    #[test]
    fn a_removed_temporary_name_taken_by_another_publisher_survives_the_guard() {
        // Interleaving: this publisher removes its temporary name; before the guard is dropped
        // another publisher process creates the same (now free) name. The guard must not delete it.
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("u05-tests");
        fs::create_dir_all(&base).unwrap();
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = base.join(format!("temporary-guard-{time}"));
        fs::write(&path, b"mine").unwrap();
        let mut guard = Temporary::new(path.clone());
        guard.remove().unwrap();
        assert!(!path.exists());
        fs::write(&path, b"other publisher").unwrap();
        drop(guard);
        assert_eq!(fs::read(&path).unwrap(), b"other publisher");
        fs::remove_file(&path).unwrap();

        // An armed guard (an error path) still cleans up its own file.
        fs::write(&path, b"abandoned").unwrap();
        drop(Temporary::new(path.clone()));
        assert!(!path.exists());
    }
}
