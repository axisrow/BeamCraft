// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Projects as folders, as people see them in a file manager.
//!
//! A project is a store directory (`blobs/`, `revisions/`, `ledger/`). Its name is the name of its
//! folder. [`list_projects`] lists the projects directly inside one folder without writing
//! anything; [`project_info`] reads one; [`next_project_name`] suggests a free folder name.
//!
//! Dates come from identities, not from file times: a ProjectId and a revision id are UUIDv7, so
//! the creation time is the timestamp of the ProjectId and the last change is the timestamp of the
//! head revision's id. An identity named explicitly by a caller carries whatever time it holds.
//!
//! Creating a project writes a first revision and needs the model and the layer engine; that is a
//! command and stays with the command line.

use crate::Refusal;
use beamcraft_host::revision::{RevisionInfo, RevisionStore};
use beamcraft_host::storage::BlobStore;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::Uuid7;
use std::fs;
use std::path::{Path, PathBuf};

/// Longest project folder name, in characters.
pub const MAX_NAME_CHARS: usize = 100;
/// Most directory entries [`list_projects`] looks at in one folder.
pub const MAX_SCANNED: usize = 4_096;
/// Most projects [`list_projects`] returns.
pub const MAX_LISTED: usize = 512;
/// Most numbered candidates [`next_project_name`] tries.
const MAX_CANDIDATES: u32 = 10_000;

const fn refuse(code: ErrorCode, message: &'static str) -> Refusal {
    Refusal::new(code, message)
}

const fn invalid(message: &'static str) -> Refusal {
    Refusal::invalid(message)
}

/// Checks a project name that becomes a folder name on Windows, Linux and macOS: 1 to
/// [`MAX_NAME_CHARS`] characters, no path separator, no `:*?"<>|`, no control character, no
/// leading or trailing space, no trailing dot, not `.` or `..` and not a reserved Windows device
/// name such as `CON` or `COM1` (with or without an extension). Letters of any script are allowed.
///
/// # Errors
/// A name that cannot be a portable folder name is `INVALID_ARGUMENT`.
pub fn check_project_name(name: &str) -> Result<(), Refusal> {
    let count = name.chars().count();
    if count == 0 || count > MAX_NAME_CHARS {
        return Err(invalid("the project name must be 1 to 100 characters"));
    }
    if name
        .chars()
        .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
    {
        return Err(invalid(
            "the project name contains a path separator, a control character or one of :*?\"<>|",
        ));
    }
    if name.trim() != name || name.ends_with('.') {
        return Err(invalid(
            "the project name starts or ends with a space or ends with a dot",
        ));
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.ends_with(|c: char| c.is_ascii_digit() && c != '0'));
    if reserved || name == "." || name == ".." {
        return Err(invalid(
            "the project name is reserved by the operating system",
        ));
    }
    Ok(())
}

/// The state of a project folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectState {
    /// A readable head revision.
    Ready {
        /// The ProjectId the head records.
        project_id: String,
        /// Creation time in Unix milliseconds: the timestamp of the ProjectId.
        created_ms: u64,
        /// Time of the last change in Unix milliseconds: the timestamp of the head revision id.
        modified_ms: u64,
        /// The head revision, as the host read it.
        head: Box<RevisionInfo>,
    },
    /// A store folder without a revision.
    Empty,
    /// A store whose head does not read; the code says why.
    Unreadable(ErrorCode),
    /// The folder does not exist or is not a store.
    Missing,
}

impl ProjectState {
    /// Whether the folder is a project with a readable head.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// One project folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    /// The project's folder, as found.
    pub path: PathBuf,
    /// The folder name; the project's name.
    pub name: String,
    /// What reading the folder found.
    pub state: ProjectState,
}

/// Whether `dir` looks like a store, by its `revisions/` directory. Nothing is opened or written.
#[must_use]
pub fn is_store(dir: &Path) -> bool {
    dir.join("revisions").is_dir()
}

/// Whether a remembered project has definitely disappeared. Read errors do not prove absence.
#[must_use]
pub fn project_is_missing(dir: &Path) -> bool {
    [dir.to_path_buf(), dir.join("revisions")]
        .iter()
        .any(|path| fs::metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound))
}

fn absolute(path: &Path) -> Result<PathBuf, Refusal> {
    std::path::absolute(path).map_err(|_| invalid("the path cannot be made absolute"))
}

/// Reads one project folder. The head is read without verifying every layer blob; opening the
/// project verifies it in full. Nothing is written.
///
/// # Errors
/// A path that cannot be made absolute or has no UTF-8 folder name is `INVALID_ARGUMENT`. A store
/// that does not read is not an error: it is [`ProjectState::Unreadable`].
pub fn project_info(dir: &Path) -> Result<ProjectInfo, Refusal> {
    let path = absolute(dir)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("the project folder has no UTF-8 name"))?
        .to_owned();
    let state = if is_store(&path) {
        match BlobStore::new(&path) {
            Ok(blobs) => match RevisionStore::new(blobs).head() {
                Ok(Some(info)) => {
                    let content = info.manifest().content();
                    let time = |text: &str| {
                        Uuid7::parse(text)
                            .map(|id| id.unix_ms())
                            .map_err(|_| refuse(ErrorCode::IntegrityError, "head identity"))
                    };
                    match (
                        time(content.project().as_str()),
                        time(content.revision().as_str()),
                    ) {
                        (Ok(created_ms), Ok(modified_ms)) => ProjectState::Ready {
                            project_id: content.project().as_str().to_owned(),
                            created_ms,
                            modified_ms,
                            head: Box::new(info),
                        },
                        _ => ProjectState::Unreadable(ErrorCode::IntegrityError),
                    }
                }
                Ok(None) => ProjectState::Empty,
                Err(error) => ProjectState::Unreadable(error.code()),
            },
            Err(error) => ProjectState::Unreadable(error.code()),
        }
    } else {
        ProjectState::Missing
    };
    Ok(ProjectInfo { path, name, state })
}

/// The projects directly inside `dir`, by name. Entries that are not store folders are left out;
/// at most [`MAX_SCANNED`] entries are looked at and [`MAX_LISTED`] projects returned, and the
/// second value says whether the folder held more.
///
/// # Errors
/// A folder that does not exist is `NOT_FOUND`; one that cannot be read is `INVALID_ARGUMENT`.
pub fn list_projects(dir: &Path) -> Result<(Vec<ProjectInfo>, bool), Refusal> {
    let dir = absolute(dir)?;
    let entries = fs::read_dir(&dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => refuse(ErrorCode::NotFound, "the folder does not exist"),
        _ => invalid("the folder cannot be read"),
    })?;
    let mut projects = Vec::new();
    let mut truncated = false;
    for (seen, entry) in entries.enumerate() {
        if seen >= MAX_SCANNED || projects.len() >= MAX_LISTED {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else { continue };
        // Links are not followed: a project is a real folder inside the listed one.
        if !entry.file_type().is_ok_and(|t| t.is_dir()) || !is_store(&entry.path()) {
            continue;
        }
        if let Ok(info) = project_info(&entry.path()) {
            projects.push(info);
        }
    }
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((projects, truncated))
}

/// `name`, `name 1`, `name 2`, ...: the candidate folder name with number `index`.
#[must_use]
pub fn numbered_name(name: &str, index: u32) -> String {
    if index == 0 {
        name.into()
    } else {
        format!("{name} {index}")
    }
}

/// Suggest a free folder name without writing anything. Creation still reserves it atomically.
///
/// # Errors
/// An invalid name or a missing parent folder is refused; so is a parent that cannot be inspected
/// or one with no free name within 10,000 candidates.
pub fn next_project_name(parent: &Path, name: &str) -> Result<String, Refusal> {
    check_project_name(name)?;
    if !parent.is_dir() {
        return Err(refuse(
            ErrorCode::NotFound,
            "the parent folder does not exist",
        ));
    }
    for index in 0..MAX_CANDIDATES {
        let candidate = numbered_name(name, index);
        check_project_name(&candidate)?;
        match fs::symlink_metadata(parent.join(&candidate)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => {}
            Err(_) => {
                return Err(refuse(
                    ErrorCode::PermissionDenied,
                    "the parent folder cannot be inspected",
                ));
            }
        }
    }
    Err(refuse(
        ErrorCode::LimitExceeded,
        "no free project name within the candidate limit",
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("session-project-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn project_names_are_portable_folder_names() {
        for good in [
            "Каркас A-F 1-14",
            "307-KM",
            "a",
            "Nave.v2",
            "COM0",
            "Console",
        ] {
            assert!(check_project_name(good).is_ok(), "{good}");
        }
        let long = "x".repeat(101);
        for bad in [
            "", " a", "a ", "a.", ".", "..", "a/b", "a\\b", "a:b", "a*", "a?", "a\"", "a<", "a|",
            "a\tb", "CON", "con.txt", "COM1", "lpt9.x", "NUL", &long,
        ] {
            assert!(check_project_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_suggested_name_is_the_first_free_one() {
        let parent = scratch("names");
        assert_eq!(
            next_project_name(&parent, "New Model").unwrap(),
            "New Model"
        );
        fs::create_dir(parent.join("New Model")).unwrap();
        assert_eq!(
            next_project_name(&parent, "New Model").unwrap(),
            "New Model 1"
        );
        fs::create_dir(parent.join("New Model 1")).unwrap();
        assert_eq!(
            next_project_name(&parent, "New Model").unwrap(),
            "New Model 2"
        );
        // Nothing was written by the suggestion itself.
        assert!(!parent.join("New Model 2").exists());
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn a_missing_parent_or_a_bad_name_is_refused() {
        let parent = scratch("refused");
        let missing = parent.join("not-here");
        assert_eq!(
            next_project_name(&missing, "a").unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            next_project_name(&parent, "a/b").unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn a_folder_without_revisions_is_not_a_project() {
        let parent = scratch("not-store");
        let folder = parent.join("plain");
        fs::create_dir(&folder).unwrap();
        assert!(!is_store(&folder));
        assert!(project_is_missing(&folder.join("gone")));
        assert!(project_is_missing(&folder), "no revisions/ directory");
        assert_eq!(project_info(&folder).unwrap().state, ProjectState::Missing);
        let (listed, truncated) = list_projects(&parent).unwrap();
        assert!(listed.is_empty() && !truncated);
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn an_empty_store_is_listed_as_empty() {
        let parent = scratch("empty-store");
        let store = parent.join("Hall");
        fs::create_dir_all(store.join("revisions")).unwrap();
        fs::create_dir_all(store.join("blobs")).unwrap();
        let (listed, _) = list_projects(&parent).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Hall");
        assert!(!listed[0].state.is_ready());
        let _ = fs::remove_dir_all(&parent);
    }
}
