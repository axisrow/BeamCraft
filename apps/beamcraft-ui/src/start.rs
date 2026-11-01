// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The start window and the start page, shown when the window is opened without a model.
//!
//! The start window asks for the environment and for the author: a name and an e-mail address,
//! saved in the user's settings. The author identity those settings hold is recorded as the actor
//! of every edit made in the window. "Don't show at start" skips the start window next time.
//!
//! The start page lists the recently opened projects, the projects of a folder, and creates a new
//! project from a template. Shared (cloud) projects are not available yet; the tab says so.
//!
//! The settings, the environments, the recent list, the projects of a folder and the next free
//! project name come from the `session` crate, called in this process on a worker thread. Creating
//! a project is a command: it runs `create-project` in a separate process, and its reply is read
//! with `crate::tools::rejection` so a refusal is shown with the CLI's code and message.

use crate::command;
use crate::stats::Stats;
use crate::theme::{self, Icon, Kind, Palette};
use crate::tools::{ToolError, decode, rejection, text};
use beamcraft_host::ids::IdSource;
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::Uuid7;
use session::Refusal;
use session::environment::{
    EnvironmentEntry, EnvironmentState, environment_folders, list_environments,
};
use session::project::{ProjectInfo, ProjectState, list_projects, next_project_name};
use session::recent::{RecentError, recent_projects, remember_project};
use session::settings::{self, Settings, SettingsError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

/// How long one start command may run.
const DEADLINE: Duration = Duration::from_secs(60);
/// Longest text in a field.
const MAX_FIELD: usize = 1_024;
/// Initial editable model name; translate with the UI when other languages are added.
const DEFAULT_MODEL_NAME: &str = "New Model";

/// The author as the settings hold it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Profile {
    /// The author identity (UUIDv7).
    pub actor_id: String,
    /// Display name.
    pub name: String,
    /// E-mail address.
    pub email: String,
}

/// The user's settings as the page shows them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsView {
    /// Appearance, with the system used for older settings replies.
    pub theme: egui::ThemePreference,
    /// The configuration directory the CLI used.
    pub config_dir: String,
    /// The author, once entered.
    pub profile: Option<Profile>,
    /// The chosen environment id.
    pub environment: Option<String>,
    /// Skip the start window.
    pub skip_launcher: bool,
    /// The folder listed under "All models".
    pub models_dir: Option<String>,
}

/// One environment as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentView {
    /// Its id, `None` when it does not read.
    pub id: Option<String>,
    /// Its title, when declared.
    pub title: Option<String>,
    /// `builtin` or `user`.
    pub origin: String,
    /// Another environment of the same id answers for it.
    pub shadowed: bool,
    /// Why it does not read.
    pub refused: Option<String>,
}

impl EnvironmentView {
    /// The text shown for it in the list.
    pub fn label(&self) -> String {
        match (&self.id, &self.title) {
            (Some(id), Some(title)) if title != id => format!("{title} ({id})"),
            (Some(id), _) => id.clone(),
            (None, _) => "unreadable environment".to_owned(),
        }
    }

    fn usable_id(&self) -> Option<&str> {
        self.id.as_deref().filter(|_| !self.shadowed)
    }
}

/// One project folder as the page lists it, or as `create-project` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectView {
    /// The folder name.
    pub name: String,
    /// The store directory.
    pub path: String,
    /// `ready`, `empty`, `missing` or the code of an unreadable store.
    pub state: String,
    /// Creation time, Unix milliseconds.
    pub created_ms: Option<u64>,
    /// Time of the last change, Unix milliseconds.
    pub modified_ms: Option<u64>,
}

impl ProjectView {
    /// Whether the project can be opened.
    pub fn ready(&self) -> bool {
        self.state == "ready"
    }
}

fn unreadable() -> ToolError {
    ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field")
}

fn reply(exit_code: Option<i32>, stdout: &[u8], outcome: &str) -> Result<Json, ToolError> {
    let reply = match decode(stdout) {
        Ok(reply) => reply,
        Err(error) => {
            return Err(match exit_code {
                Some(0) => error,
                _ => ToolError::local("CLI_FAILED", "the CLI failed without a JSON reply"),
            });
        }
    };
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    if text(&reply, "outcome").as_deref() != Some(outcome) {
        return Err(ToolError::local("UNREADABLE_REPLY", "unexpected outcome"));
    }
    Ok(reply)
}

fn optional_uint(json: &Json, key: &str) -> Option<u64> {
    json.get(key).ok().and_then(|v| v.unsigned().ok())
}

/// A typed failure of the platform crate as the page shows it: the stable code and its message.
fn failure(code: ErrorCode, message: &str) -> ToolError {
    ToolError::local(code.as_str(), message)
}

fn settings_failure(error: SettingsError) -> ToolError {
    failure(error.code, error.message)
}

fn refusal(error: Refusal) -> ToolError {
    failure(error.code, error.message)
}

fn recent_failure(error: RecentError) -> ToolError {
    failure(error.code(), error.message())
}

/// The configuration directory: the one named, else the current user's.
fn resolve_config_dir(config_dir: Option<&Path>) -> Result<PathBuf, ToolError> {
    config_dir
        .map(Path::to_path_buf)
        .or_else(settings::default_dir)
        .ok_or_else(|| failure(ErrorCode::NotFound, "no configuration directory is known"))
}

fn fresh_actor() -> Result<Uuid7, SettingsError> {
    IdSource::system().next_id().map_err(|_| SettingsError {
        code: ErrorCode::BackendUnavailable,
        message: "no identity could be generated",
    })
}

fn settings_view(dir: &Path, settings: &Settings) -> SettingsView {
    SettingsView {
        theme: crate::appearance::egui_theme(settings.theme),
        config_dir: dir.to_string_lossy().into_owned(),
        profile: settings.profile.as_ref().map(|p| Profile {
            actor_id: p.actor.to_string(),
            name: p.name.clone(),
            email: p.email.clone(),
        }),
        environment: settings.environment.clone(),
        skip_launcher: settings.skip_launcher,
        models_dir: settings
            .models_dir
            .as_ref()
            .map(|d| d.to_string_lossy().into_owned()),
    }
}

fn environment_views(entries: &[EnvironmentEntry]) -> Vec<EnvironmentView> {
    entries
        .iter()
        .map(|entry| match &entry.state {
            EnvironmentState::Ready {
                id,
                title,
                shadowed,
            } => EnvironmentView {
                id: Some(id.clone()),
                title: title.clone(),
                origin: entry.origin.to_owned(),
                shadowed: *shadowed,
                refused: None,
            },
            EnvironmentState::Refused(why) => EnvironmentView {
                id: None,
                title: None,
                origin: entry.origin.to_owned(),
                shadowed: false,
                refused: Some(format!("{}: {}", why.code.as_str(), why.message)),
            },
        })
        .collect()
}

fn project_view(info: &ProjectInfo) -> ProjectView {
    let (state, created_ms, modified_ms) = match &info.state {
        ProjectState::Ready {
            created_ms,
            modified_ms,
            ..
        } => ("ready".to_owned(), Some(*created_ms), Some(*modified_ms)),
        ProjectState::Empty => ("empty".to_owned(), None, None),
        ProjectState::Missing => ("missing".to_owned(), None, None),
        ProjectState::Unreadable(code) => (code.as_str().to_owned(), None, None),
    };
    ProjectView {
        name: info.name.clone(),
        path: info.path.to_string_lossy().into_owned(),
        state,
        created_ms,
        modified_ms,
    }
}

fn recent_views(dir: &Path) -> Result<Vec<ProjectView>, ToolError> {
    let recent = recent_projects(dir).map_err(recent_failure)?;
    Ok(recent.iter().map(|r| project_view(&r.project)).collect())
}

/// Everything the page shows at start: the settings, the environments and the recent projects.
fn load_state(
    config_dir: Option<&Path>,
) -> Result<(SettingsView, Vec<EnvironmentView>, Vec<ProjectView>), ToolError> {
    let dir = resolve_config_dir(config_dir)?;
    let settings = Settings::load(&dir).map_err(settings_failure)?;
    let (folders, _) = environment_folders(Some(&dir));
    let environments = environment_views(&list_environments(&folders));
    Ok((
        settings_view(&dir, &settings),
        environments,
        recent_views(&dir)?,
    ))
}

/// Saves the start window's choices and returns the settings and the recent projects.
fn save_choices(
    config_dir: Option<&Path>,
    name: &str,
    email: &str,
    environment: &str,
    skip_launcher: bool,
) -> Result<(SettingsView, Vec<ProjectView>), ToolError> {
    let dir = resolve_config_dir(config_dir)?;
    let settings = Settings::update(&dir, |settings| {
        settings.set_profile(name, email, fresh_actor)?;
        settings.environment = Some(environment.to_owned());
        settings.skip_launcher = skip_launcher;
        Ok(true)
    })
    .map_err(settings_failure)?;
    Ok((settings_view(&dir, &settings), recent_views(&dir)?))
}

/// Remembers `folder` as the models folder, then lists the projects in it. The folder is saved
/// first and a failure to save it does not stop the listing.
fn list_folder_projects(
    config_dir: Option<&Path>,
    folder: &str,
) -> Result<Vec<ProjectView>, ToolError> {
    if let Ok(dir) = resolve_config_dir(config_dir) {
        let _ = Settings::update(&dir, |settings| {
            settings.models_dir = Some(PathBuf::from(folder));
            Ok(true)
        });
    }
    let (projects, _truncated) = list_projects(Path::new(folder)).map_err(refusal)?;
    Ok(projects.iter().map(project_view).collect())
}

/// Remembers the project at `path` as opened now and returns it.
fn remember(config_dir: Option<&Path>, path: &str) -> Result<ProjectView, ToolError> {
    let dir = resolve_config_dir(config_dir)?;
    let info = remember_project(&dir, Path::new(path), now_ms()).map_err(recent_failure)?;
    Ok(project_view(&info))
}

fn project_of(json: &Json) -> Result<ProjectView, ToolError> {
    let state = match json.get("state").map_err(|_| unreadable())? {
        Json::String(state) => state.clone(),
        other => text(other, "unreadable").ok_or_else(unreadable)?,
    };
    Ok(ProjectView {
        name: text(json, "name").ok_or_else(unreadable)?,
        path: text(json, "path").ok_or_else(unreadable)?,
        state,
        created_ms: optional_uint(json, "created_ms"),
        modified_ms: optional_uint(json, "modified_ms"),
    })
}

/// Reads the reply of `create-project`: the project.
pub fn parse_project(
    exit_code: Option<i32>,
    stdout: &[u8],
    outcome: &str,
) -> Result<ProjectView, ToolError> {
    let reply = reply(exit_code, stdout, outcome).map_err(|mut error| {
        if outcome == "project_created" && error.code == "DUPLICATE_ID" {
            error.message = "A model or folder with this name already exists in this location. Choose a different name or location.".to_owned();
        }
        error
    })?;
    project_of(reply.get("project").map_err(|_| unreadable())?)
}

fn with_config(mut args: Vec<String>, config_dir: Option<&Path>) -> Vec<String> {
    if let Some(dir) = config_dir {
        args.push("--config-dir".into());
        args.push(dir.to_string_lossy().into_owned());
    }
    args
}

/// The `create-project` arguments of the Create tab.
pub fn project_create_args(
    config_dir: Option<&Path>,
    parent: &str,
    name: &str,
    actor_id: Option<&str>,
    environment: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "create-project".to_owned(),
        "--parent".into(),
        parent.to_owned(),
        "--name".into(),
        name.to_owned(),
    ];
    if name == DEFAULT_MODEL_NAME {
        args.extend(["--unique-name".into(), "true".into()]);
    }
    if let Some(environment) = environment {
        // The new model is seeded with the environment's profiles and the default materials.
        args.push("--environment".into());
        args.push(environment.to_owned());
    }
    if let Some(actor) = actor_id {
        args.push("--actor-id".into());
        args.push(actor.to_owned());
    }
    with_config(args, config_dir)
}

/// A Unix time in milliseconds as `YYYY-MM-DD HH:MM` (UTC).
pub fn format_time(ms: u64) -> String {
    let secs = ms / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rem / 3_600,
        (rem % 3_600) / 60
    )
}

/// The tabs of the start page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Recently opened projects.
    Recent,
    /// The projects of a folder.
    All,
    /// Shared projects (not available yet).
    Shared,
    /// A new project.
    Create,
}

enum Reply {
    Loaded(Result<(SettingsView, Vec<EnvironmentView>, Vec<ProjectView>), ToolError>),
    Saved(Result<(SettingsView, Vec<ProjectView>), ToolError>),
    Listed(Result<Vec<ProjectView>, ToolError>),
    Opened(Result<ProjectView, ToolError>),
    Suggested {
        parent: String,
        previous: String,
        name: Result<String, ToolError>,
    },
}

struct PreviewJob {
    path: String,
    reply: Receiver<Result<crate::preview::ModelPreview, String>>,
    discard: bool,
}

/// The start window and the start page.
pub struct StartPage {
    previews: std::collections::BTreeMap<String, Result<crate::preview::ModelPreview, String>>,
    preview_job: Option<PreviewJob>,
    cli: PathBuf,
    config_dir: Option<PathBuf>,
    stats: Arc<Stats>,
    job: Option<Receiver<Reply>>,
    launcher: bool,
    loaded: bool,
    settings: SettingsView,
    environments: Vec<EnvironmentView>,
    environment: Option<String>,
    name: String,
    email: String,
    skip: bool,
    tab: Tab,
    recent: Vec<ProjectView>,
    all: Vec<ProjectView>,
    folder: String,
    new_name: String,
    suggested_name: String,
    new_parent: String,
    note: Option<Result<String, String>>,
    open: Option<PathBuf>,
    search: String,
    selected: Option<usize>,
    logo: Option<egui::TextureHandle>,
    logo_tried: bool,
    small_window: Option<bool>,
}

impl StartPage {
    /// The start page of the CLI `cli`; `config_dir` replaces the user's configuration directory.
    pub fn new(cli: PathBuf, config_dir: Option<PathBuf>, stats: Arc<Stats>) -> Self {
        let mut page = Self {
            previews: std::collections::BTreeMap::new(),
            preview_job: None,
            cli,
            config_dir,
            stats,
            job: None,
            launcher: true,
            loaded: false,
            settings: SettingsView::default(),
            environments: Vec::new(),
            environment: None,
            name: String::new(),
            email: String::new(),
            skip: false,
            tab: Tab::Recent,
            recent: Vec::new(),
            all: Vec::new(),
            folder: String::new(),
            new_name: DEFAULT_MODEL_NAME.to_owned(),
            suggested_name: DEFAULT_MODEL_NAME.to_owned(),
            new_parent: String::new(),
            note: None,
            open: None,
            search: String::new(),
            selected: None,
            logo: None,
            logo_tried: false,
            small_window: None,
        };
        page.load();
        page
    }

    /// The author identity of the saved profile, recorded as the actor of the window's edits.
    pub fn actor_id(&self) -> Option<String> {
        self.settings.profile.as_ref().map(|p| p.actor_id.clone())
    }

    /// The author's name of the saved profile.
    pub fn author_name(&self) -> Option<String> {
        self.settings.profile.as_ref().map(|p| p.name.clone())
    }

    /// The configuration directory the window was started with, if any.
    pub fn config_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref()
    }

    /// The chosen environment id.
    pub fn environment(&self) -> Option<&str> {
        self.settings.environment.as_deref()
    }

    /// Runs a command of the CLI (a separate process) on a worker thread.
    fn spawn(&mut self, work: impl FnOnce(&Path, &Stats) -> Reply + Send + 'static) {
        let cli = self.cli.clone();
        let stats = Arc::clone(&self.stats);
        self.run_worker(move || work(&cli, &stats));
    }

    /// Runs `work` on a worker thread; the page polls for its reply.
    fn run_worker(&mut self, work: impl FnOnce() -> Reply + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-start".into())
            .spawn(move || {
                let _ = tx.send(work());
            });
        if spawned.is_ok() {
            self.job = Some(rx);
            self.note = None;
        } else {
            self.note = Some(Err("the command thread could not be started".into()));
        }
    }

    fn load(&mut self) {
        let config = self.config_dir.clone();
        self.run_worker(move || Reply::Loaded(load_state(config.as_deref())));
    }

    fn save_launcher(&mut self) {
        let Some(environment) = self.environment.clone() else {
            self.note = Some(Err("choose an environment".into()));
            return;
        };
        let config = self.config_dir.clone();
        let name = self.name.trim().to_owned();
        let email = self.email.trim().to_owned();
        let skip = self.skip;
        self.run_worker(move || {
            Reply::Saved(save_choices(
                config.as_deref(),
                &name,
                &email,
                &environment,
                skip,
            ))
        });
    }

    fn list_folder(&mut self) {
        let folder = self.folder.trim().to_owned();
        if folder.is_empty() {
            self.note = Some(Err("enter a folder".into()));
            return;
        }
        // The folder is remembered for the next start; the list does not wait for that.
        let config = self.config_dir.clone();
        self.run_worker(move || Reply::Listed(list_folder_projects(config.as_deref(), &folder)));
    }

    fn open_project(&mut self, path: String) {
        let config = self.config_dir.clone();
        self.run_worker(move || Reply::Opened(remember(config.as_deref(), &path)));
    }

    /// Shows the system's folder dialog, starting at the folder typed so far; the chosen folder,
    /// or `None` when it was cancelled or could not be shown (the reason is the page's note).
    fn browse(&mut self, title: &str) -> Option<String> {
        let start = match self.tab {
            Tab::All => self.folder.trim(),
            _ => self.new_parent.trim(),
        };
        match crate::dialog::pick_folder(title, Some(Path::new(start))) {
            Ok(Some(dir)) => match dir.to_str() {
                Some(text) => Some(text.to_owned()),
                None => {
                    self.note = Some(Err("the chosen folder's path is not UTF-8 text".into()));
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                self.note = Some(Err(e.to_string()));
                None
            }
        }
    }

    fn suggest_project_name(&mut self) {
        if self.job.is_some()
            || self.new_name != self.suggested_name
            || self.new_parent.trim().is_empty()
        {
            return;
        }
        let parent = self.new_parent.trim().to_owned();
        let previous = self.new_name.clone();
        self.run_worker(move || {
            let name = next_project_name(Path::new(&parent), DEFAULT_MODEL_NAME).map_err(refusal);
            Reply::Suggested {
                parent,
                previous,
                name,
            }
        });
    }

    fn create_project(&mut self) {
        // A typed location is checked here, so the refusal names the path that was typed.
        let parent = self.new_parent.trim();
        if !Path::new(parent).is_dir() {
            self.note = Some(Err(format!(
                "The folder \"{parent}\" does not exist. Choose an existing folder with Browse…"
            )));
            return;
        }

        let args = project_create_args(
            self.config_dir.as_deref(),
            self.new_parent.trim(),
            if self.new_name == self.suggested_name {
                DEFAULT_MODEL_NAME
            } else {
                self.new_name.trim()
            },
            self.actor_id().as_deref(),
            self.environment.as_deref(),
        );
        self.spawn(move |cli, stats| {
            Reply::Opened(
                command::run(cli, &args, DEADLINE, stats)
                    .and_then(|out| parse_project(out.code, &out.stdout, "project_created")),
            )
        });
    }

    pub(crate) fn show_create(&mut self) {
        self.note = None;
        self.tab = Tab::Create;
        self.selected = None;
        self.suggest_project_name();
    }

    fn adopt(&mut self, settings: SettingsView) {
        if let Some(profile) = &settings.profile {
            self.name.clone_from(&profile.name);
            self.email.clone_from(&profile.email);
        }
        self.skip = settings.skip_launcher;
        if self.folder.is_empty() {
            self.folder = settings.models_dir.clone().unwrap_or_default();
        }
        if self.new_parent.is_empty() {
            self.new_parent = settings.models_dir.clone().unwrap_or_default();
        }
        self.settings = settings;
    }

    fn poll(&mut self) {
        let Some(rx) = &self.job else { return };
        let reply = match rx.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                self.note = Some(Err("the command thread ended without an answer".into()));
                return;
            }
        };
        self.job = None;
        match reply {
            Reply::Suggested {
                parent,
                previous,
                name,
            } => {
                if self.new_parent.trim() == parent
                    && self.new_name == previous
                    && let Ok(name) = name
                {
                    self.new_name = name.clone();
                    self.suggested_name = name;
                }
            }
            Reply::Loaded(Ok((settings, environments, recent))) => {
                self.loaded = true;
                self.environments = environments;
                let usable = |id: &str| self.environments.iter().any(|e| e.usable_id() == Some(id));
                let saved = settings.environment.clone().filter(|id| usable(id));
                self.environment = saved.clone().or_else(|| {
                    // The starter environment has the sections a first beam needs; the blank
                    // one is empty, so it is only the fallback.
                    let usable = |wanted: Option<&str>| {
                        self.environments.iter().find_map(|e| {
                            e.usable_id()
                                .filter(|id| wanted.is_none_or(|w| w == *id))
                                .map(str::to_owned)
                        })
                    };
                    usable(Some("starter")).or_else(|| usable(None))
                });
                self.launcher =
                    !(settings.skip_launcher && settings.profile.is_some() && saved.is_some());
                self.recent = recent;
                self.adopt(settings);
            }
            Reply::Saved(Ok((settings, recent))) => {
                self.recent = recent;
                self.adopt(settings);
                self.launcher = false;
            }
            Reply::Listed(Ok(projects)) => {
                self.note = Some(Ok(format!("{} project(s)", projects.len())));
                self.all = projects;
            }
            Reply::Opened(Ok(project)) => {
                self.open = Some(PathBuf::from(&project.path));
                self.recent.retain(|recent| recent.path != project.path);
                self.recent.insert(0, project);
                self.recent.truncate(20);
            }
            Reply::Loaded(Err(e)) | Reply::Saved(Err(e)) => self.note = Some(Err(e.to_string())),
            Reply::Listed(Err(e)) | Reply::Opened(Err(e)) => {
                self.note = Some(Err(e.to_string()));
            }
        }
    }

    /// Draws the page and returns the store to open, once one was chosen or created.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        self.poll();
        self.poll_preview(ui.ctx());
        let busy = self.job.is_some();
        if busy {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if self.logo.is_none() && !self.logo_tried {
            self.logo_tried = true;
            self.logo = theme::load_logo(ui.ctx(), 192);
        }
        self.fit_window(ui.ctx());
        let palette = Palette::current(ui);
        if !self.loaded || self.launcher {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(palette.surface)
                        .inner_margin(egui::Margin::symmetric(20, 12)),
                )
                .show(ui, |ui| {
                    if self.loaded {
                        self.launcher_ui(ui, busy);
                        // The start window is as tall as its content: no empty band below the
                        // buttons, and room for a note when one appears.
                        // The panel's own rect fills the window; the cursor is where the
                        // content ends.
                        let needed = (ui.cursor().top() + 12.0).ceil();
                        let current = ui.ctx().content_rect().height();
                        if (needed - current).abs() > 2.0 && needed > 200.0 {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(
                                egui::vec2(LAUNCHER_SIZE.x, needed),
                            ));
                        }
                    } else {
                        self.loading_ui(ui, busy);
                    }
                });
        } else {
            self.home_ui(ui, busy);
        }
        self.open.take()
    }

    /// Returns to the model library without asking for the author again.
    pub(crate) fn return_to_models(&mut self) {
        self.previews.clear();
        if let Some(job) = &mut self.preview_job {
            job.discard = true;
        }
        self.tab = Tab::Recent;
        self.search.clear();
        self.selected = None;
        self.small_window = None;
    }

    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub(crate) fn resizable(&self) -> bool {
        self.loaded && !self.launcher
    }

    fn poll_preview(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.preview_job else { return };
        let result = match job.reply.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            Err(TryRecvError::Disconnected) => {
                Err("The model preview worker ended without an answer".into())
            }
        };
        if self.previews.len() >= 20 {
            self.previews.pop_first();
        }
        if !job.discard {
            self.previews.insert(job.path.clone(), result);
        }
        self.preview_job = None;
    }

    fn request_preview(&mut self, path: &str, ctx: &egui::Context) {
        if self.previews.contains_key(path) || self.preview_job.is_some() {
            return;
        }
        let cli = self.cli.clone();
        let store = PathBuf::from(path);
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = std::sync::mpsc::channel();
        match std::thread::Builder::new()
            .name("model-library-preview".into())
            .spawn(move || {
                let result = crate::source::load(&cli, &store, &stats)
                    .and_then(|snapshot| crate::preview::ModelPreview::from_snapshot(&snapshot))
                    .map_err(|error| error.to_string());
                let _ = tx.send(result);
            }) {
            Ok(_) => {
                self.preview_job = Some(PreviewJob {
                    path: path.to_owned(),
                    reply: rx,
                    discard: false,
                })
            }
            Err(error) => {
                self.previews.insert(
                    path.to_owned(),
                    Err(format!("Model preview worker could not start: {error}")),
                );
            }
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    /// Sizes the window for what it shows: the start window is a small fixed window, the start
    /// page a large resizable one; each change is sent once and centred on the monitor.
    fn fit_window(&mut self, ctx: &egui::Context) {
        let small = !self.loaded || self.launcher;
        if self.small_window == Some(small) {
            return;
        }
        self.small_window = Some(small);
        let size = if small {
            LAUNCHER_SIZE
        } else {
            egui::vec2(1280.0, 800.0)
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(!small));
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(if small {
            egui::vec2(LAUNCHER_SIZE.x, 400.0)
        } else {
            egui::vec2(1100.0, 680.0)
        }));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
            let at = ((monitor - size) / 2.0).max(egui::Vec2::ZERO);
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(at.to_pos2()));
        }
    }

    fn note_ui(&self, ui: &mut egui::Ui) {
        let palette = Palette::current(ui);
        match &self.note {
            Some(Ok(note)) => {
                ui.label(egui::RichText::new(note).color(palette.muted));
            }
            Some(Err(error)) => {
                ui.label(egui::RichText::new(error).color(palette.error).strong());
            }
            None => {}
        }
    }

    fn loading_ui(&mut self, ui: &mut egui::Ui, busy: bool) {
        ui.vertical_centered(|ui| {
            ui.add_space(160.0);
            theme::logo(ui, self.logo.as_ref(), 64.0);
            ui.add_space(12.0);
            ui.label(theme::bold("BeamCraft", 26.0));
            ui.add_space(12.0);
            if busy {
                ui.spinner();
            } else if theme::button(ui, Kind::Primary, "Retry", true).clicked() {
                self.load();
            }
            self.note_ui(ui);
        });
    }

    /// The start window: the environment and the author.
    fn launcher_ui(&mut self, ui: &mut egui::Ui, busy: bool) {
        let palette = Palette::current(ui);
        ui.vertical_centered(|ui| {
            theme::logo(ui, self.logo.as_ref(), 64.0);
            ui.add_space(6.0);
            ui.label(theme::bold("Welcome to BeamCraft", 26.0));
            ui.label(
                egui::RichText::new(
                    "Pick the settings to work with and tell us who you are. You can change both later.",
                )
                .color(palette.muted),
            );
        });
        ui.add_space(14.0);
        ui.label(theme::bold("Environment", 14.0));
        let selected = self
            .environments
            .iter()
            .find(|e| e.usable_id().is_some() && e.usable_id() == self.environment.as_deref())
            .map_or_else(|| "choose…".to_owned(), EnvironmentView::label);
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::Block, 26.0, palette.line);
            egui::ComboBox::from_id_salt("start-environment")
                .width(ui.available_width())
                .height(260.0)
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for env in &self.environments {
                        if let Some(id) = env.usable_id() {
                            ui.selectable_value(
                                &mut self.environment,
                                Some(id.to_owned()),
                                env.label(),
                            );
                        }
                    }
                });
        });
        ui.label(
            egui::RichText::new(
                "Sections, materials and defaults. Blank is empty: a base for your own.",
            )
            .small()
            .color(palette.muted),
        );
        for env in &self.environments {
            if let Some(reason) = &env.refused {
                ui.label(
                    egui::RichText::new(format!("An environment was not loaded ({reason})."))
                        .small()
                        .color(palette.error),
                );
            }
        }
        if self.environments.iter().all(|e| e.usable_id().is_none()) {
            ui.label(
                egui::RichText::new("No environment was found: reinstall BeamCraft or add one.")
                    .color(palette.error),
            );
        }
        ui.add_space(6.0);
        ui.columns(2, |cols| {
            if let [left, right] = cols {
                left.label(theme::bold("Your name", 14.0));
                field(left, &mut self.name, 128);
                right.label(theme::bold("E-mail", 14.0));
                field(right, &mut self.email, 254);
            }
        });
        ui.label(
            egui::RichText::new(
                "Shown next to the parts you create or change, so your team knows who edited what. Not a login.",
            )
            .small()
            .color(palette.muted),
        );
        ui.add_space(4.0);
        ui.checkbox(&mut self.skip, "Don't ask again at start");
        ui.add_space(8.0);
        let row = egui::vec2(ui.available_width(), 36.0);
        ui.allocate_ui_with_layout(
            row,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                let ready = self.environment.is_some()
                    && !self.name.trim().is_empty()
                    && !self.email.trim().is_empty();
                if theme::button(ui, Kind::Primary, "Continue", ready && !busy).clicked() {
                    self.save_launcher();
                }
                if theme::button(ui, Kind::Secondary, "Quit", !busy).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if busy {
                    ui.spinner();
                }
            },
        );
        self.note_ui(ui);
    }

    fn home_ui(&mut self, ui: &mut egui::Ui, busy: bool) {
        let palette = Palette::current(ui);
        egui::Panel::left("start-nav")
            .exact_size(216.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.side)
                    .inner_margin(egui::Margin::symmetric(14, 24)),
            )
            .show(ui, |ui| {
                let rect = ui.max_rect().expand2(egui::vec2(16.0, 20.0));
                ui.painter().line_segment(
                    [rect.right_top(), rect.right_bottom()],
                    egui::Stroke::new(1.0, palette.rule),
                );
                self.nav_ui(ui, busy);
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.page))
            .show(ui, |ui| match self.tab {
                Tab::Create => self.create_ui(ui, busy),
                Tab::Recent | Tab::All | Tab::Shared => self.list_ui(ui, busy),
            });
    }

    fn nav_ui(&mut self, ui: &mut egui::Ui, busy: bool) {
        let palette = Palette::current(ui);
        ui.horizontal(|ui| {
            theme::logo(ui, self.logo.as_ref(), 32.0);
            ui.label(theme::bold("BeamCraft", 19.0).color(palette.side_text()));
        });
        ui.add_space(26.0);
        let counts = [
            (Tab::Recent, "Recent models", Icon::Recent, String::new()),
            (Tab::All, "All models", Icon::Folder, String::new()),
            (Tab::Create, "New model", Icon::Plus, String::new()),
        ];
        for (tab, label, icon, count) in counts {
            let active = self.tab == tab;
            if nav_item(ui, label, icon, &count, active).clicked() && !busy {
                if tab == Tab::Create {
                    self.show_create();
                } else {
                    self.tab = tab;
                }
                self.selected = None;
                if tab == Tab::All && self.all.is_empty() && !self.folder.trim().is_empty() && !busy
                {
                    self.list_folder();
                }
            }
        }
        ui.add_space((ui.available_height() - PROFILE_CARD_HEIGHT).max(0.0));
        ui.painter().hline(
            ui.max_rect().x_range(),
            ui.cursor().top(),
            egui::Stroke::new(1.0, palette.side_grid),
        );
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Environment")
                    .size(11.0)
                    .color(palette.side_text()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(self.environment().unwrap_or("none"))
                        .size(11.0)
                        .color(palette.side_text()),
                );
            });
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            let (avatar, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(avatar.center(), 14.0, palette.side_grid);
            let name = self
                .settings
                .profile
                .as_ref()
                .map_or("Author", |profile| profile.name.as_str());
            ui.painter().text(
                avatar.center(),
                egui::Align2::CENTER_CENTER,
                name.chars().next().unwrap_or('A'),
                egui::FontId::proportional(11.0),
                palette.side_text(),
            );
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(name)
                        .size(12.0)
                        .color(palette.side_text()),
                );
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(
                            egui::RichText::new("Local workspace · Settings")
                                .size(10.0)
                                .color(palette.side_text()),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    self.launcher = true;
                }
            });
        });
    }

    fn header(&mut self, ui: &mut egui::Ui, title: &str, back: bool, busy: bool) {
        let palette = Palette::current(ui);
        egui::Frame::new()
            .fill(palette.page)
            .inner_margin(egui::Margin::symmetric(36, 28))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if back && theme::button(ui, Kind::Secondary, "‹ Back", !busy).clicked() {
                        self.tab = Tab::Recent;
                    }
                    ui.label(theme::bold(if back { title } else { "Your models" }, 28.0));
                    if back || self.tab == Tab::Recent {
                        return;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::button(ui, Kind::Primary, "+  New model", !busy).clicked() {
                            self.show_create();
                        }
                        if theme::icon_text_button(ui, Icon::Folder, "Open model...", !busy, 140.0)
                            .clicked()
                            && let Some(path) = self.browse("Open a BeamCraft model folder")
                        {
                            self.open_project(path);
                        }
                    });
                });
            });
    }

    fn list_ui(&mut self, ui: &mut egui::Ui, busy: bool) {
        let palette = Palette::current(ui);
        let title = match self.tab {
            Tab::All => "All models",
            Tab::Shared => "Shared models",
            Tab::Recent | Tab::Create => "Recent",
        };
        self.header(ui, title, false, busy);
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(36, 0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::bold(title, 13.0));
                    if self.tab == Tab::All {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .id(egui::Id::unique("model-search"))
                                .hint_text("Search models")
                                .desired_width(240.0)
                                .char_limit(MAX_FIELD),
                        );
                        if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
                            response.request_focus();
                        }
                    });
                    }
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(14.0);
                if self.tab == Tab::Shared {
                    ui.vertical_centered(|ui| {
                        ui.add_space(120.0);
                        theme::icon(ui, Icon::Shared, 96.0, palette.line);
                        ui.label(theme::bold("Shared models are coming soon", 22.0));
                        ui.label(
                            egui::RichText::new(
                                "Work on one model with your team through the BeamCraft cloud. Until then, models live in folders on this computer.",
                            )
                            .color(palette.muted),
                        );
                    });
                    return;
                }
                if self.tab == Tab::All {
                    ui.horizontal(|ui| {
                        theme::icon(ui, Icon::Folder, 26.0, palette.line);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.folder)
                                .hint_text("Folder with models")
                                .desired_width(460.0)
                                .min_size(egui::vec2(0.0, 40.0))
                                .align(egui::Align2::LEFT_CENTER)
                                .char_limit(MAX_FIELD),
                        );
                        if theme::button(ui, Kind::Secondary, "Browse…", !busy).clicked()
                            && let Some(dir) = self.browse("Folder with models")
                        {
                            self.folder = dir;
                            self.list_folder();
                        }
                        if theme::button(ui, Kind::Secondary, "Refresh", !busy).clicked() {
                            self.list_folder();
                        }
                        if busy {
                            ui.spinner();
                        }
                    });
                    ui.add_space(6.0);
                }
                let projects: Vec<ProjectView> = match self.tab {
                    Tab::All => self.all.clone(),
                    _ => self.recent.clone(),
                };
                let needle = if self.tab == Tab::All { self.search.trim().to_lowercase() } else { String::new() };
                let shown: Vec<(usize, &ProjectView)> = projects
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| needle.is_empty() || p.name.to_lowercase().contains(&needle))
                    .collect();
                if shown.is_empty() {
                    let empty = match (self.tab, needle.is_empty()) {
                        (_, false) => "No model matches the search.",
                        (Tab::All, true) => "No model in this folder yet. Create one with New model.",
                        _ => "No model was opened yet. Create one with New model, or list a folder under All models.",
                    };
                    ui.label(egui::RichText::new(empty).color(palette.muted));
                } else {
                    if !shown.iter().any(|(index, _)| Some(*index) == self.selected) {
                        self.selected = shown.first().map(|(index, _)| *index);
                    }
                    let preview_width = 268.0;
                    let table_width = (ui.available_width() - preview_width - 20.0).max(280.0);
                    let body_height = (ui.available_height() - 70.0).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(table_width, body_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| self.project_table(ui, &shown, busy),
                        );
                        if let Some((_, project)) = shown.iter().find(|(index, _)| Some(*index) == self.selected) {
                            self.project_preview(ui, project, busy, body_height);
                        }
                    });
                }
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Double-click a model to open it. Enter opens the selected model.")
                        .small()
                        .color(palette.muted),
                );
                self.note_ui(ui);
            });
    }

    fn project_table(&mut self, ui: &mut egui::Ui, projects: &[(usize, &ProjectView)], busy: bool) {
        let palette = Palette::current(ui);
        let now = now_ms();
        let mut open = None;
        egui::Frame::new().show(ui, |ui| {
            ui.set_width(ui.available_width() - 6.0);
            let width = ui.available_width();
            let columns = [0.0, width * 0.60, width * 0.81];
            let head = ui
                .allocate_exact_size(egui::vec2(width, 40.0), egui::Sense::hover())
                .0;
            ui.painter().rect_filled(
                head,
                egui::CornerRadius {
                    nw: 5,
                    ne: 5,
                    sw: 0,
                    se: 0,
                },
                palette.page,
            );
            for (x, text) in columns.iter().zip(["MODEL NAME", "MODIFIED", "CREATED"]) {
                ui.painter().text(
                    egui::pos2(head.left() + 12.0 + x, head.center().y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    egui::FontId::new(11.0, theme::bold_family()),
                    palette.text,
                );
            }
            ui.painter().line_segment(
                [head.left_bottom(), head.right_bottom()],
                egui::Stroke::new(1.0, palette.rule),
            );
            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 40.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (index, project) in projects {
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(width, ROW_HEIGHT),
                            egui::Sense::click(),
                        );
                        let selected = self.selected == Some(*index);
                        let fill = if selected {
                            palette.select
                        } else if response.hovered() {
                            palette.head
                        } else {
                            palette.page
                        };
                        let painter = ui.painter();
                        painter.rect_filled(rect, 0.0, fill);
                        painter.line_segment(
                            [rect.left_bottom(), rect.right_bottom()],
                            egui::Stroke::new(1.0, palette.rule),
                        );
                        let preview = egui::Rect::from_min_size(
                            rect.left_center() + egui::vec2(10.0, -24.0),
                            egui::vec2(62.0, 48.0),
                        );
                        painter.rect_filled(preview, 3, palette.head);
                        painter.rect_stroke(
                            preview,
                            3,
                            egui::Stroke::new(1.0, palette.rule),
                            egui::StrokeKind::Inside,
                        );
                        if let Some(Ok(model)) = self.previews.get(&project.path) {
                            model.paint(painter, preview, palette);
                        } else {
                            theme::paint_icon(
                                painter,
                                preview.shrink(10.0),
                                Icon::Block,
                                palette.muted,
                            );
                        }
                        let text_x = preview.right() + 10.0;
                        // Name and folder stop before the date columns.
                        let names = painter.with_clip_rect(egui::Rect::from_x_y_ranges(
                            text_x..=rect.left() + 4.0 + columns[1],
                            rect.y_range(),
                        ));
                        names.text(
                            egui::pos2(text_x, rect.center().y - 10.0),
                            egui::Align2::LEFT_CENTER,
                            &project.name,
                            egui::FontId::new(13.0, theme::bold_family()),
                            palette.text,
                        );
                        let meta = if project.ready() {
                            parent_of(&project.path)
                        } else {
                            format!("cannot be opened: {}", project.state)
                        };
                        names.text(
                            egui::pos2(text_x, rect.center().y + 12.0),
                            egui::Align2::LEFT_CENTER,
                            meta,
                            egui::FontId::proportional(12.0),
                            if project.ready() {
                                palette.muted
                            } else {
                                palette.error
                            },
                        );
                        let when =
                            |ms: Option<u64>| ms.map(|ms| format_when(ms, now)).unwrap_or_default();
                        for (x, text, color) in [
                            (columns[1], when(project.modified_ms), palette.text),
                            (columns[2], when(project.created_ms), palette.muted),
                        ] {
                            painter.text(
                                egui::pos2(rect.left() + 12.0 + x, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                text,
                                egui::FontId::proportional(11.0),
                                color,
                            );
                        }
                        if response.clicked() {
                            self.selected = Some(*index);
                        }
                        if (response.double_clicked()
                            || (selected
                                && !ui.ctx().egui_wants_keyboard_input()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter))))
                            && project.ready()
                            && !busy
                        {
                            open = Some(project.path.clone());
                        }
                    }
                });
        });
        if let Some(path) = open {
            self.open_project(path);
        }
    }

    fn project_preview(
        &mut self,
        ui: &mut egui::Ui,
        project: &ProjectView,
        busy: bool,
        height: f32,
    ) {
        let palette = Palette::current(ui);
        if project.ready() {
            self.request_preview(&project.path, ui.ctx());
        }
        ui.allocate_ui_with_layout(
            egui::vec2(268.0, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(268.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(268.0, 184.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 4, palette.head);
                if let Some(Ok(model)) = self.previews.get(&project.path) {
                    model.paint(ui.painter(), rect, palette);
                } else {
                    theme::paint_icon(
                        ui.painter(),
                        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(48.0)),
                        Icon::Block,
                        palette.muted,
                    );
                }
                match self.previews.get(&project.path) {
                    Some(Ok(model)) => {
                        ui.label(
                            egui::RichText::new(format!("{} elements · mm", model.elements))
                                .small()
                                .color(palette.muted),
                        );
                    }
                    Some(Err(error)) => {
                        ui.label(
                            egui::RichText::new("Preview unavailable")
                                .small()
                                .color(palette.error),
                        )
                        .on_hover_text(error);
                        if ui.small_button("Retry preview").clicked() {
                            self.previews.remove(&project.path);
                        }
                    }
                    None if !project.ready() => {
                        ui.label(
                            egui::RichText::new("Preview unavailable for this model")
                                .small()
                                .color(palette.muted),
                        );
                    }
                    None => {
                        ui.label(
                            egui::RichText::new("Loading model preview...")
                                .small()
                                .color(palette.muted),
                        );
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(theme::bold(&project.name, 16.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("Local model")
                                .size(10.0)
                                .color(palette.muted),
                        );
                    });
                });
                ui.label(
                    egui::RichText::new(&project.path)
                        .small()
                        .color(palette.muted),
                );
                ui.add_space(16.0);
                for (label, value) in [
                    (
                        "Created",
                        project
                            .created_ms
                            .map(format_time)
                            .unwrap_or_else(|| "Unknown".into()),
                    ),
                    (
                        "Modified",
                        project
                            .modified_ms
                            .map(format_time)
                            .unwrap_or_else(|| "Unknown".into()),
                    ),
                    ("Units", "mm".into()),
                    ("State", project.state.clone()),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(label).size(12.0).color(palette.muted));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(value).size(12.0));
                        });
                    });
                }
                ui.add_space((ui.available_height() - 36.0).max(16.0));
                if theme::icon_text_button(
                    ui,
                    Icon::Folder,
                    "Open model",
                    project.ready() && !busy,
                    268.0,
                )
                .clicked()
                {
                    self.open_project(project.path.clone());
                }
            },
        );
    }

    fn create_ui(&mut self, ui: &mut egui::Ui, busy: bool) {
        let palette = Palette::current(ui);
        self.header(ui, "New model", true, busy);
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(32, 24))
            .show(ui, |ui| {
                ui.set_max_width(800.0);
                ui.label(theme::bold("Name", 14.0));
                field(ui, &mut self.new_name, 100);
                ui.label(
                    egui::RichText::new("Also the name of the model's folder.")
                        .small()
                        .color(palette.muted),
                );
                ui.add_space(8.0);
                ui.label(theme::bold("Location", 14.0));
                ui.horizontal(|ui| {
                    let location = ui.add(
                        egui::TextEdit::singleline(&mut self.new_parent)
                            .hint_text("Choose a folder with Browse…")
                            .desired_width(ui.available_width() - 130.0)
                            .min_size(egui::vec2(0.0, 42.0))
                            .align(egui::Align2::LEFT_CENTER)
                            .margin(egui::Margin::symmetric(12, 8))
                            .char_limit(MAX_FIELD),
                    );
                    if location.lost_focus() {
                        self.suggest_project_name();
                    }
                    if theme::button(ui, Kind::Secondary, "Browse…", !busy).clicked()
                        && let Some(dir) = self.browse("Where to create the model")
                    {
                        self.new_parent = dir;
                        self.suggest_project_name();
                    }
                });
                ui.add_space(8.0);
                ui.label(theme::bold("Template", 14.0));
                ui.horizontal(|ui| {
                    template_card(ui, "Blank", "An empty model with no grids or levels.", true);
                    template_card(ui, "Grid and levels", "Coming with the model tools.", false);
                    template_card(
                        ui,
                        "Your templates",
                        "Save any model as a template, later.",
                        false,
                    );
                });
                ui.add_space(8.0);
                ui.label(theme::bold("Who works on it", 14.0));
                ui.horizontal(|ui| {
                    let _ = ui.radio(true, "Just me");
                    ui.add_enabled(false, egui::RadioButton::new(false, "A team (coming soon)"));
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let ready =
                        !self.new_name.trim().is_empty() && !self.new_parent.trim().is_empty();
                    if theme::button(ui, Kind::Primary, "Create and open", ready && !busy).clicked()
                    {
                        self.create_project();
                    }
                    if theme::button(ui, Kind::Secondary, "Cancel", !busy).clicked() {
                        self.tab = Tab::Recent;
                    }
                    if busy {
                        ui.spinner();
                    }
                });
                self.note_ui(ui);
            });
    }
}

/// The size of the start window.
pub const LAUNCHER_SIZE: egui::Vec2 = egui::vec2(460.0, 520.0);
/// Height of the profile card at the bottom of the start page's side panel.
const PROFILE_CARD_HEIGHT: f32 = 108.0;
/// Height of one row of the model list; tall enough for a model preview.
const ROW_HEIGHT: f32 = 82.0;

fn field(ui: &mut egui::Ui, text: &mut String, limit: usize) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(ui.available_width())
            .min_size(egui::vec2(0.0, 36.0))
            .align(egui::Align2::LEFT_CENTER)
            .margin(egui::Margin::symmetric(12, 8))
            .char_limit(limit),
    )
}

fn nav_item(
    ui: &mut egui::Ui,
    label: &str,
    icon: Icon,
    count: &str,
    active: bool,
) -> egui::Response {
    let palette = Palette::current(ui);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::click());
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(5);
    if active || response.hovered() {
        painter.rect_filled(rect, radius, palette.side_grid);
    }
    let text = palette.side_text();
    theme::paint_icon(
        painter,
        egui::Rect::from_center_size(
            rect.left_center() + egui::vec2(24.0, 0.0),
            egui::Vec2::splat(19.0),
        ),
        icon,
        palette.side_text(),
    );
    painter.text(
        rect.left_center() + egui::vec2(48.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        text,
    );
    let badge = painter.layout_no_wrap(
        count.to_owned(),
        egui::FontId::new(12.0, theme::bold_family()),
        palette.text,
    );
    let pill = egui::Rect::from_center_size(
        rect.right_center() - egui::vec2(14.0 + badge.size().x / 2.0, 0.0),
        badge.size() + egui::vec2(16.0, 4.0),
    );
    painter.galley(pill.center() - badge.size() / 2.0, badge, palette.text);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn template_card(ui: &mut egui::Ui, title: &str, text: &str, available: bool) {
    let palette = Palette::current(ui);
    let frame = if available {
        theme::card(palette)
    } else {
        egui::Frame::new()
            .fill(palette.head)
            .stroke(egui::Stroke::new(1.0, palette.rule))
            .corner_radius(egui::CornerRadius::same(5))
    };
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(230.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(230.0, 110.0), egui::Sense::hover());
            if available {
                theme::paint_preview(ui.painter(), rect);
            }
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let color = if available {
                            palette.text
                        } else {
                            palette.muted
                        };
                        ui.label(theme::bold(title, 14.0).color(color));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_enabled(available, egui::RadioButton::new(available, ""));
                        });
                    });
                    ui.label(egui::RichText::new(text).small().color(palette.muted));
                });
        });
    });
}

fn parent_of(path: &str) -> String {
    Path::new(path)
        .parent()
        .and_then(Path::to_str)
        .unwrap_or_default()
        .to_owned()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A time as people say it: "just now", "5 minutes ago", "3 hours ago", "yesterday",
/// "4 days ago", else the date ("Oct 12, 2026", UTC).
pub fn format_when(ms: u64, now_ms: u64) -> String {
    let ago = now_ms.saturating_sub(ms) / 1000;
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match ago {
        0..60 => "just now".to_owned(),
        60..3_600 => plural(ago / 60, "minute"),
        3_600..86_400 => plural(ago / 3_600, "hour"),
        86_400..172_800 => "yesterday".to_owned(),
        172_800..604_800 => plural(ago / 86_400, "day"),
        _ => {
            let date = format_time(ms);
            let (year, rest) = date.split_at(4);
            let month: usize = rest.get(1..3).and_then(|m| m.parse().ok()).unwrap_or(1);
            let day: u32 = rest.get(4..6).and_then(|d| d.parse().ok()).unwrap_or(1);
            let names = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            let name = names.get(month.saturating_sub(1)).copied().unwrap_or("Jan");
            format!("{name} {day}, {year}")
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    /// A configuration directory of its own, so a test never reads or writes the user's settings.
    fn scratch_config(tag: &str) -> Option<PathBuf> {
        Some(std::env::temp_dir().join(format!("ui-start-{tag}-{}", std::process::id())))
    }

    #[test]
    fn new_model_entry_suggests_a_name_and_keeps_user_edits_during_the_reply() {
        let mut page = StartPage::new(
            PathBuf::from("missing-test-cli"),
            scratch_config("page"),
            Arc::new(Stats::default()),
        );
        page.job = None;
        page.show_create();
        assert_eq!(page.tab, Tab::Create);
        assert_eq!(page.new_name, DEFAULT_MODEL_NAME);
        let (sender, receiver) = std::sync::mpsc::channel();
        page.job = Some(receiver);
        page.new_name = "My structure".into();
        sender
            .send(Reply::Suggested {
                parent: page.new_parent.clone(),
                previous: DEFAULT_MODEL_NAME.into(),
                name: Ok("New model 3".into()),
            })
            .unwrap();
        page.poll();
        assert_eq!(page.new_name, "My structure");
    }

    #[test]
    fn welcome_content_fits_compact_window_with_small_bottom_padding() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let mut page = StartPage::new(
            PathBuf::from("missing-test-cli"),
            scratch_config("page"),
            Arc::new(Stats::default()),
        );
        page.job = None;
        page.loaded = true;
        page.name = "UI Test".into();
        page.email = "ui@example.test".into();
        page.environment = Some("blank".into());
        page.environments = vec![EnvironmentView {
            id: Some("blank".into()),
            title: Some("Blank".into()),
            origin: "builtin".into(),
            shadowed: false,
            refused: None,
        }];
        for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            ctx.set_theme(preference);
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, LAUNCHER_SIZE)),
                    ..Default::default()
                },
                |ui| {
                    egui::Panel::top("test-title")
                        .exact_size(36.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("BeamCraft");
                                theme::window_controls_with_maximize(ui, false);
                            });
                        });
                    page.ui(ui);
                },
            );
            let mut bottom = 0.0_f32;
            let mut buttons = 0;
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    assert!(!matches!(text.galley.text(), "Structural modeling" | "□"));
                    if matches!(text.galley.text(), "Continue" | "Quit") {
                        bottom = bottom.max(text.pos.y + text.galley.size().y);
                        buttons += 1;
                        assert!(text.pos.x >= 20.0);
                        assert!(text.pos.x + text.galley.size().x <= LAUNCHER_SIZE.x - 20.0);
                    }
                }
            }
            let size = output
                .viewport_output
                .values()
                .flat_map(|viewport| &viewport.commands)
                .filter_map(|command| {
                    if let egui::ViewportCommand::InnerSize(size) = command {
                        Some(*size)
                    } else {
                        None
                    }
                })
                .next_back();
            output.drop_without_applying_deltas();
            assert_eq!(buttons, 2);
            if let Some(size) = size {
                assert!(size.y <= 540.0, "compact welcome is too tall: {size:?}");
                assert!(
                    size.y - bottom <= 40.0,
                    "excessive bottom padding: {}",
                    size.y - bottom
                );
            }
        }
    }

    #[test]
    fn native_library_and_create_pages_render_with_inter_in_both_themes() {
        let ctx = egui::Context::default();
        assert!(
            crate::app::inter(&ctx),
            "the admitted Inter assets must load"
        );
        theme::install(&ctx);
        let mut page = StartPage::new(
            PathBuf::from("beamcraft-test-missing-cli"),
            scratch_config("library"),
            Arc::new(Stats::default()),
        );
        page.job = None;
        page.loaded = true;
        page.launcher = false;
        page.recent = vec![ProjectView {
            name: "Synthetic engineering model".into(),
            path: "synthetic-model".into(),
            state: "ready".into(),
            created_ms: Some(0),
            modified_ms: Some(0),
        }];
        for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            ctx.set_theme(preference);
            for tab in [Tab::Recent, Tab::Create, Tab::All] {
                page.tab = tab;
                page.search = if tab == Tab::Recent {
                    "non-matching query from All models".into()
                } else {
                    String::new()
                };
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 800.0),
                        )),
                        ..Default::default()
                    },
                    |ui| page.home_ui(ui, false),
                );
                let text: String = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape {
                            Some(text.galley.text())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let has_search = text.contains("Search models");
                let has_header_open = text.contains("Open model...");
                let has_header_new = text.contains("+  New model");
                let actions_fit = output.shapes.iter().all(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && matches!(
                            text.galley.text(),
                            "Open model"
                                | "Model thumbnail not generated"
                                | "Double-click a model to open it. Enter opens the selected model."
                        )
                    {
                        let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                        return bounds.right() <= 1280.0 && bounds.bottom() <= 800.0;
                    }
                    true
                });
                output.drop_without_applying_deltas();
                assert!(actions_fit, "library action clipped");
                assert!(text.contains("BeamCraft"), "{tab:?}: {text}");
                assert_eq!(has_search, tab == Tab::All);
                if tab == Tab::Recent {
                    assert!(!has_header_open && !has_header_new);
                    assert!(text.contains("Your models"));
                    assert!(text.contains("Synthetic engineering model"));
                    assert!(text.contains("Open model"));
                    assert_eq!(page.selected, Some(0));
                }
            }
        }
    }

    #[test]
    fn shows_settings_as_the_page_reads_them() {
        let mut settings = Settings::default();
        settings
            .set_profile("Ana", "a@b.cl", || {
                Uuid7::parse("0199e7a0-0000-7000-8000-000000000001").map_err(|_| SettingsError {
                    code: ErrorCode::InvalidArgument,
                    message: "id",
                })
            })
            .unwrap();
        settings.environment = Some("blank".into());
        settings.skip_launcher = true;
        settings.theme = session::settings::ThemePreference::Dark;
        let view = settings_view(Path::new("/c"), &settings);
        assert_eq!(view.environment.as_deref(), Some("blank"));
        assert_eq!(view.theme, egui::ThemePreference::Dark);
        let profile = view.profile.unwrap();
        assert_eq!(profile.name, "Ana");
        assert_eq!(profile.actor_id, "0199e7a0-0000-7000-8000-000000000001");
        assert!(view.skip_launcher && view.models_dir.is_none());
        assert_eq!(view.config_dir, Path::new("/c").to_string_lossy());
    }

    #[test]
    fn a_missing_configuration_directory_is_a_typed_refusal() {
        // An explicit directory always resolves; only the fallback can fail.
        assert_eq!(
            resolve_config_dir(Some(Path::new("/c"))).unwrap(),
            Path::new("/c")
        );
    }

    #[test]
    fn shows_environments_and_projects() {
        let ready = EnvironmentEntry {
            path: PathBuf::from("/e"),
            origin: "builtin",
            state: EnvironmentState::Ready {
                id: "blank".into(),
                title: Some("Blank".into()),
                shadowed: false,
            },
        };
        let refused = EnvironmentEntry {
            path: PathBuf::from("/u"),
            origin: "user",
            state: EnvironmentState::Refused(Refusal::invalid("no id")),
        };
        let envs = environment_views(&[ready, refused]);
        assert_eq!(envs[0].label(), "Blank (blank)");
        assert_eq!(envs[0].usable_id(), Some("blank"));
        assert_eq!(envs[1].usable_id(), None);
        assert!(envs[1].refused.as_deref().unwrap().contains("no id"));
        let unreadable = project_view(&ProjectInfo {
            path: PathBuf::from("/m/B"),
            name: "B".into(),
            state: ProjectState::Unreadable(ErrorCode::IntegrityError),
        });
        assert_eq!(unreadable.state, "INTEGRITY_ERROR");
        assert!(!unreadable.ready());
        let missing = project_view(&ProjectInfo {
            path: PathBuf::from("/m/C"),
            name: "C".into(),
            state: ProjectState::Missing,
        });
        assert_eq!(
            (missing.state.as_str(), missing.created_ms),
            ("missing", None)
        );
    }
    #[test]
    fn builds_arguments() {
        let args = project_create_args(Some(Path::new("/c")), "/m", "Nave 1", Some("id"), None);
        assert_eq!(
            args,
            [
                "create-project",
                "--parent",
                "/m",
                "--name",
                "Nave 1",
                "--actor-id",
                "id",
                "--config-dir",
                "/c"
            ]
        );
        let args = project_create_args(None, "/m", "Nave 1", None, Some("starter"));
        assert_eq!(
            args,
            [
                "create-project",
                "--parent",
                "/m",
                "--name",
                "Nave 1",
                "--environment",
                "starter"
            ]
        );
    }

    #[test]
    fn says_when_like_people_do() {
        let now = 1_792_799_652_530;
        assert_eq!(format_when(now - 5_000, now), "just now");
        assert_eq!(format_when(now - 60_000, now), "1 minute ago");
        assert_eq!(format_when(now - 2 * 3_600_000, now), "2 hours ago");
        assert_eq!(format_when(now - 30 * 3_600_000, now), "yesterday");
        assert_eq!(format_when(now - 4 * 86_400_000, now), "4 days ago");
        assert_eq!(format_when(now - 30 * 86_400_000, now), "Sep 23, 2026");
        assert_eq!(format_when(now + 10_000, now), "just now");
    }

    #[test]
    fn formats_utc_times() {
        assert_eq!(format_time(0), "1970-01-01 00:00");
        assert_eq!(format_time(951_782_400_000), "2000-02-29 00:00");
        assert_eq!(format_time(1_792_799_652_530), "2026-10-23 23:54");
    }
}
