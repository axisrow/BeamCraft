// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U14 adapter: the application-layer wiring around [`beamcraft_host::compaction`] that the host
//! cannot do itself. It finds the live personal workspaces that pin revisions of the store, makes
//! the verified backup a real compaction requires, refuses to compact a workspace's own draft
//! store, and holds the store lease of a running coordinator.
//!
//! Nothing here decides what is kept or deleted: the host engine does, and refuses what is unsafe.
//! This module only supplies facts (which revisions workspaces are based on) and safeguards.

use crate::plate::PlateError;
use crate::workspace::read_meta;
use beamcraft_host::compaction::{CompactionError, Pin};
use beamcraft_host::lease::{LeaseError, StoreLease};
use beamcraft_host::revision::RevisionStore;
use beamcraft_host::storage::BlobStore;
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Most directories one `--workspaces-root` scan looks at.
const MAX_WORKSPACES: usize = 4096;
/// Most entries (files and directories) one backup copies.
const MAX_BACKUP_ENTRIES: usize = 2_000_000;
/// Deepest directory nesting a store has (`blobs/ab/<digest>` is two below the root).
const MAX_DEPTH: usize = 4;

fn refuse(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError::new(code, message)
}

fn invalid(message: &'static str) -> PlateError {
    refuse(ErrorCode::InvalidArgument, message)
}

/// Whether `root` is a personal workspace's draft store (it holds `workspace.json`). Such a store
/// carries the undo history of a draft and is never compacted.
pub fn is_workspace_store(root: &Path) -> bool {
    fs::symlink_metadata(root.join("workspace.json")).is_ok()
}

/// The revisions of the store that live workspaces are based on.
///
/// `dirs` names workspace directories (each must hold valid workspace metadata); `roots` name
/// directories whose immediate children are workspaces (children without `workspace.json` are not
/// workspaces and are skipped; a child with unusable metadata refuses the whole scan, because a
/// workspace that cannot be read cannot be protected). `store` itself is never a workspace.
pub fn collect_pins(
    store: &Path,
    roots: &[PathBuf],
    dirs: &[PathBuf],
) -> Result<Vec<Pin>, PlateError> {
    let own = fs::canonicalize(store).ok();
    let mut seen = BTreeSet::new();
    let mut pins = Vec::new();
    let mut add = |dir: &Path, required: bool| -> Result<(), PlateError> {
        let canonical = fs::canonicalize(dir)
            .map_err(|_| refuse(ErrorCode::NotFound, "a workspace directory does not exist"))?;
        if own.as_ref() == Some(&canonical) {
            return Err(invalid("the store itself is named as a workspace"));
        }
        if !required && !is_workspace_store(&canonical) {
            return Ok(());
        }
        if !seen.insert(canonical) {
            return Ok(());
        }
        let meta = read_meta(dir)?;
        pins.push(Pin {
            label: dir.display().to_string(),
            revision: meta.base_revision,
        });
        Ok(())
    };
    for dir in dirs {
        add(dir, true)?;
    }
    for root in roots {
        let entries = fs::read_dir(root)
            .map_err(|_| refuse(ErrorCode::NotFound, "the workspaces root does not exist"))?;
        let mut children = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|_| {
                refuse(
                    ErrorCode::BackendUnavailable,
                    "the workspaces root is unreadable",
                )
            })?;
            if children.len() >= MAX_WORKSPACES {
                return Err(refuse(
                    ErrorCode::LimitExceeded,
                    "the workspaces root holds too many entries",
                ));
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|_| {
                refuse(
                    ErrorCode::BackendUnavailable,
                    "the workspaces root is unreadable",
                )
            })?;
            if metadata.is_dir() {
                children.push(entry.path());
            }
        }
        children.sort();
        for child in children {
            add(&child, false)?;
        }
    }
    Ok(pins)
}

fn backup_failed(code: ErrorCode) -> CompactionError {
    CompactionError::SafeguardFailed(code)
}

/// Whether the backup directory is a usable, separate destination: absolute, not inside the store,
/// not containing it, and absent or an empty directory.
fn check_destination(store: &Path, destination: &Path) -> Result<(), CompactionError> {
    if !destination.is_absolute() {
        return Err(backup_failed(ErrorCode::InvalidArgument));
    }
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.is_dir() => {
            let empty = fs::read_dir(destination).is_ok_and(|mut entries| entries.next().is_none());
            if !empty {
                return Err(backup_failed(ErrorCode::InvalidArgument));
            }
        }
        Ok(_) => return Err(backup_failed(ErrorCode::InvalidArgument)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(backup_failed(ErrorCode::BackendUnavailable)),
    }
    let store =
        fs::canonicalize(store).map_err(|_| backup_failed(ErrorCode::BackendUnavailable))?;
    // The destination may not exist yet: resolve its nearest existing ancestor.
    let mut existing = destination.to_path_buf();
    let mut tail = Vec::new();
    while !existing.exists() {
        match (
            existing.file_name().map(ToOwned::to_owned),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                tail.push(name);
                existing = parent.to_path_buf();
            }
            _ => return Err(backup_failed(ErrorCode::InvalidArgument)),
        }
    }
    let mut resolved =
        fs::canonicalize(&existing).map_err(|_| backup_failed(ErrorCode::BackendUnavailable))?;
    resolved.extend(tail.iter().rev());
    if resolved.starts_with(&store) || store.starts_with(&resolved) {
        return Err(backup_failed(ErrorCode::InvalidArgument));
    }
    Ok(())
}

/// Every regular file below `root` with its size, as relative paths. Links and other entries are
/// refused: a store holds only directories and regular files.
fn tree(root: &Path) -> Result<Vec<(PathBuf, u64)>, CompactionError> {
    fn visit(
        root: &Path,
        dir: &Path,
        depth: usize,
        out: &mut Vec<(PathBuf, u64)>,
    ) -> Result<(), CompactionError> {
        if depth > MAX_DEPTH {
            return Err(backup_failed(ErrorCode::LimitExceeded));
        }
        let entries =
            fs::read_dir(dir).map_err(|_| backup_failed(ErrorCode::BackendUnavailable))?;
        for entry in entries {
            let entry = entry.map_err(|_| backup_failed(ErrorCode::BackendUnavailable))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| backup_failed(ErrorCode::BackendUnavailable))?;
            let relative = path
                .strip_prefix(root)
                .map_err(|_| backup_failed(ErrorCode::IntegrityError))?
                .to_path_buf();
            if relative == Path::new("maintenance.lock") {
                continue;
            }
            if out.len() >= MAX_BACKUP_ENTRIES {
                return Err(backup_failed(ErrorCode::LimitExceeded));
            }
            if metadata.is_dir() {
                visit(root, &path, depth.saturating_add(1), out)?;
            } else if metadata.is_file() {
                out.push((relative, metadata.len()));
            } else {
                return Err(backup_failed(ErrorCode::IntegrityError));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    visit(root, root, 0, &mut out)?;
    out.sort();
    Ok(out)
}

fn copy_all(
    source: &Path,
    destination: &Path,
    files: &[(PathBuf, u64)],
) -> Result<(), CompactionError> {
    for (relative, _) in files {
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| backup_failed(ErrorCode::BackendUnavailable))?;
        }
        fs::copy(source.join(relative), &target)
            .map_err(|_| backup_failed(ErrorCode::BackendUnavailable))?;
    }
    Ok(())
}

/// Copies the whole store to `destination` and verifies the copy: the same files with the same
/// sizes, the small control files (head log, floor marker, ledger, claims) byte for byte, and the
/// copy's head opens with every blob verified. The caller holds the exclusive store lease, so the
/// copy is a consistent snapshot. A destination that is not absolute, not empty, or inside (or
/// around) the store is refused; a failed copy is removed again.
pub fn backup_store(store: &Path, destination: &Path) -> Result<(), CompactionError> {
    check_destination(store, destination)?;
    let outcome = copy_and_verify(store, destination);
    if outcome.is_err() {
        // The destination was absent or empty before: nothing of the user's is removed.
        let _ = fs::remove_dir_all(destination);
    }
    outcome
}

fn copy_and_verify(store: &Path, destination: &Path) -> Result<(), CompactionError> {
    fs::create_dir_all(destination).map_err(|_| backup_failed(ErrorCode::BackendUnavailable))?;
    let files = tree(store)?;
    copy_all(store, destination, &files)?;
    let copied = tree(destination)?;
    if copied != files {
        return Err(backup_failed(ErrorCode::IntegrityError));
    }
    for (relative, _) in &files {
        if relative.starts_with("blobs") {
            continue;
        }
        let same = fs::read(store.join(relative)).ok() == fs::read(destination.join(relative)).ok();
        if !same {
            return Err(backup_failed(ErrorCode::IntegrityError));
        }
    }
    let blobs = BlobStore::new(destination).map_err(|e| backup_failed(e.code()))?;
    RevisionStore::new(blobs)
        .open()
        .map_err(|e| backup_failed(e.code()))?;
    Ok(())
}

/// The lease a running `serve-coordinator` holds on its store for as long as it serves, so that a
/// compaction cannot start under it. `Ok(None)` for every other command, and when the `--store`
/// value is missing or unusable (the command then reports its own error).
pub fn hold_coordinator_lease(
    args: &[std::ffi::OsString],
) -> Result<Option<StoreLease>, PlateError> {
    if args
        .first()
        .is_none_or(|command| command != "serve-coordinator")
    {
        return Ok(None);
    }
    let store = args
        .iter()
        .position(|arg| arg == "--store")
        .and_then(|at| args.get(at.saturating_add(1)))
        .and_then(|value| value.to_str());
    let Some(blobs) = store.and_then(|root| BlobStore::new(root).ok()) else {
        return Ok(None);
    };
    match StoreLease::shared(&blobs) {
        Ok(lease) => Ok(Some(lease)),
        Err(LeaseError::Busy) => Err(refuse(
            ErrorCode::BackendUnavailable,
            "the store is being compacted",
        )),
        Err(LeaseError::Storage(error)) => Err(refuse(error.code(), "the store lock is unusable")),
    }
}
