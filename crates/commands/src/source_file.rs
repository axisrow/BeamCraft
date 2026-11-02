// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Bounded read of one local file named by the user, shared by the byte-storage commands and the
//! R01 reference attachment. The checks are the same everywhere: a regular file opened without
//! following a replaced reparse point or blocking on a FIFO, at most `cap` bytes, and no
//! network or device path (on Windows only drive-letter paths are accepted).

use beamcraft_model::ErrorCode;
use std::io::Read;

/// Reads at most `cap` bytes of the regular file at `path`.
///
/// `INVALID_ARGUMENT` for a path or file kind that is not accepted, `NOT_FOUND` when it cannot be
/// opened, `LIMIT_EXCEEDED` when it is larger than `cap`.
pub fn read_file(path: &str, cap: usize) -> Result<Vec<u8>, ErrorCode> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Check the opened handle below; a path metadata check cannot close a replacement race.
    // O_NONBLOCK prevents a FIFO replacement from blocking the open itself.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        const O_NONBLOCK: i32 = 0x800;
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        const O_NONBLOCK: i32 = 0x4;
        #[cfg(not(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        )))]
        return Err(ErrorCode::InvalidArgument);
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        options.custom_flags(O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::{
            os::windows::fs::{MetadataExt, OpenOptionsExt},
            path::{Component, Prefix},
        };
        if matches!(std::path::Path::new(path).components().next(), Some(Component::Prefix(p)) if !matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err(ErrorCode::InvalidArgument);
        }
        let metadata = std::fs::symlink_metadata(path).map_err(|_| ErrorCode::NotFound)?;
        if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
            return Err(ErrorCode::InvalidArgument);
        }
        // Open a concurrently substituted reparse point itself rather than its target.
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(|_| ErrorCode::NotFound)?;
    let metadata = file.metadata().map_err(|_| ErrorCode::InvalidArgument)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(ErrorCode::InvalidArgument);
        }
    }
    // The kind is decided before any size: st_size of a directory or a device means nothing.
    if !metadata.is_file() {
        return Err(ErrorCode::InvalidArgument);
    }
    if metadata.len() > cap as u64 {
        return Err(ErrorCode::LimitExceeded);
    }
    let mut data = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| ErrorCode::InvalidArgument)?;
    if data.len() > cap {
        return Err(ErrorCode::LimitExceeded);
    }
    Ok(data)
}
