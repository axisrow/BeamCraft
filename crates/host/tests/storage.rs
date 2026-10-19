// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Public-API end-to-end tests against the real filesystem; no storage mocks.

// Test-only setup helpers may fail loudly; production code has no such allowances.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use beamcraft_host::storage::{BlobStore, MAX_BLOB_BYTES, Published, StorageError};
use beamcraft_model::{
    ErrorCode,
    digest::{ContentDigest, Sha256},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("u05-tests");
        fs::create_dir_all(&base).unwrap();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = base.join(format!("test-{time}-{id}"));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn store(&self) -> BlobStore {
        BlobStore::new(&self.0).unwrap()
    }

    fn blob(&self, digest: ContentDigest) -> PathBuf {
        let name = digest.to_string();
        self.0.join("blobs").join(&name[..2]).join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // The absolute, original test directory must remain inside this worktree's test area.
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("u05-tests");
        assert!(self.0.is_absolute() && self.0.starts_with(&base) && self.0 != base);
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn round_trip_republish_missing_and_empty() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let missing = Sha256::digest(b"missing").unwrap();
    assert!(!store.contains(&missing).unwrap());
    assert_eq!(store.read(&missing), Err(StorageError::NotFound));
    for data in [b"synthetic immutable bytes\0\xff".as_slice(), b""] {
        let (digest, status) = store.publish(data).unwrap();
        assert_eq!(status, Published::Created);
        let path = fixture.blob(digest);
        assert_eq!(path.file_name().unwrap().to_str().unwrap().len(), 64);
        // Set a distinctive old timestamp so filesystem timestamp resolution cannot hide
        // a rewrite occurring immediately after the original publication.
        let old_time = UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old_time))
            .unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            store.publish(data).unwrap(),
            (digest, Published::AlreadyPresent)
        );
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), before);
        assert_eq!(store.read(&digest).unwrap(), data);
        assert!(store.contains(&digest).unwrap());
        store.verify(&digest).unwrap();
    }
    assert_eq!(fs::read_dir(fixture.0.join("blobs")).unwrap().count(), 2);
}

#[test]
fn tampered_truncated_and_wrong_named_bytes_are_never_overwritten() {
    for replacement in [b"different same-length bytes".as_slice(), b"cut", b""] {
        let fixture = Fixture::new();
        let store = fixture.store();
        let original = b"original immutable contents";
        let (digest, _) = store.publish(original).unwrap();
        let path = fixture.blob(digest);
        fs::write(&path, replacement).unwrap();
        for error in [
            store.read(&digest).unwrap_err(),
            store.verify(&digest).unwrap_err(),
            store.contains(&digest).unwrap_err(),
        ] {
            assert_eq!(error, StorageError::DigestMismatch);
            assert_eq!(error.code(), ErrorCode::IntegrityError);
        }
        assert_eq!(
            store.publish(original),
            Err(StorageError::IntegrityConflict)
        );
        assert_eq!(fs::read(&path).unwrap(), replacement);
        assert!(
            !store
                .read(&digest)
                .unwrap_err()
                .to_string()
                .contains(&fixture.0.display().to_string())
        );
    }
    let fixture = Fixture::new();
    let store = fixture.store();
    let (digest, _) = store.publish(b"first bytes").unwrap();
    let other = Sha256::digest(b"other bytes").unwrap();
    let wrong = fixture.blob(other);
    fs::create_dir_all(wrong.parent().unwrap()).unwrap();
    fs::copy(fixture.blob(digest), wrong).unwrap();
    assert_eq!(store.read(&other), Err(StorageError::DigestMismatch));
    assert_eq!(
        store.publish(b"other bytes"),
        Err(StorageError::IntegrityConflict)
    );
}

#[test]
fn size_caps_including_real_maximum_and_growing_file() {
    let fixture = Fixture::new();
    let store = BlobStore::with_limit(&fixture.0, 64).unwrap();
    let (digest, _) = store.publish(&[1; 64]).unwrap();
    assert_eq!(store.read(&digest).unwrap(), [1; 64]);
    assert_eq!(store.publish(&[1; 65]), Err(StorageError::LimitExceeded));
    fs::write(fixture.blob(digest), [1; 65]).unwrap();
    assert_eq!(store.read(&digest), Err(StorageError::LimitExceeded));
    assert_eq!(
        store.publish(&[1; 64]),
        Err(StorageError::IntegrityConflict)
    );
    assert!(matches!(
        BlobStore::with_limit(&fixture.0, MAX_BLOB_BYTES + 1),
        Err(StorageError::InvalidArgument)
    ));
    let empty = BlobStore::with_limit(&fixture.0, 0).unwrap();
    let (digest, _) = empty.publish(b"").unwrap();
    assert!(empty.read(&digest).unwrap().is_empty());
    assert_eq!(empty.publish(b"x"), Err(StorageError::LimitExceeded));
    let store = fixture.store();
    let mut data = vec![0x5a; MAX_BLOB_BYTES];
    let (digest, _) = store.publish(&data).unwrap();
    assert_eq!(store.read(&digest).unwrap(), data);
    data.push(0);
    assert_eq!(store.publish(&data), Err(StorageError::LimitExceeded));
}

#[test]
fn concurrent_publishers_install_once_and_cleanup() {
    let fixture = Fixture::new();
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = fixture.store();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            store.publish(&[42; 8192]).unwrap()
        }));
    }
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        results
            .iter()
            .filter(|(_, p)| *p == Published::Created)
            .count(),
        1
    );
    assert!(results.iter().all(|(d, _)| *d == results[0].0));
    assert_eq!(fixture.store().read(&results[0].0).unwrap(), [42; 8192]);
    for entry in fs::read_dir(fixture.0.join("blobs")).unwrap() {
        assert!(
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".publish-")
        );
    }
}

#[test]
fn roots_and_non_regular_entries_are_rejected() {
    let fixture = Fixture::new();
    assert!(matches!(
        BlobStore::new("relative"),
        Err(StorageError::InvalidArgument)
    ));
    assert!(matches!(
        BlobStore::new(fixture.0.join("absent")),
        Err(StorageError::InvalidArgument)
    ));
    let file = fixture.0.join("ordinary-file");
    fs::write(&file, b"").unwrap();
    assert!(matches!(
        BlobStore::new(file),
        Err(StorageError::UnsafeEntry)
    ));
    let store = fixture.store();
    let (digest, _) = store.publish(b"directory fixture").unwrap();
    let path = fixture.blob(digest);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(store.read(&digest), Err(StorageError::UnsafeEntry));
    assert_eq!(
        store.publish(b"directory fixture"),
        Err(StorageError::UnsafeEntry)
    );
    assert_eq!(store.contains(&digest), Err(StorageError::UnsafeEntry));
}

#[cfg(any(unix, windows))]
fn symlink(source: &Path, target: &Path, directory: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let _ = directory;
        std::os::unix::fs::symlink(source, target)
    }
    #[cfg(windows)]
    {
        if directory {
            std::os::windows::fs::symlink_dir(source, target)
        } else {
            std::os::windows::fs::symlink_file(source, target)
        }
    }
}

#[test]
#[cfg(any(unix, windows))]
fn links_at_blob_shard_blobs_root_and_ancestor_are_rejected() {
    let fixture = Fixture::new();
    let outside = Fixture::new(); // Separate authorized test root, still inside this worktree.
    let target = outside.0.join("untouched");
    fs::write(&target, b"outside bytes").unwrap();
    let store = fixture.store();
    let (digest, _) = store.publish(b"link fixture").unwrap();
    let blob = fixture.blob(digest);
    fs::remove_file(&blob).unwrap();
    if let Err(e) = symlink(&target, &blob, false) {
        if e.kind() == std::io::ErrorKind::PermissionDenied
            || e.kind() == std::io::ErrorKind::Unsupported
        {
            eprintln!(
                "SKIP symlink/reparse fixtures: platform denied link creation ({:?})",
                e.kind()
            );
            return;
        }
        panic!("unexpected symlink fixture failure: {e}");
    }
    assert_eq!(store.read(&digest), Err(StorageError::UnsafeEntry));
    assert_eq!(
        store.publish(b"link fixture"),
        Err(StorageError::UnsafeEntry)
    );
    fs::remove_file(&blob).unwrap();
    let shard = blob.parent().unwrap();
    fs::remove_dir(shard).unwrap();
    symlink(&outside.0, shard, true).unwrap();
    assert_eq!(store.read(&digest), Err(StorageError::UnsafeEntry));
    assert_eq!(
        store.publish(b"link fixture"),
        Err(StorageError::UnsafeEntry)
    );
    fs::remove_dir(shard)
        .or_else(|_| fs::remove_file(shard))
        .unwrap();
    let blobs = fixture.0.join("blobs");
    fs::remove_dir(&blobs).unwrap();
    symlink(&outside.0, &blobs, true).unwrap();
    assert_eq!(
        store.publish(b"link fixture"),
        Err(StorageError::UnsafeEntry)
    );
    fs::remove_dir(&blobs)
        .or_else(|_| fs::remove_file(&blobs))
        .unwrap();
    let root_link = fixture.0.join("root-link");
    symlink(&outside.0, &root_link, true).unwrap();
    assert!(matches!(
        BlobStore::new(&root_link),
        Err(StorageError::UnsafeEntry)
    ));
    fs::create_dir(outside.0.join("child")).unwrap();
    // Off macOS an ancestor link stays rejected. On macOS any ancestor link is resolved
    // instead (system links such as /var -> /private/var sit before ordinary roots; this
    // user-created link passes too) and the store binds to the resolved directory.
    #[cfg(target_os = "macos")]
    assert!(BlobStore::new(root_link.join("child")).is_ok());
    #[cfg(not(target_os = "macos"))]
    assert!(matches!(
        BlobStore::new(root_link.join("child")),
        Err(StorageError::UnsafeEntry)
    ));
    fs::remove_dir(&root_link)
        .or_else(|_| fs::remove_file(&root_link))
        .unwrap();
    assert_eq!(fs::read(target).unwrap(), b"outside bytes");
}

#[test]
fn invalid_digest_text_cannot_become_a_resource_path() {
    for text in [
        "",
        "abc",
        "../escape",
        "C:\\private\\blob",
        "/etc/passwd",
        "https://example.test/a",
        &"A".repeat(64),
        &"0".repeat(63),
        &"0".repeat(65),
        &format!("{} ", "0".repeat(63)),
    ] {
        assert!(text.parse::<ContentDigest>().is_err(), "{text}");
    }
}

#[test]
#[cfg(windows)]
fn remote_and_device_roots_are_rejected_before_io() {
    for root in [
        r"\\example.invalid\share",
        r"\\?\UNC\example.invalid\share",
        r"\\.\C:\",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1\",
    ] {
        assert!(
            matches!(BlobStore::new(root), Err(StorageError::InvalidArgument)),
            "{root}"
        );
    }
}

#[test]
fn stable_error_mapping_and_safe_messages() {
    for (error, code) in [
        (StorageError::InvalidArgument, ErrorCode::InvalidArgument),
        (StorageError::UnsafeEntry, ErrorCode::IntegrityError),
        (StorageError::NotFound, ErrorCode::NotFound),
        (StorageError::DigestMismatch, ErrorCode::IntegrityError),
        (StorageError::IntegrityConflict, ErrorCode::IntegrityError),
        (StorageError::LimitExceeded, ErrorCode::LimitExceeded),
        (
            StorageError::Io(std::io::ErrorKind::PermissionDenied),
            ErrorCode::PermissionDenied,
        ),
        (
            StorageError::Io(std::io::ErrorKind::Unsupported),
            ErrorCode::BackendUnavailable,
        ),
    ] {
        assert_eq!(error.code(), code);
        assert!(!error.to_string().is_empty());
    }
}
