// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Projects as folders, and the environments a session starts with.
//!
//! A project is a store directory (`blobs/`, `revisions/`, `ledger/`). Its name is the name of
//! its folder, as people see it in a file manager. [`create_project`] makes the folder inside an
//! existing parent and commits its first revision: an empty model whose root layer records the
//! ProjectId (D1). [`list_projects`] lists the projects directly inside one folder without
//! writing anything; [`project_info`] reads one.
//!
//! Dates come from identities, not from file times: a ProjectId and a revision id are UUIDv7, so
//! the creation time is the timestamp of the ProjectId and the last change is the timestamp of the
//! head revision's id. An identity named explicitly by a caller carries whatever time it holds.
//!
//! An environment is the A07.3 environment layer (`environment.usda`) in a folder of its own.
//! [`list_environments`] reads the folders of the built-in environments shipped with the program
//! and of the user's own ones, in that order; the first environment of an id wins and a later one
//! of the same id is reported as shadowed. An environment that does not read is listed with its
//! refusal, never skipped silently.

use crate::library::parse_environment;
use crate::plate::{
    DERIVED_LAYER, PlateError, ROOT_LAYER, RevisionSummary, claim_project, host_uuid, load_base,
    summarize,
};
use crate::publication::commit_verified;
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, LayerEntry, LayerRole, RevisionDraft, RevisionStore,
};
use beamcraft_host::storage::BlobStore;
use beamcraft_layer_engine::authored::schema_identity;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, Uuid7, WorkspaceId};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Longest project folder name, in characters.
pub const MAX_NAME_CHARS: usize = 100;
/// Most directory entries [`list_projects`] or [`list_environments`] looks at in one folder.
pub const MAX_SCANNED: usize = 4_096;
/// Most projects [`list_projects`] returns.
pub const MAX_LISTED: usize = 512;
/// File name of an environment layer inside its folder.
pub const ENVIRONMENT_FILE: &str = "environment.usda";
/// Name of the folder of environments, next to the program and in the user's settings directory.
pub const ENVIRONMENTS_DIR: &str = "environments";

const fn refuse(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError { code, message }
}

const fn invalid(message: &'static str) -> PlateError {
    refuse(ErrorCode::InvalidArgument, message)
}

/// Checks a project name that becomes a folder name on Windows, Linux and macOS: 1 to
/// [`MAX_NAME_CHARS`] characters, no path separator, no `:*?"<>|`, no control character, no
/// leading or trailing space, no trailing dot, not `.` or `..` and not a reserved Windows device
/// name such as `CON` or `COM1` (with or without an extension). Letters of any script are allowed.
pub fn check_project_name(name: &str) -> Result<(), PlateError> {
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

/// Identities of one project creation.
#[derive(Debug, Clone)]
pub struct ProjectContext {
    /// The logical operation of the first revision.
    pub op_id: OpId,
    /// Identity of the first revision.
    pub revision_id: RevisionId,
    /// The author (trusted context).
    pub actor: Uuid7,
    /// The new project's identity.
    pub project: ProjectId,
    /// The workspace that authors the first revision.
    pub workspace: WorkspaceId,
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
        /// The head revision.
        revision: Box<RevisionSummary>,
    },
    /// A store folder without a revision.
    Empty,
    /// A store whose head does not read; the code says why.
    Unreadable(ErrorCode),
    /// The folder does not exist or is not a store.
    Missing,
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
pub fn is_store(dir: &Path) -> bool {
    dir.join("revisions").is_dir()
}

fn absolute(path: &Path) -> Result<PathBuf, PlateError> {
    std::path::absolute(path).map_err(|_| invalid("the path cannot be made absolute"))
}

/// Reads one project folder. The head is read without verifying every layer blob; opening the
/// project verifies it in full. Nothing is written.
pub fn project_info(dir: &Path) -> Result<ProjectInfo, PlateError> {
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
                            revision: Box::new(summarize(&info)),
                        },
                        _ => ProjectState::Unreadable(ErrorCode::IntegrityError),
                    }
                }
                Ok(None) => ProjectState::Empty,
                Err(error) => ProjectState::Unreadable(PlateError::from(error).code),
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
pub fn list_projects(dir: &Path) -> Result<(Vec<ProjectInfo>, bool), PlateError> {
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

/// Creates the folder `parent/name` and commits the first revision of a new, empty project.
///
/// The parent must exist; a folder of that name must not. When the commit fails, the folder this
/// call created is removed again, so a failed creation leaves nothing behind.
pub fn create_project(
    parent: &Path,
    name: &str,
    context: &ProjectContext,
    hook: &mut dyn CommitHook,
) -> Result<ProjectInfo, PlateError> {
    check_project_name(name)?;
    let parent = absolute(parent)?;
    if !parent.is_dir() {
        return Err(refuse(
            ErrorCode::NotFound,
            "the parent folder does not exist",
        ));
    }
    let dir = parent.join(name);
    fs::create_dir(&dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => refuse(
            ErrorCode::DuplicateId,
            "a file or folder of that name already exists",
        ),
        std::io::ErrorKind::PermissionDenied => {
            refuse(ErrorCode::PermissionDenied, "the folder cannot be created")
        }
        _ => invalid("the folder cannot be created"),
    })?;
    match first_revision(&dir, context, hook) {
        Ok(()) => project_info(&dir),
        Err(error) => {
            // Only what this call made is removed: the folder did not exist before it.
            let _ = fs::remove_dir_all(&dir);
            Err(error)
        }
    }
}

/// Commits the initial layer set of a store without a revision: the root that records the
/// ProjectId, and empty authored and derived layers. An empty model is not an edit, so it is not
/// authored through the layer engine; it is the same base every first command starts from, and
/// it is verified at commit like every revision (U03 index, structural validation).
fn first_revision(
    dir: &Path,
    context: &ProjectContext,
    hook: &mut dyn CommitHook,
) -> Result<(), PlateError> {
    let blobs = BlobStore::new(dir).map_err(|e| refuse(e.code(), "store root is not usable"))?;
    let store = RevisionStore::new(blobs);
    let _claim = claim_project(context.project);
    let (head, base) = load_base(&store, &context.project, &context.workspace)?;
    if head.is_some() {
        return Err(refuse(
            ErrorCode::DuplicateId,
            "the folder already holds a project",
        ));
    }
    let mut entries = Vec::with_capacity(base.assets.len());
    for asset in &base.assets {
        store
            .blobs()
            .publish(&asset.bytes)
            .map_err(|e| refuse(e.code(), "layer blob could not be stored"))?;
        let role = if asset.name == DERIVED_LAYER {
            LayerRole::Derived
        } else {
            LayerRole::Authored
        };
        entries.push(LayerEntry::from_bytes(&asset.name, role, &asset.bytes)?);
    }
    let draft = RevisionDraft {
        project: host_uuid(&base.project)?,
        workspace: host_uuid(&base.workspace)?,
        revision: host_uuid(&context.revision_id.to_string())?,
        op: host_uuid(&context.op_id.to_string())?,
        workspace_generation: base
            .generation
            .checked_add(1)
            .ok_or(refuse(ErrorCode::LimitExceeded, "generation exhausted"))?,
        schema: schema_identity()?.digest,
        root_layer: ROOT_LAYER.into(),
        entries,
    };
    let committed = commit_verified(&store, &draft, &base.expected, hook)?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(refuse(
            ErrorCode::IntegrityError,
            "a new project was reported as replayed",
        ));
    }
    Ok(())
}

/// One environment found in an environments folder.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentEntry {
    /// The environment file.
    pub path: PathBuf,
    /// Where it was found: `builtin` or `user`.
    pub origin: &'static str,
    /// What reading it found.
    pub state: EnvironmentState,
}

/// The state of one environment file.
#[derive(Debug, Clone, PartialEq)]
pub enum EnvironmentState {
    /// A readable environment.
    Ready {
        /// `bim:environment:id`.
        id: String,
        /// `bim:environment:title`, when declared.
        title: Option<String>,
        /// Another environment of the same id was found first and answers for it.
        shadowed: bool,
    },
    /// An environment file that does not read.
    Refused(PlateError),
}

/// The folders of the built-in environments: `environments/` next to the program, inside the
/// macOS application bundle's `Resources`, and in the source tree of a development build.
pub fn builtin_environment_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.join(ENVIRONMENTS_DIR));
        dirs.push(dir.join("../Resources").join(ENVIRONMENTS_DIR));
    }
    dirs.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(ENVIRONMENTS_DIR),
    );
    dirs
}

/// The environments of the given folders (each `(folder, origin)`), in folder order and by folder
/// name inside one folder. Missing folders are skipped; the same environment file reached twice
/// (a development build's folder is also the program's) is listed once.
pub fn list_environments(folders: &[(PathBuf, &'static str)]) -> Vec<EnvironmentEntry> {
    let mut found: Vec<EnvironmentEntry> = Vec::new();
    let mut seen_files: Vec<PathBuf> = Vec::new();
    for (folder, origin) in folders {
        let Ok(entries) = fs::read_dir(folder) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .take(MAX_SCANNED)
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path().join(ENVIRONMENT_FILE))
            .filter(|f| f.is_file())
            .collect();
        files.sort();
        for file in files {
            // The canonical path only detects the same file reached twice. It is never reported:
            // on Windows it is a `\\?\` device path, which every file command refuses.
            let canonical = fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
            if seen_files.contains(&canonical) {
                continue;
            }
            seen_files.push(canonical);
            let file = plain(&file);
            let state = match read_environment(&file) {
                Ok((id, title)) => {
                    let shadowed = found.iter().any(|e| {
                        matches!(&e.state, EnvironmentState::Ready { id: other, .. } if *other == id)
                    });
                    EnvironmentState::Ready {
                        id,
                        title,
                        shadowed,
                    }
                }
                Err(error) => EnvironmentState::Refused(error),
            };
            found.push(EnvironmentEntry {
                path: file,
                origin,
                state,
            });
        }
    }
    found
}

/// `path` made absolute with its `.` and `..` segments folded away, on the text alone: the form
/// a person and every file command accept (no `\\?\` prefix, no link resolution).
fn plain(path: &Path) -> PathBuf {
    use std::path::Component;
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn read_environment(file: &Path) -> Result<(String, Option<String>), PlateError> {
    let limit = crate::library::MAX_ENVIRONMENT_BYTES;
    let mut bytes = Vec::new();
    fs::File::open(file)
        .and_then(|f| {
            f.take(u64::try_from(limit).unwrap_or(u64::MAX) + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| {
            refuse(
                ErrorCode::NotFound,
                "the environment file could not be read",
            )
        })?;
    let info = parse_environment(bytes)?;
    Ok((info.id, info.title))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn reported_paths_are_plain() {
        let p = plain(Path::new("/a/b/../c/./d.usda"));
        // On Windows the root gains the current drive; the rest is the same everywhere.
        assert!(p.is_absolute() && p.ends_with("a/c/d.usda"), "{p:?}");
        assert!(p.components().all(|c| !matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )));
        assert!(!p.to_string_lossy().starts_with("\\\\"));
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
}
