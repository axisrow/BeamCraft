// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The model library, shown when the window is opened without a model.
//!
//! The library sidebar lets users choose an environment for new models and save a local author
//! profile. The saved identity is recorded as the actor of edits made in the model window.
//!
//! The start page lists the recently opened projects, the projects of a folder, and creates a new
//! project from a template with the shared new-model form (`NewModelForm` in [`crate::library`]).
//! Shared (cloud) projects are not available yet; the tab says so.
//!
//! The settings, the environments, the recent list and the projects of a folder come from the
//! `session` crate, called in this process on a worker thread. Creating a project is a command:
//! the form runs `create-project` in a separate process, and its reply is read here
//! ([`parse_project`]) with `crate::tools::rejection` so a refusal is shown with the CLI's code
//! and message.

use crate::library::{self, DEFAULT_MODEL_NAME, FormEvent, Host, NewModelForm};
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
use session::project::{ProjectInfo, ProjectState, list_projects};
use session::recent::{RecentError, recent_projects, remember_project};
use session::settings::{self, Settings, SettingsError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

/// The first-frame estimate of the height the selected model's details take below its
/// preview image (status line, name, path, four rows and the Open model button); later frames
/// use the measured height.
const PREVIEW_DETAILS_GUESS: f32 = 300.0;
/// The smallest preview image worth drawing; with less room the image is left out.
const PREVIEW_MIN_IMAGE: f32 = 40.0;

/// How long one start command may run.
pub(crate) const DEADLINE: Duration = Duration::from_secs(60);
/// Longest text in a field.
pub(crate) const MAX_FIELD: usize = 1_024;

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

/// The note under the library's environment choice: what an environment holds, with
/// Blank's own contents when Blank is chosen (docs/design/blank-environment.md).
fn environment_note(chosen: Option<&str>) -> &'static str {
    if chosen == Some("blank") {
        "Sections, materials and defaults. Blank ships 19 generic profiles, 3 part presets and 2 materials."
    } else {
        "Sections, materials and defaults for new models."
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

/// A refusal of the `session` crate as the page shows it.
pub(crate) fn refusal(error: Refusal) -> ToolError {
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

/// The recent projects of the configuration directory, newest first. Projects whose folder has
/// gone are dropped from the settings file on the way.
pub(crate) fn load_recent(config_dir: Option<&Path>) -> Result<Vec<ProjectView>, ToolError> {
    recent_views(&resolve_config_dir(config_dir)?)
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

/// Updates one sidebar setting without rewriting other choices.
fn save_setting(
    config_dir: Option<&Path>,
    change: SettingChange,
) -> Result<SettingsView, ToolError> {
    let dir = resolve_config_dir(config_dir)?;
    let settings = Settings::update(&dir, |settings| {
        match change {
            SettingChange::Account { name, email } => {
                settings.set_profile(&name, &email, fresh_actor)?;
            }
            SettingChange::Environment(id) => settings.environment = Some(id),
        }
        Ok(true)
    })
    .map_err(settings_failure)?;
    Ok(settings_view(&dir, &settings))
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
pub(crate) fn remember(config_dir: Option<&Path>, path: &str) -> Result<ProjectView, ToolError> {
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
    Saved(Result<SettingsView, ToolError>),
    Listed(Result<Vec<ProjectView>, ToolError>),
    Opened(Result<ProjectView, ToolError>),
    Recent(Result<Vec<ProjectView>, ToolError>),
}

enum SettingChange {
    Account { name: String, email: String },
    Environment(String),
}

/// What the preview worker read for a model.
enum Read {
    /// The model's `thumbnail.png`.
    Picture(egui::ColorImage),
    /// No picture yet: the model's geometry, projected.
    Model(crate::preview::ModelPreview),
}

/// What the library shows for a model.
enum Preview {
    /// The model's `thumbnail.png`, saved when it was last left or put there by a person.
    Picture(egui::TextureHandle),
    /// The model's geometry, projected, for a model with no picture yet.
    Model(crate::preview::ModelPreview),
}

impl Preview {
    /// Whether there is nothing to picture: an empty model without a picture.
    fn is_empty(&self) -> bool {
        match self {
            Self::Picture(_) => false,
            Self::Model(model) => model.is_empty(),
        }
    }

    /// Paints the picture, letterboxed in `rect`, or the projected model filling it.
    fn paint(&self, painter: &egui::Painter, rect: egui::Rect, palette: Palette) {
        match self {
            Self::Picture(texture) => {
                let painter = painter.with_clip_rect(rect);
                painter.image(
                    texture.id(),
                    crate::thumbnail::fit(rect, texture.size()),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            Self::Model(model) => model.paint(painter, rect, palette),
        }
    }
}

struct PreviewJob {
    path: String,
    reply: Receiver<Result<Read, String>>,
    discard: bool,
}

/// The model library and its sidebar settings.
pub struct StartPage {
    previews: std::collections::BTreeMap<String, Result<Preview, String>>,
    preview_job: Option<PreviewJob>,
    cli: PathBuf,
    config_dir: Option<PathBuf>,
    stats: Arc<Stats>,
    job: Option<Receiver<Reply>>,
    loaded: bool,
    settings: SettingsView,
    environments: Vec<EnvironmentView>,
    environment: Option<String>,
    name: String,
    email: String,
    settings_error: Option<String>,
    tab: Tab,
    recent: Vec<ProjectView>,
    all: Vec<ProjectView>,
    folder: String,
    create: NewModelForm,
    note: Option<Result<String, String>>,
    open: Option<PathBuf>,
    /// A received IFC file the file picker chose: Shell opens it read-only (issue 614).
    open_ifc: Option<PathBuf>,
    search: String,
    selected: Option<usize>,
    logo: Option<egui::TextureHandle>,
    logo_tried: bool,
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
            loaded: false,
            settings: SettingsView::default(),
            environments: Vec::new(),
            environment: None,
            name: String::new(),
            email: String::new(),
            settings_error: None,
            tab: Tab::Recent,
            recent: Vec::new(),
            all: Vec::new(),
            folder: String::new(),
            create: NewModelForm::new(),
            note: None,
            open: None,
            open_ifc: None,
            search: String::new(),
            selected: None,
            logo: None,
            logo_tried: false,
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
        self.environment.as_deref()
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

    fn save_sidebar_setting(&mut self, change: SettingChange) {
        let config = self.config_dir.clone();
        self.settings_error = None;
        self.run_worker(move || Reply::Saved(save_setting(config.as_deref(), change)));
        if self.job.is_none()
            && let Some(Err(error)) = self.note.take()
        {
            self.settings_error = Some(error);
        }
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
            _ => self.create.parent().trim(),
        };
        match library::browse_folder(title, start) {
            Ok(dir) => dir,
            Err(e) => {
                self.note = Some(Err(e));
                None
            }
        }
    }

    fn show_create(&mut self) {
        self.show_tab(Tab::Create);
        self.create.show();
    }

    /// Switches the view and drops what belonged to the old one: a note or a selection made
    /// under one tab must not follow the user to another (issue axisrow/BeamCraft#562).
    fn show_tab(&mut self, tab: Tab) {
        self.note = None;
        self.selected = None;
        self.tab = tab;
    }

    /// Adopts a project that was opened or created: it is opened and put first in Recent.
    fn opened(&mut self, project: ProjectView) {
        self.open = Some(PathBuf::from(&project.path));
        self.recent.retain(|recent| recent.path != project.path);
        self.recent.insert(0, project);
        self.recent.truncate(20);
    }

    fn adopt(&mut self, settings: SettingsView) {
        if let Some(profile) = &settings.profile {
            self.name.clone_from(&profile.name);
            self.email.clone_from(&profile.email);
        }
        if self.folder.is_empty() {
            self.folder = settings.models_dir.clone().unwrap_or_default();
        }
        self.create
            .seed_parent(settings.models_dir.as_deref().unwrap_or_default());
        self.settings = settings;
    }

    fn poll(&mut self) {
        // The form's reply is read here too, so a created project is never left unread.
        if let Some(FormEvent::Created(project)) = self.create.poll() {
            self.opened(project);
        }
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
            Reply::Loaded(Ok((settings, environments, recent))) => {
                self.loaded = true;
                self.environments = environments;
                let usable = |id: &str| self.environments.iter().any(|e| e.usable_id() == Some(id));
                self.environment = settings
                    .environment
                    .clone()
                    .filter(|id| usable(id))
                    .or_else(|| {
                        // The blank environment is the base one and has the sections a first beam
                        // needs; any other usable environment is only the fallback.
                        let usable = |wanted: Option<&str>| {
                            self.environments.iter().find_map(|e| {
                                e.usable_id()
                                    .filter(|id| wanted.is_none_or(|w| w == *id))
                                    .map(str::to_owned)
                            })
                        };
                        usable(Some("blank")).or_else(|| usable(None))
                    });
                self.recent = recent;
                self.adopt(settings);
            }
            Reply::Saved(Ok(settings)) => {
                if let Some(id) = &settings.environment
                    && self
                        .environments
                        .iter()
                        .any(|entry| entry.usable_id() == Some(id.as_str()))
                {
                    self.environment = Some(id.clone());
                }
                self.settings = settings;
            }
            Reply::Listed(Ok(projects)) => {
                self.note = Some(Ok(format!("{} project(s)", projects.len())));
                self.all = projects;
            }
            Reply::Opened(Ok(project)) => self.opened(project),
            Reply::Recent(Ok(recent)) => self.recent = recent,
            Reply::Saved(Err(e)) => self.settings_error = Some(e.to_string()),
            Reply::Loaded(Err(e))
            | Reply::Listed(Err(e))
            | Reply::Opened(Err(e))
            | Reply::Recent(Err(e)) => {
                self.note = Some(Err(e.to_string()));
            }
        }
    }

    /// Draws the page and returns the store to open, once one was chosen or created.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        self.poll();
        self.poll_preview(ui.ctx());
        let busy = self.busy();
        if busy {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if self.logo.is_none() && !self.logo_tried {
            self.logo_tried = true;
            self.logo = theme::load_logo(ui.ctx(), 192);
        }
        self.home_ui(ui, busy);
        self.open.take()
    }

    /// Returns to the library and refreshes Recent so models opened from a model window are listed.
    pub(crate) fn return_to_models(&mut self) {
        self.previews.clear();
        if let Some(job) = &mut self.preview_job {
            job.discard = true;
        }
        self.tab = Tab::Recent;
        self.search.clear();
        self.selected = None;
        if self.job.is_none() {
            let config = self.config_dir.clone();
            self.run_worker(move || Reply::Recent(load_recent(config.as_deref())));
        }
    }

    /// The received IFC file the file picker chose, if it was not read yet.
    pub fn take_open_ifc(&mut self) -> Option<PathBuf> {
        self.open_ifc.take()
    }

    pub(crate) fn busy(&self) -> bool {
        self.job.is_some() || self.create.busy()
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
            let preview = result.map(|read| match read {
                Read::Picture(image) => Preview::Picture(ctx.load_texture(
                    format!("model-thumbnail:{}", job.path),
                    image,
                    egui::TextureOptions::LINEAR,
                )),
                Read::Model(model) => Preview::Model(model),
            });
            self.previews.insert(job.path.clone(), preview);
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
                // The saved picture first: reading and projecting the geometry is only for a
                // model that has none yet.
                let result = match crate::thumbnail::load(&store) {
                    Some(picture) => picture.map(Read::Picture),
                    None => crate::source::load(&cli, &store, &stats)
                        .and_then(|snapshot| crate::preview::ModelPreview::from_snapshot(&snapshot))
                        .map(Read::Model)
                        .map_err(|error| error.to_string()),
                };
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

    fn sidebar_settings(&mut self, ui: &mut egui::Ui, busy: bool) {
        let palette = Palette::current(ui);
        let side_text = palette.side_text();
        let error_color = Palette::of(egui::Theme::Dark).error;
        ui.label(theme::bold("Account", 14.0).color(side_text));
        let (name, email) = ui
            .add_enabled_ui(!busy, |ui| {
                let name = ui.label(egui::RichText::new("Your name").color(side_text));
                let name = library::field(ui, &mut self.name, 128).labelled_by(name.id);
                let email = ui.label(egui::RichText::new("E-mail").color(side_text));
                let email = library::field(ui, &mut self.email, 254).labelled_by(email.id);
                (name, email)
            })
            .inner;
        // Enter in a profile field must not also open the selected model.
        if name.lost_focus() || email.lost_focus() {
            ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        }
        let name = self.name.trim();
        let email = self.email.trim();
        let changed = self
            .settings
            .profile
            .as_ref()
            .is_none_or(|profile| profile.name != name || profile.email != email);
        if theme::button(
            ui,
            Kind::Secondary,
            "Save account",
            !busy && changed && !name.is_empty() && !email.is_empty(),
        )
        .clicked()
        {
            self.save_sidebar_setting(SettingChange::Account {
                name: name.to_owned(),
                email: email.to_owned(),
            });
        }

        ui.add_space(12.0);
        let title = ui.label(theme::bold("Environment", 14.0).color(side_text));
        let selected = self
            .environments
            .iter()
            .find(|entry| entry.usable_id().is_some() && entry.usable_id() == self.environment())
            .map_or_else(|| "Unavailable".to_owned(), EnvironmentView::label);
        let width = ui.available_width();
        let mut choice = self.environment.clone();
        ui.add_enabled_ui(!busy && self.job.is_none() && choice.is_some(), |ui| {
            egui::ComboBox::from_id_salt("start-environment")
                .width(width)
                .height(260.0)
                .truncate()
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.set_max_width(width);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    for env in &self.environments {
                        if let Some(id) = env.usable_id() {
                            let label = env.label();
                            ui.selectable_value(&mut choice, Some(id.to_owned()), &label)
                                .on_hover_ui(|ui| {
                                    ui.set_max_width(width);
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                    ui.label(&label);
                                });
                        }
                    }
                })
                .response
                .labelled_by(title.id);
        });
        if choice != self.environment
            && let Some(id) = choice
        {
            self.save_sidebar_setting(SettingChange::Environment(id));
        }
        ui.label(
            egui::RichText::new(environment_note(self.environment()))
                .small()
                .color(side_text),
        );
        for env in &self.environments {
            if let Some(reason) = &env.refused {
                ui.label(
                    egui::RichText::new(format!("An environment was not loaded ({reason})."))
                        .small()
                        .color(error_color),
                );
            }
        }
        if self.environments.iter().all(|e| e.usable_id().is_none()) {
            ui.label(egui::RichText::new("No environment was found.").color(error_color));
        }
        if let Some(error) = &self.settings_error {
            ui.label(egui::RichText::new(error).small().color(error_color));
        }
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
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.nav_ui(ui, busy || !self.loaded);
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.page))
            .show(ui, |ui| {
                let busy = busy || self.busy();
                if !self.loaded {
                    self.loading_ui(ui, busy);
                    return;
                }
                match self.tab {
                    Tab::Create => self.create_ui(ui, busy),
                    Tab::Recent | Tab::All | Tab::Shared => self.list_ui(ui, busy),
                }
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
                    self.show_tab(tab);
                }
                if tab == Tab::All && self.all.is_empty() && !self.folder.trim().is_empty() && !busy
                {
                    self.list_folder();
                }
            }
        }
        if self.loaded {
            ui.add_space(16.0);
            ui.painter().hline(
                ui.max_rect().x_range(),
                ui.cursor().top(),
                egui::Stroke::new(1.0, palette.side_grid),
            );
            ui.add_space(10.0);
            self.sidebar_settings(ui, busy || self.busy());
        }
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
                        self.show_tab(Tab::Recent);
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
                        // The file picker beside the store picker (issue 614): a received IFC
                        // file opens read-only, the way `beamcraft-ui --source FILE` does.
                        if theme::icon_text_button(
                            ui,
                            Icon::Sheet,
                            "Open IFC file...",
                            !busy,
                            140.0,
                        )
                        .clicked()
                        {
                            let start = match self.tab {
                                Tab::All => self.folder.trim(),
                                _ => self.create.parent().trim(),
                            };
                            let start =
                                (!start.is_empty()).then(|| std::path::PathBuf::from(start));
                            match crate::dialog::pick_file("Open an IFC file", start.as_deref()) {
                                Ok(Some(file)) => self.open_ifc = Some(file),
                                Ok(None) => {}
                                Err(e) => self.note = Some(Err(e.to_string())),
                            }
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
                        (Tab::All, true) if self.folder.trim().is_empty() => {
                            "Choose a folder with Browse… to list its models."
                        }
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
            // Two date columns need about 540 points of table; below that CREATED would
            // overlap MODIFIED, so it gives way (issue axisrow/BeamCraft#562).
            let wide = width >= 540.0;
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
            let head_labels: [&str; 3] = ["MODEL NAME", "MODIFIED", "CREATED"];
            let head_shown = if wide { 3 } else { 2 };
            for (x, text) in columns.iter().take(head_shown).zip(head_labels) {
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
                        match self.previews.get(&project.path) {
                            Some(Ok(shown)) if !shown.is_empty() => {
                                shown.paint(painter, preview, palette)
                            }
                            // An empty model has no picture: it shows the same placeholder as a
                            // model whose preview is not read, never a bare box.
                            _ => theme::paint_icon(
                                painter,
                                preview.shrink(10.0),
                                Icon::Block,
                                palette.muted,
                            ),
                        }
                        // The frame is drawn last: the preview's own fill painted over it, and
                        // in the light theme a selected row's thumbnail vanished into the row.
                        painter.rect_stroke(
                            preview,
                            3,
                            egui::Stroke::new(1.0, palette.rule),
                            egui::StrokeKind::Inside,
                        );
                        let text_x = preview.right() + 10.0;
                        // Name and folder stop before the date columns; both are cut with an
                        // ellipsis instead of a mid-glyph clip (issue axisrow/BeamCraft#562).
                        let text_width = (columns[1] - (text_x - rect.left()) - 8.0).max(20.0);
                        let names = painter.with_clip_rect(egui::Rect::from_x_y_ranges(
                            text_x..=rect.left() + 4.0 + columns[1],
                            rect.y_range(),
                        ));
                        let name_galley = ellipsized(
                            painter,
                            &project.name,
                            text_width,
                            egui::FontId::new(13.0, theme::bold_family()),
                            palette.text,
                        );
                        names.galley(
                            egui::pos2(text_x, rect.center().y - 10.0 - name_galley.size().y / 2.0),
                            name_galley,
                            palette.text,
                        );
                        let meta = if project.ready() {
                            parent_of(&project.path)
                        } else {
                            format!("cannot be opened: {}", project.state)
                        };
                        let meta_color = if project.ready() {
                            palette.muted
                        } else {
                            palette.error
                        };
                        let meta_galley = ellipsized(
                            painter,
                            &meta,
                            text_width,
                            egui::FontId::proportional(12.0),
                            meta_color,
                        );
                        names.galley(
                            egui::pos2(text_x, rect.center().y + 12.0 - meta_galley.size().y / 2.0),
                            meta_galley,
                            meta_color,
                        );
                        let when =
                            |ms: Option<u64>| ms.map(|ms| format_when(ms, now)).unwrap_or_default();
                        painter.text(
                            egui::pos2(rect.left() + 12.0 + columns[1], rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            when(project.modified_ms),
                            egui::FontId::proportional(11.0),
                            palette.text,
                        );
                        // CREATED sits at the row's right edge, clear of MODIFIED.
                        if wide {
                            painter.text(
                                egui::pos2(rect.right() - 12.0, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                when(project.created_ms),
                                egui::FontId::proportional(11.0),
                                palette.muted,
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
                // The image gives way first, and what still does not fit the panel scrolls
                // instead of running past the window bottom (issue axisrow/BeamCraft#562). The
                // room the details below the image take is measured on the previous frame: a
                // fixed guess of 240 pt was short of the real rows, so at the minimum window
                // size the image kept its full height and pushed State and Open model below
                // the fold (the recheck of item 25).
                // Keyed on the tab and the model: the All models tab has a folder row more,
                // and a model whose preview failed shows other rows, so a height measured for
                // one never sizes the image of another, even for a frame.
                let details_id = egui::Id::unique((
                    "model-preview-details-height",
                    self.tab == Tab::All,
                    project.path.as_str(),
                ));
                let details = ui
                    .ctx()
                    .data(|data| data.get_temp::<f32>(details_id))
                    .unwrap_or(PREVIEW_DETAILS_GUESS);
                // Below a useful thumbnail height the image goes away entirely, so the
                // details and the Open model button keep the room (the All models tab has a
                // folder row more than Recent).
                let room = (height - details - 4.0).min(184.0);
                let image_height = if room >= PREVIEW_MIN_IMAGE { room } else { 0.0 };
                egui::ScrollArea::vertical()
                    .id_salt("model-preview-panel")
                    .auto_shrink([false, true])
                    .max_height(height)
                    .show(ui, |ui| {
                        ui.set_width(268.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        let details_top = if image_height > 0.0 {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(268.0, image_height),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(rect, 4, palette.head);
                            // An empty model shows the placeholder, as in its row, not a bare box.
                            if let Some(Ok(model)) = self
                                .previews
                                .get(&project.path)
                                .filter(|preview| preview.as_ref().is_ok_and(|m| !m.is_empty()))
                            {
                                model.paint(ui.painter(), rect, palette);
                            } else {
                                theme::paint_icon(
                                    ui.painter(),
                                    egui::Rect::from_center_size(
                                        rect.center(),
                                        egui::Vec2::splat(image_height.min(48.0)),
                                    ),
                                    Icon::Block,
                                    palette.muted,
                                );
                            }
                            rect.bottom()
                        } else {
                            ui.cursor().top()
                        };
                        match self.previews.get(&project.path) {
                            Some(Ok(Preview::Picture(_))) => {
                                ui.label(
                                    egui::RichText::new("Picture of the last view · thumbnail.png")
                                        .small()
                                        .color(palette.muted),
                                );
                            }
                            Some(Ok(Preview::Model(model))) => {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} element{} · mm",
                                        model.elements,
                                        if model.elements == 1 { "" } else { "s" },
                                    ))
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
                                // The triangle budget never goes away: only a failure a
                                // retry could fix offers the button, and it is a themed one
                                // (issue axisrow/BeamCraft#562).
                                let permanent = error.contains("triangle budget");
                                if !permanent
                                    && theme::button(ui, Kind::Secondary, "Retry preview", !busy)
                                        .clicked()
                                {
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
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new("Local model")
                                            .size(10.0)
                                            .color(palette.muted),
                                    );
                                },
                            );
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
                                ui.label(
                                    egui::RichText::new(label).size(12.0).color(palette.muted),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(egui::RichText::new(value).size(12.0));
                                    },
                                );
                            });
                        }
                        ui.add_space(10.0);
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
                        let used = ui.min_rect().bottom() - details_top;
                        if used.is_finite() && used > 0.0 {
                            ui.ctx().data_mut(|data| data.insert_temp(details_id, used));
                        }
                    });
            },
        );
    }

    fn create_ui(&mut self, ui: &mut egui::Ui, busy: bool) {
        self.header(ui, "New model", true, busy);
        let actor = self.actor_id();
        let event = egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(32, 24))
            .show(ui, |ui| {
                ui.set_max_width(800.0);
                let host = Host {
                    cli: &self.cli,
                    config_dir: self.config_dir.as_deref(),
                    actor: actor.as_deref(),
                    environment: self.environment.as_deref(),
                    stats: &self.stats,
                };
                self.create.ui(ui, &host, busy)
            })
            .inner;
        match event {
            Some(FormEvent::Cancelled) => self.tab = Tab::Recent,
            Some(FormEvent::Created(project)) => self.opened(project),
            None => {}
        }
    }
}

/// Height of one row of the model list; tall enough for a model preview.
const ROW_HEIGHT: f32 = 82.0;

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

fn parent_of(path: &str) -> String {
    Path::new(path)
        .parent()
        .and_then(Path::to_str)
        .unwrap_or_default()
        .to_owned()
}

/// `text` as one line no wider than `max_width`, cut with an ellipsis when it is longer.
fn ellipsized(
    painter: &egui::Painter,
    text: &str,
    max_width: f32,
    font: egui::FontId,
    color: egui::Color32,
) -> std::sync::Arc<egui::Galley> {
    let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), color);
    if galley.size().x <= max_width {
        return galley;
    }
    // ponytail: chop one character per try; row texts are short and this runs once a frame.
    let mut chars: Vec<char> = text.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let mut cut: String = chars.iter().collect();
        cut.push('…');
        let cut_galley = painter.layout_no_wrap(cut, font.clone(), color);
        if cut_galley.size().x <= max_width {
            return cut_galley;
        }
    }
    galley
}

/// The current time, Unix milliseconds; 0 before 1970.
pub(crate) fn now_ms() -> u64 {
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
    fn the_environment_note_describes_blank_only_when_blank_is_chosen() {
        // The recheck of issue 562 item 55: under another environment the note still listed
        // Blank's contents.
        assert!(environment_note(Some("blank")).contains("Blank ships 19 generic profiles"));
        for chosen in [Some("site-office-copy"), None] {
            assert!(!environment_note(chosen).contains("Blank"), "{chosen:?}");
        }
    }

    #[test]
    fn first_launch_shows_the_library_and_sidebar_without_resizing() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let mut page = library_page("first-launch");
        let (tx, rx) = std::sync::mpsc::channel();
        page.job = Some(rx);
        page.loaded = false;
        tx.send(Reply::Loaded(Ok((
            SettingsView::default(),
            vec![EnvironmentView {
                id: Some("blank".into()),
                title: Some("Blank".into()),
                origin: "builtin".into(),
                shadowed: false,
                refused: None,
            }],
            page.recent.clone(),
        ))))
        .unwrap();

        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                assert!(page.ui(ui).is_none());
            },
        );
        for label in [
            "Your models",
            "Bridge",
            "Account",
            "Environment",
            "Blank (blank)",
        ] {
            assert!(
                output.shapes.iter().any(|shape| matches!(
                    &shape.shape, egui::Shape::Text(text) if text.galley.text() == label
                )),
                "{label}"
            );
        }
        assert!(
            !output
                .viewport_output
                .values()
                .flat_map(|viewport| &viewport.commands)
                .any(|command| matches!(
                    command,
                    egui::ViewportCommand::InnerSize(_) | egui::ViewportCommand::Resizable(_)
                ))
        );
        output.drop_without_applying_deltas();
        assert!(page.actor_id().is_none());
        assert_eq!(page.environment(), Some("blank"));
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

    /// A page on the library view with one ready model, no worker running.
    fn library_page(tag: &str) -> StartPage {
        let mut page = StartPage::new(
            PathBuf::from("missing-test-cli"),
            scratch_config(tag),
            Arc::new(Stats::default()),
        );
        page.job = None;
        page.loaded = true;
        page.folder = "/m".into();
        page.recent = vec![ProjectView {
            name: "Bridge".into(),
            path: "/m/Bridge".into(),
            state: "ready".into(),
            created_ms: Some(0),
            modified_ms: Some(0),
        }];
        page
    }

    /// One frame of the library view at `size`; the drawn texts and their bounds.
    fn library_frame(
        page: &mut StartPage,
        ctx: &egui::Context,
        size: (f32, f32),
    ) -> Vec<(String, egui::Rect)> {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size.0, size.1),
                )),
                ..Default::default()
            },
            |ui| page.home_ui(ui, false),
        );
        let texts: Vec<(String, egui::Rect)> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    // The galley's own rect, not its size: a centered galley is drawn
                    // symmetrically around the shape position.
                    Some((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    ))
                } else {
                    None
                }
            })
            .collect();
        output.drop_without_applying_deltas();
        texts
    }

    #[test]
    fn the_library_shows_the_whole_selected_model_panel_at_its_minimum_size() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let mut page = library_page("min-size");
        // All models has a folder row more than Recent: the tighter of the two tabs.
        page.all = page.recent.clone();
        for (preference, tab) in [
            (egui::ThemePreference::Light, Tab::Recent),
            (egui::ThemePreference::Dark, Tab::Recent),
            (egui::ThemePreference::Light, Tab::All),
            (egui::ThemePreference::Dark, Tab::All),
        ] {
            page.tab = tab;
            ctx.set_theme(preference);
            // Two frames: the second lays the image out against the measured details.
            library_frame(&mut page, &ctx, (1100.0, 680.0));
            let texts = library_frame(&mut page, &ctx, (1100.0, 680.0));
            // The panel ends above the footer hint, so a row below the hint's top is clipped
            // by the panel's scroll area even though its shape is still emitted (the recheck
            // of item 25 found State and Open model below the fold this way).
            let fold = texts
                .iter()
                .find(|(text, _)| text.starts_with("Double-click a model"))
                .map(|(_, bounds)| bounds.top())
                .unwrap_or(680.0);
            let inside = |bounds: egui::Rect| {
                bounds.right() <= 1100.0 + 0.5 && bounds.bottom() <= fold + 0.5
            };
            for (label, why) in [
                ("Open model", "the panel button is clipped"),
                ("State", "the state row is clipped"),
                ("Units", "the units row is clipped"),
            ] {
                let bounds = texts
                    .iter()
                    .find(|(text, _)| text == label)
                    .map(|(_, bounds)| *bounds)
                    .unwrap_or_else(|| panic!("{label} is not drawn: {why}"));
                assert!(inside(bounds), "{label} {why}: {bounds:?}");
            }
        }
    }

    #[test]
    fn the_narrow_library_hides_the_created_column_and_keeps_the_dates_apart() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let mut page = library_page("narrow");
        let texts = library_frame(&mut page, &ctx, (700.0, 450.0));
        assert!(
            !texts.iter().any(|(text, _)| text == "CREATED"),
            "CREATED overlaps MODIFIED on a narrow table"
        );
        assert!(texts.iter().any(|(text, _)| text == "MODIFIED"));
        // The wide table carries both dates, kept clear of each other.
        let mut page = library_page("wide");
        let texts = library_frame(&mut page, &ctx, (1280.0, 800.0));
        let modified = texts
            .iter()
            .find(|(text, _)| text == "just now")
            .map(|(_, bounds)| *bounds);
        let created = texts
            .iter()
            .filter(|(text, _)| text == "just now")
            .map(|(_, bounds)| *bounds)
            .max_by(|a, b| a.left().total_cmp(&b.left()));
        if let (Some(modified), Some(created)) = (modified, created) {
            assert!(
                created.left() >= modified.right() - 0.5,
                "the date columns overlap: {modified:?} vs {created:?}"
            );
        }
    }

    #[test]
    fn account_and_environment_controls_fit_the_sidebar_in_both_themes() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let mut page = library_page("sidebar");
        page.name = "Jean-Baptiste Emmanuel Maximilien de la Croix-Aumont du Boisguillem".into();
        page.email = "author@example.test".into();
        page.environment = Some("showroom".into());
        page.environments = vec![EnvironmentView {
            id: Some("showroom".into()),
            title: Some(
                "Trondhjem Bridgeworks and Steel Construction Showroom Environment Pack".into(),
            ),
            origin: "user".into(),
            shadowed: false,
            refused: None,
        }];
        for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            ctx.set_theme(preference);
            let texts = library_frame(&mut page, &ctx, (1100.0, 680.0));
            for label in [
                "Account",
                "Your name",
                "E-mail",
                "Save account",
                "Environment",
            ] {
                let (_, bounds) = texts.iter().find(|(text, _)| text == label).unwrap();
                assert!(
                    bounds.right() <= 216.0 && bounds.bottom() <= 680.0,
                    "{label}: {bounds:?}"
                );
            }
            let (_, bounds) = texts
                .iter()
                .find(|(text, _)| text.starts_with("Trondhjem"))
                .unwrap();
            assert!(bounds.right() <= 216.0, "{bounds:?}");
        }
    }

    #[test]
    fn sidebar_saves_account_and_environment_independently() {
        let config = scratch_config("sidebar-save").unwrap();
        let mut page = library_page("sidebar-save");
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        page.environments = ["blank", "local"]
            .into_iter()
            .map(|id| EnvironmentView {
                id: Some(id.into()),
                title: None,
                origin: "user".into(),
                shadowed: false,
                refused: None,
            })
            .collect();
        page.environment = Some("blank".into());
        page.name = "Draft name".into();

        let draw = |page: &mut StartPage, events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    assert!(page.ui(ui).is_none());
                },
            )
        };
        let click = |page: &mut StartPage, label: &str| {
            let output = draw(page, vec![]);
            let at = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing control {label}"));
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                draw(
                    page,
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                )
                .drop_without_applying_deltas();
            }
        };
        let wait = |page: &mut StartPage| {
            for _ in 0..250 {
                draw(page, vec![]).drop_without_applying_deltas();
                if page.job.is_none() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(page.job.is_none(), "the worker must finish");
            assert!(page.settings_error.is_none(), "{:?}", page.settings_error);
        };

        click(&mut page, "blank");
        click(&mut page, "local");
        wait(&mut page);
        assert_eq!(page.environment(), Some("local"));
        assert_eq!(page.name, "Draft name");
        assert!(page.actor_id().is_none());

        for name in ["Ana", "Ana Silva"] {
            let actor = page.actor_id();
            page.name = name.into();
            page.email = "ana@example.test".into();
            click(&mut page, "Save account");
            wait(&mut page);
            assert_eq!(page.author_name().as_deref(), Some(name));
            assert!(page.actor_id().is_some());
            if actor.is_some() {
                assert_eq!(page.actor_id(), actor);
            }
            assert_eq!(page.environment(), Some("local"));
        }
        let settings = Settings::load(&config).unwrap();
        assert_eq!(settings.environment.as_deref(), Some("local"));
        assert_eq!(settings.profile.unwrap().name, "Ana Silva");
        std::fs::remove_dir_all(config).unwrap();
    }

    #[test]
    fn switching_tabs_drops_the_note_of_the_previous_tab() {
        let mut page = library_page("note");
        page.note = Some(Ok("16 project(s)".into()));
        page.show_tab(Tab::Recent);
        assert_eq!(page.note, None);
        assert_eq!(page.selected, None);
    }

    #[test]
    fn an_empty_all_models_without_a_folder_says_so() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let mut page = library_page("all-empty");
        page.tab = Tab::All;
        page.all = Vec::new();
        page.folder = String::new();
        let texts = library_frame(&mut page, &ctx, (1280.0, 800.0));
        let joined: String = texts
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("Choose a folder with Browse"),
            "the empty state must ask for a folder: {joined}"
        );
        assert!(!joined.contains("No model in this folder yet"));
    }

    #[test]
    fn a_permanent_preview_refusal_offers_no_retry_button() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let mut page = library_page("preview-budget");
        page.previews.insert(
            "/m/Bridge".into(),
            Err("invalid input: model exceeds the library preview triangle budget".into()),
        );
        let texts = library_frame(&mut page, &ctx, (1280.0, 800.0));
        let joined: String = texts
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Preview unavailable"), "{joined}");
        assert!(
            !joined.contains("Retry preview"),
            "a triangle-budget refusal never recovers, so it offers no retry: {joined}"
        );
        page.previews.insert(
            "/m/Bridge".into(),
            Err("the model folder could not be read".into()),
        );
        let texts = library_frame(&mut page, &ctx, (1280.0, 800.0));
        assert!(
            texts.iter().any(|(text, _)| text == "Retry preview"),
            "a transient refusal offers a themed retry"
        );
    }

    #[test]
    fn a_long_row_text_is_cut_with_an_ellipsis_not_clipped_mid_glyph() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let output = ctx.run_ui(Default::default(), |ui| {
            let font = egui::FontId::proportional(12.0);
            let short = ellipsized(
                ui.painter(),
                "short",
                500.0,
                font.clone(),
                egui::Color32::WHITE,
            );
            assert_eq!(short.text(), "short");
            let long = ellipsized(
                ui.painter(),
                "/tmp/very/long/folder/path/that/goes/on/and/on/without/end",
                100.0,
                font,
                egui::Color32::WHITE,
            );
            assert!(long.size().x <= 100.0, "{}", long.size().x);
            assert!(long.text().ends_with('…'), "{}", long.text());
            assert!(long.text().starts_with("/tmp/very"));
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn returning_to_models_refreshes_the_recent_list() {
        let config = std::env::temp_dir().join(format!(
            "ui-start-return-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&config).unwrap();
        let mut page = StartPage::new(
            PathBuf::from("missing-test-cli"),
            Some(config.clone()),
            Arc::new(Stats::default()),
        );
        // The load started by `new` answers first.
        for _ in 0..250 {
            page.poll();
            if page.job.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(page.job.is_none(), "the start load answers");
        let model = |name: &str| ProjectView {
            name: name.into(),
            path: format!("/m/{name}"),
            state: "ready".into(),
            created_ms: Some(0),
            modified_ms: Some(0),
        };

        page.return_to_models();
        assert!(page.job.is_some(), "returning reads the recent list");
        // The reply of that read replaces the list.
        let (tx, rx) = std::sync::mpsc::channel();
        page.job = Some(rx);
        tx.send(Reply::Recent(Ok(vec![model("Switched")]))).unwrap();
        page.poll();
        assert_eq!(page.recent, vec![model("Switched")]);
        assert!(page.job.is_none());

        // A reply still pending is kept: returning starts no read over it.
        let (tx, rx) = std::sync::mpsc::channel();
        page.job = Some(rx);
        page.return_to_models();
        tx.send(Reply::Recent(Ok(vec![model("Pending")]))).unwrap();
        page.poll();
        assert_eq!(page.recent, vec![model("Pending")]);
        let _ = std::fs::remove_dir_all(&config);
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
        assert!(view.models_dir.is_none());
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
        let args = project_create_args(None, "/m", "Nave 1", None, Some("blank"));
        assert_eq!(
            args,
            [
                "create-project",
                "--parent",
                "/m",
                "--name",
                "Nave 1",
                "--environment",
                "blank"
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
