// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Personal views are an independent bounded file, never a model revision layer.
use beamcraft_model::{
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
/// Byte limit of one personal views document.
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
/// Fixed, safe storage failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewsError {
    /// Domain error code.
    pub code: ErrorCode,
    /// Fixed explanation without a caller path.
    pub message: &'static str,
}
fn error(code: ErrorCode, message: &'static str) -> ViewsError {
    ViewsError { code, message }
}
fn io(_: std::io::Error) -> ViewsError {
    error(
        ErrorCode::BackendUnavailable,
        "personal views file is not usable",
    )
}
/// The bytes read and the exact digest used for compare-and-swap.
#[derive(Debug, Clone)]
pub struct PersonalBytes {
    /// Missing on a fresh personal workspace.
    pub bytes: Option<Vec<u8>>,
    /// Missing exactly when the file is absent.
    pub digest: Option<ContentDigest>,
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
/// Reads one contained regular personal document without accepting a symbolic link.
pub fn read(root: &Path) -> Result<PersonalBytes, ViewsError> {
    let path = root.join("views.usda");
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersonalBytes {
                bytes: None,
                digest: None,
            });
        }
        Err(e) => return Err(io(e)),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(error(
            ErrorCode::PermissionDenied,
            "views document must be a regular file",
        ));
    }
    if meta.len() > MAX_BYTES as u64 {
        return Err(error(
            ErrorCode::LimitExceeded,
            "views document exceeds its byte limit",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(io)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > MAX_BYTES {
        return Err(error(
            ErrorCode::LimitExceeded,
            "views document exceeds its byte limit",
        ));
    }
    let digest = Sha256::digest(&bytes)
        .map_err(|_| error(ErrorCode::LimitExceeded, "views digest limit exceeded"))?;
    Ok(PersonalBytes {
        bytes: Some(bytes),
        digest: Some(digest),
    })
}
/// Atomically publish a complete validated document if its previous bytes still match.
/// A held/crashed writer lock refuses explicitly; it is never silently stolen.
pub fn compare_and_swap(
    root: &Path,
    expected: Option<ContentDigest>,
    bytes: &[u8],
) -> Result<ContentDigest, ViewsError> {
    if bytes.len() > MAX_BYTES {
        return Err(error(
            ErrorCode::LimitExceeded,
            "views document exceeds its byte limit",
        ));
    }
    let lock_path = root.join(".views.lock");
    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                error(
                    ErrorCode::StaleWorkspace,
                    "another personal views writer holds the lock",
                )
            } else {
                io(e)
            }
        })?;
    let _guard = Lock(lock_path);
    drop(lock);
    if read(root)?.digest != expected {
        return Err(error(
            ErrorCode::StaleWorkspace,
            "personal views changed since the supplied version",
        ));
    }
    crate::output::publish_file(&root.join("views.usda"), true, |file| {
        file.write_all(bytes)
            .map_err(|_| crate::output::OutputError {
                code: ErrorCode::BackendUnavailable,
                message: "personal views could not be written",
            })
    })
    .map_err(|e| error(e.code, e.message))?;
    Sha256::digest(bytes)
        .map_err(|_| error(ErrorCode::LimitExceeded, "views digest limit exceeded"))
}
