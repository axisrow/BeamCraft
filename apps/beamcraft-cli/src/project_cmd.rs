// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Start commands: the user's settings, the environments, and creating, listing and remembering
//! projects. They call `beamcraft_host::settings` and `beamcraft_cli::project` and print one JSON
//! object with the same error shape and exit codes as the plate commands.
//!
//! The settings live in the user's configuration directory unless `--config-dir` names another.
//! `project-create` takes its author from the settings profile unless `--actor-id` names one, and
//! records the new project as recently opened; a project is created even when that record cannot
//! be written (`recent_recorded` is then false).

use crate::plate_cmd::{id_error, revision_json};
use beamcraft_cli::plate::PlateError;
use beamcraft_cli::project::{
    ENVIRONMENTS_DIR, EnvironmentEntry, EnvironmentState, ProjectContext, ProjectInfo,
    ProjectState, builtin_environment_dirs, create_project, list_environments, list_projects,
    project_info,
};
use beamcraft_host::ids::IdSource;
use beamcraft_host::revision::NoFault;
use beamcraft_host::settings::{Settings, SettingsError, default_dir};
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
        code: ErrorCode::InvalidArgument,
        message,
    }
}

fn settings_error(error: SettingsError) -> PlateError {
    PlateError {
        code: error.code,
        message: error.message,
    }
}

/// Whether `command` is one of the start commands.
pub fn handles(command: &OsString) -> bool {
    [
        "settings-show",
        "settings-set",
        "environment-list",
        "project-create",
        "project-list",
        "recent-list",
        "recent-add",
    ]
    .iter()
    .any(|c| command == c)
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
        "settings-show" | "environment-list" | "recent-list" => &["--config-dir"],
        "settings-set" => &[
            "--config-dir",
            "--name",
            "--email",
            "--environment",
            "--skip-launcher",
            "--models-dir",
        ],
        "project-create" => &[
            "--config-dir",
            "--parent",
            "--name",
            "--op-id",
            "--revision-id",
            "--actor-id",
            "--project-id",
            "--workspace-id",
        ],
        "project-list" => &["--dir"],
        "recent-add" => &["--config-dir", "--store"],
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
                code: ErrorCode::NotFound,
                message: "no configuration directory is known; name one with --config-dir",
            }),
        }
    };
    match *command {
        "settings-show" => {
            let dir = config_dir()?;
            let settings = Settings::load(&dir).map_err(settings_error)?;
            settings_json("settings", &dir, &settings)
        }
        "settings-set" => {
            let dir = config_dir()?;
            let mut settings = Settings::load(&dir).map_err(settings_error)?;
            match (given("--name"), given("--email")) {
                (Some(name), Some(email)) => settings
                    .set_profile(name, email, || {
                        fresh_id().map_err(|e| SettingsError {
                            code: e.code,
                            message: e.message,
                        })
                    })
                    .map_err(settings_error)?,
                (None, None) => {}
                _ => return Err(invalid("--name and --email are given together")),
            }
            if let Some(id) = given("--environment") {
                settings.environment = (id != "none").then(|| id.to_owned());
            }
            if let Some(flag) = given("--skip-launcher") {
                settings.skip_launcher = boolean(flag)?;
            }
            if let Some(dir) = given("--models-dir") {
                settings.models_dir = (dir != "none").then(|| PathBuf::from(dir));
            }
            settings.save(&dir).map_err(settings_error)?;
            settings_json("settings_saved", &dir, &settings)
        }
        "environment-list" => {
            let mut folders: Vec<(PathBuf, &'static str)> = builtin_environment_dirs()
                .into_iter()
                .map(|d| (d, "builtin"))
                .collect();
            let user = given("--config-dir")
                .map(PathBuf::from)
                .or_else(default_dir)
                .map(|d| d.join(ENVIRONMENTS_DIR));
            if let Some(user) = &user {
                folders.push((user.clone(), "user"));
            }
            let entries = list_environments(&folders);
            Ok(Json::object([
                ("outcome", Json::text("environments")),
                (
                    "user_dir",
                    user.as_deref().map_or(Ok(Json::Null), path_json)?,
                ),
                (
                    "environments",
                    Json::Array(entries.iter().map(environment_json).collect::<Run<_>>()?),
                ),
            ]))
        }
        "project-create" => {
            let parent = given("--parent").ok_or_else(|| invalid("--parent is required"))?;
            let name = given("--name").ok_or_else(|| invalid("--name is required"))?;
            // The settings are read only when they are needed: for the author, and to remember
            // the project. A damaged settings file never blocks a creation that names its actor.
            let dir = config_dir();
            let actor = match given("--actor-id") {
                Some(text) => Uuid7::parse(text).map_err(id_error)?,
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
                    Some(text) => OpId::parse(text).map_err(id_error)?,
                    None => OpId::from_uuid(fresh_id()?),
                },
                revision_id: match given("--revision-id") {
                    Some(text) => RevisionId::parse(text).map_err(id_error)?,
                    None => RevisionId::from_uuid(fresh_id()?),
                },
                actor,
                project: match given("--project-id") {
                    Some(text) => ProjectId::parse(text).map_err(id_error)?,
                    None => ProjectId::from_uuid(fresh_id()?),
                },
                workspace: match given("--workspace-id") {
                    Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
                    None => WorkspaceId::from_uuid(fresh_id()?),
                },
            };
            let info = create_project(Path::new(parent), name, &context, &mut NoFault)?;
            let recorded = dir.is_ok_and(|dir| remember(&dir, &info.path).is_ok());
            Ok(Json::object([
                ("outcome", Json::text("project_created")),
                ("actor_id", Json::text(actor.to_string())),
                ("project", project_json(&info)?),
                ("recent_recorded", Json::Bool(recorded)),
            ]))
        }
        "project-list" => {
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
        "recent-list" => {
            let settings = Settings::load(&config_dir()?).map_err(settings_error)?;
            let mut recent = Vec::with_capacity(settings.recent.len());
            for r in &settings.recent {
                let project = project_info(&r.path)?;
                recent.push(Json::object([
                    ("opened_ms", Json::uint(r.opened_ms)),
                    ("project", project_json(&project)?),
                ]));
            }
            Ok(Json::object([
                ("outcome", Json::text("recent")),
                ("recent", Json::Array(recent)),
            ]))
        }
        "recent-add" => {
            let store = given("--store").ok_or_else(|| invalid("--store is required"))?;
            let info = project_info(Path::new(store))?;
            if !matches!(info.state, ProjectState::Ready { .. }) {
                return Err(PlateError {
                    code: ErrorCode::NotFound,
                    message: "the folder holds no readable project",
                });
            }
            let dir = config_dir()?;
            remember(&dir, &info.path)?;
            Ok(Json::object([
                ("outcome", Json::text("recent_added")),
                ("project", project_json(&info)?),
            ]))
        }
        _ => Err(invalid("unknown command")),
    }
}

fn fresh_id() -> Run<beamcraft_model::id::Uuid7> {
    IdSource::system().next_id().map_err(|_| PlateError {
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

fn settings_json(outcome: &str, dir: &Path, settings: &Settings) -> Run<Json> {
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("config_dir", path_json(dir)?),
        ("settings", settings.to_json().map_err(settings_error)?),
    ]))
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

fn environment_json(entry: &EnvironmentEntry) -> Run<Json> {
    let path = path_json(&entry.path)?;
    let origin = Json::text(entry.origin);
    Ok(match &entry.state {
        EnvironmentState::Ready {
            id,
            title,
            shadowed,
        } => Json::object([
            ("id", Json::text(id.clone())),
            ("title", title.clone().map_or(Json::Null, Json::text)),
            ("origin", origin),
            ("path", path),
            ("shadowed", Json::Bool(*shadowed)),
            ("refused", Json::Null),
        ]),
        EnvironmentState::Refused(error) => Json::object([
            ("id", Json::Null),
            ("title", Json::Null),
            ("origin", origin),
            ("path", path),
            ("shadowed", Json::Bool(false)),
            (
                "refused",
                Json::object([
                    ("code", Json::text(error.code.as_str())),
                    ("message", Json::text(error.message)),
                ]),
            ),
        ]),
    })
}
