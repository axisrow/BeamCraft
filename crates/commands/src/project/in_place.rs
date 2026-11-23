// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

use super::{
    ProjectContext, ProjectEnvironmentChoice, ProjectLayout, first_revision, invalid, refuse,
};
use beamcraft_host::revision::CommitHook;
use beamcraft_host::storage::BlobStore;
use beamcraft_model::error::ErrorCode;
use session::Refusal;
use session::project::{ProjectInfo, check_project_name, project_info};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const STAGING_DIR: &str = ".beamcraft-init";
const MAX_ENTRIES: usize = 32_768;
const MAX_DEPTH: usize = 256;

/// Initializes an existing empty directory as a project and takes its name from the directory.
///
/// The first revision and the caller's optional setup are prepared in a private staging directory.
/// The staging directory also reserves initialization across processes. `environment` records the
/// chosen environment in the first revision's root layer, as `create-project` does for a named
/// folder (#628). An error keeps the destination directory and rolls back the entries installed
/// by this call.
pub fn create_project_in_place<T>(
    dir: &Path,
    layout: Option<&ProjectLayout>,
    context: &ProjectContext,
    hook: &mut dyn CommitHook,
    environment: Option<&ProjectEnvironmentChoice>,
    prepare: impl FnOnce(&Path) -> Result<T, Refusal>,
) -> Result<(ProjectInfo, T), Refusal> {
    let dir = std::path::absolute(dir).map_err(|_| invalid("the path cannot be made absolute"))?;
    let name = dir
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| invalid("the project folder has no UTF-8 name"))?;
    check_project_name(name)?;
    if dir.to_str().is_none() {
        return Err(invalid("the project path is not UTF-8"));
    }
    BlobStore::new(&dir).map_err(|error| refuse(error.code(), "store root is not usable"))?;
    require_empty(&dir, None)?;
    let staging_path = dir.join(STAGING_DIR);
    fs::create_dir(&staging_path).map_err(folder_error)?;
    let staging = Staging(staging_path);
    require_empty(&dir, Some(OsStr::new(STAGING_DIR)))?;

    let store = staging.0.join("store");
    fs::create_dir(&store).map_err(folder_error)?;
    first_revision(&store, context, hook, environment, layout)?;
    let prepared = prepare(&store)?;
    let info = project_info(&store)?;
    if !info.state.is_ready() {
        return Err(refuse(
            ErrorCode::IntegrityError,
            "the new project has no readable head",
        ));
    }
    require_empty(&dir, Some(OsStr::new(STAGING_DIR)))?;
    let mut published = Published::default();
    published.install(&store, &dir, 0)?;
    // Only expose the store to project discovery once its other entries are in place.
    published.install(&store.join("revisions"), &dir.join("revisions"), 1)?;
    published.entries.clear();
    Ok((
        ProjectInfo {
            name: name.to_owned(),
            path: dir,
            state: info.state,
        },
        prepared,
    ))
}

fn require_empty(dir: &Path, except: Option<&OsStr>) -> Result<(), Refusal> {
    for entry in fs::read_dir(dir).map_err(folder_error)? {
        let entry = entry.map_err(folder_error)?;
        if Some(entry.file_name().as_os_str()) != except {
            return Err(refuse(
                ErrorCode::DuplicateId,
                "the project folder is not empty",
            ));
        }
    }
    Ok(())
}

fn folder_error(error: io::Error) -> Refusal {
    match error.kind() {
        io::ErrorKind::AlreadyExists => refuse(
            ErrorCode::DuplicateId,
            "the project folder entry already exists",
        ),
        io::ErrorKind::NotFound => refuse(
            ErrorCode::NotFound,
            "the project folder entry was not found",
        ),
        io::ErrorKind::PermissionDenied => refuse(
            ErrorCode::PermissionDenied,
            "the project folder is not accessible",
        ),
        _ => refuse(
            ErrorCode::BackendUnavailable,
            "the project folder could not be initialized",
        ),
    }
}

struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        // This is the private directory this call reserved, never the caller's directory.
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct Published {
    entries: Vec<(PathBuf, bool)>,
}

impl Published {
    fn install(&mut self, from: &Path, to: &Path, depth: usize) -> Result<(), Refusal> {
        if self.entries.len() >= MAX_ENTRIES || depth > MAX_DEPTH {
            return Err(refuse(
                ErrorCode::LimitExceeded,
                "the staged project is too large",
            ));
        }
        let kind = fs::symlink_metadata(from)
            .map_err(folder_error)?
            .file_type();
        if kind.is_dir() {
            if depth > 0 {
                fs::create_dir(to).map_err(folder_error)?;
                self.entries.push((to.to_path_buf(), true));
            }
            for entry in fs::read_dir(from).map_err(folder_error)? {
                let entry = entry.map_err(folder_error)?;
                let name = entry.file_name();
                if depth == 0 && name == "revisions" {
                    continue;
                }
                self.install(&entry.path(), &to.join(name), depth + 1)?;
            }
        } else if kind.is_file() {
            // Hard links publish atomically without replacing a destination, as the store does.
            fs::hard_link(from, to).map_err(folder_error)?;
            self.entries.push((to.to_path_buf(), false));
        } else {
            return Err(invalid("the staged project contains an unsupported entry"));
        }
        Ok(())
    }
}

impl Drop for Published {
    fn drop(&mut self) {
        for (path, directory) in self.entries.drain(..).rev() {
            if directory {
                let _ = fs::remove_dir(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
    }
}
