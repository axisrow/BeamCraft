// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Start commands: `create-project` and `list-project`. They call `beamcraft_host::settings` and
//! `beamcraft_cli::project` and print one JSON object with the same error shape and exit codes as
//! the plate commands.
//!
//! The settings live in the user's configuration directory unless `--config-dir` names another.
//! `create-project` takes its author from the settings profile unless `--actor-id` names one, and
//! records the new project as recently opened; a project is created even when that record cannot
//! be written (`recent_recorded` is then false).

use crate::plate_cmd::revision_json;
use beamcraft_cli::definition::{DefineRequest, DefinitionRecord, define};
use beamcraft_cli::plate::PlateError;
use beamcraft_cli::project::{
    ENVIRONMENTS_DIR, EnvironmentState, ProjectContext, ProjectInfo, ProjectState,
    builtin_environment_dirs, create_project, create_project_unique, install_environment,
    list_environments, list_projects,
};
use beamcraft_host::ids::IdSource;
use beamcraft_host::revision::{NoFault, RevisionStore, Uuid7 as HostUuid7};
use beamcraft_host::settings::{Settings, SettingsError, default_dir};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, Uuid7, WorkspaceId};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

type Run<T> = Result<T, PlateError>;

const fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

fn settings_error(error: SettingsError) -> PlateError {
    PlateError {
        rule_id: None,
        code: error.code,
        message: error.message,
    }
}

/// Runs one start command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

fn run(args: &[OsString]) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    let own: &[&str] = match *command {
        "create-project" => &[
            "--config-dir",
            "--parent",
            "--name",
            "--unique-name",
            "--environment",
            "--op-id",
            "--revision-id",
            "--actor-id",
            "--project-id",
            "--workspace-id",
        ],
        "list-project" => &["--dir"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 32 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !own.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let given = |key: &str| options.get(key).copied();
    let config_dir = || -> Run<PathBuf> {
        match given("--config-dir") {
            Some(dir) => Ok(PathBuf::from(dir)),
            None => default_dir().ok_or(PlateError {
                rule_id: None,
                code: ErrorCode::NotFound,
                message: "no configuration directory is known; name one with --config-dir",
            }),
        }
    };
    match *command {
        "create-project" => {
            let parent = given("--parent").ok_or_else(|| invalid("--parent is required"))?;
            let name = given("--name").ok_or_else(|| invalid("--name is required"))?;
            // The settings are read only when they are needed: for the author, and to remember
            // the project. A damaged settings file never blocks a creation that names its actor.
            let dir = config_dir();
            let actor = match given("--actor-id") {
                Some(text) => Uuid7::parse(text).map_err(crate::plate_cmd::context_id_error)?,
                None => {
                    let settings = Settings::load(&dir.clone()?).map_err(settings_error)?;
                    match settings.profile {
                        Some(profile) => profile.actor,
                        None => fresh_id()?,
                    }
                }
            };
            let context = ProjectContext {
                op_id: match given("--op-id") {
                    Some(text) => OpId::parse(text).map_err(crate::plate_cmd::context_id_error)?,
                    None => OpId::from_uuid(fresh_id()?),
                },
                revision_id: match given("--revision-id") {
                    Some(text) => {
                        RevisionId::parse(text).map_err(crate::plate_cmd::context_id_error)?
                    }
                    None => RevisionId::from_uuid(fresh_id()?),
                },
                actor,
                project: match given("--project-id") {
                    Some(text) => {
                        ProjectId::parse(text).map_err(crate::plate_cmd::context_id_error)?
                    }
                    None => ProjectId::from_uuid(fresh_id()?),
                },
                workspace: match given("--workspace-id") {
                    Some(text) => {
                        WorkspaceId::parse(text).map_err(crate::plate_cmd::context_id_error)?
                    }
                    None => WorkspaceId::from_uuid(fresh_id()?),
                },
            };
            let unique = given("--unique-name")
                .map(boolean)
                .transpose()?
                .unwrap_or(false);
            let create = if unique {
                create_project_unique
            } else {
                create_project
            };
            let info = create(Path::new(parent), name, &context, &mut NoFault)?;
            let seeded = match given("--environment") {
                Some(id) => {
                    let config = given("--config-dir").map(str::to_owned);
                    match seed_project(&info.path, id, config.as_deref(), &context) {
                        Ok(seeded) => Some(seeded),
                        Err(error) => {
                            // The folder was made by this call, so a failed seeding removes it
                            // again and leaves nothing behind.
                            let _ = std::fs::remove_dir_all(&info.path);
                            return Err(error);
                        }
                    }
                }
                None => None,
            };
            let recorded = dir.is_ok_and(|dir| remember(&dir, &info.path).is_ok());
            let mut reply = Json::object([
                ("outcome", Json::text("project_created")),
                ("actor_id", Json::text(actor.to_string())),
                ("project", project_json(&info)?),
                ("recent_recorded", Json::Bool(recorded)),
            ]);
            if let (Some(seeded), Json::Object(fields)) = (seeded, &mut reply) {
                fields.insert(
                    "seeded".into(),
                    Json::object([
                        ("environment", Json::text(seeded.environment)),
                        ("materials", Json::uint(seeded.materials)),
                    ]),
                );
            }
            Ok(reply)
        }
        "list-project" => {
            let dir = given("--dir").ok_or_else(|| invalid("--dir is required"))?;
            let (projects, truncated) = list_projects(Path::new(dir))?;
            Ok(Json::object([
                ("outcome", Json::text("projects")),
                (
                    "dir",
                    path_json(&std::path::absolute(dir).map_err(|_| invalid("--dir"))?)?,
                ),
                ("truncated", Json::Bool(truncated)),
                (
                    "projects",
                    Json::Array(projects.iter().map(project_json).collect::<Run<_>>()?),
                ),
            ]))
        }
        _ => Err(invalid("unknown command")),
    }
}

fn fresh_id() -> Run<beamcraft_model::id::Uuid7> {
    IdSource::system().next_id().map_err(|_| PlateError {
        rule_id: None,
        code: ErrorCode::BackendUnavailable,
        message: "no identity could be generated",
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn remember(dir: &Path, project: &Path) -> Run<()> {
    let mut settings = Settings::load(dir).map_err(settings_error)?;
    settings
        .touch_recent(project.to_path_buf(), now_ms())
        .map_err(settings_error)?;
    settings.save(dir).map_err(settings_error)
}

fn boolean(text: &str) -> Run<bool> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid("a flag is true or false")),
    }
}

fn path_json(path: &Path) -> Run<Json> {
    path.to_str()
        .map(Json::text)
        .ok_or_else(|| invalid("a path is not UTF-8"))
}

fn project_json(info: &ProjectInfo) -> Run<Json> {
    let (state, project_id, created, modified, revision) = match &info.state {
        ProjectState::Ready {
            project_id,
            created_ms,
            modified_ms,
            revision,
        } => (
            Json::text("ready"),
            Json::text(project_id.clone()),
            Json::uint(*created_ms),
            Json::uint(*modified_ms),
            revision_json(revision),
        ),
        ProjectState::Empty => (
            Json::text("empty"),
            Json::Null,
            Json::Null,
            Json::Null,
            Json::Null,
        ),
        ProjectState::Missing => (
            Json::text("missing"),
            Json::Null,
            Json::Null,
            Json::Null,
            Json::Null,
        ),
        ProjectState::Unreadable(code) => (
            Json::object([("unreadable", Json::text(code.as_str()))]),
            Json::Null,
            Json::Null,
            Json::Null,
            Json::Null,
        ),
    };
    Ok(Json::object([
        ("name", Json::text(info.name.clone())),
        ("path", path_json(&info.path)?),
        ("state", state),
        ("project_id", project_id),
        ("created_ms", created),
        ("modified_ms", modified),
        ("revision", revision),
    ]))
}

/// The materials every seeded project starts with: a name and a density in kg/m3. A density of
/// zero is the massless material of auxiliary, non-steel parts; it is a known zero mass, not an
/// unknown one.
const DEFAULT_MATERIALS: [(&str, f64); 2] = [("Steel_Undefined", 0.0), ("Steel", 7850.0)];

/// What `create-project --environment` put into the new project.
struct Seeded {
    environment: String,
    materials: u64,
}

/// The folders `list-environment` reads (built-in first, then the user's) and the user's folder.
fn environment_folders(
    config_dir: Option<&str>,
) -> (Vec<(PathBuf, &'static str)>, Option<PathBuf>) {
    let mut folders: Vec<(PathBuf, &'static str)> = builtin_environment_dirs()
        .into_iter()
        .map(|d| (d, "builtin"))
        .collect();
    let user = config_dir
        .map(PathBuf::from)
        .or_else(default_dir)
        .map(|d| d.join(ENVIRONMENTS_DIR));
    if let Some(user) = &user {
        folders.push((user.clone(), "user"));
    }
    (folders, user)
}

/// Gives the new project at `dir` its environment and its default materials: the environment `id`
/// is copied into the project folder (`environment/`), so later commands resolve profile names
/// against it without any option, and the two default materials are published, one revision each.
/// Profiles are not copied: a profile is defined in the model when it is first used. An environment
/// that is not found or does not read is refused.
fn seed_project(
    dir: &Path,
    id: &str,
    config_dir: Option<&str>,
    context: &ProjectContext,
) -> Run<Seeded> {
    let (folders, _) = environment_folders(config_dir);
    let file = list_environments(&folders)
        .into_iter()
        .find_map(|entry| match entry.state {
            EnvironmentState::Ready {
                id: ready,
                shadowed: false,
                ..
            } if ready == id => Some(entry.path),
            _ => None,
        })
        .ok_or(PlateError {
            rule_id: None,
            code: ErrorCode::NotFound,
            message: "the environment is not among the built-in or user environments",
        })?;
    let file = file
        .to_str()
        .ok_or(invalid("the environment path is not UTF-8"))?;
    // Read and admitted first; a refused environment copies nothing.
    install_environment(dir, Path::new(file))?;
    let blobs = BlobStore::new(dir).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let define_one = |definition: DefinitionRecord| -> Run<()> {
        define(
            &store,
            &DefineRequest {
                revision_id: RevisionId::from_uuid(fresh_id()?),
                op_id: OpId::from_uuid(fresh_id()?),
                actor: HostUuid7::parse(&context.actor.to_string()).map_err(PlateError::from)?,
                project: context.project,
                workspace: context.workspace,
                definition,
            },
            &mut NoFault,
        )
        .map(|_| ())
    };
    let mut seeded = Seeded {
        environment: id.to_owned(),
        materials: 0,
    };
    for (name, density) in DEFAULT_MATERIALS {
        define_one(
            DefinitionRecord::Material {
                density_kg_m3: Some(density),
                grade: None,
                standard: None,
                display_name: None,
            }
            .named(Some(name)),
        )?;
        seeded.materials += 1;
    }
    Ok(seeded)
}
