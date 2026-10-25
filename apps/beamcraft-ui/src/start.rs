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
//! Everything here runs the same CLI commands a user types (`show-settings`, `set-settings`,
//! `list-environment`, `list-recent`, `add-recent`, `list-project`, `create-project`), each in a
//! separate process on a worker thread; replies are read with `crate::tools::rejection` so a
//! refusal is shown with the CLI's code and message. Nothing here reads a store directly.

use crate::command;
use crate::stats::Stats;
use crate::theme::{self, Icon, Kind, Palette};
use crate::tools::{ToolError, decode, rejection, text};
use beamcraft_json::Json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

/// How long one start command may run.
const DEADLINE: Duration = Duration::from_secs(60);
/// Longest text in a field.
const MAX_FIELD: usize = 1_024;

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

/// The user's settings as `show-settings` reports them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsView {
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

/// One environment as `list-environment` reports it.
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

/// One project folder as `list-project`, `list-recent` or `create-project` report it.
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

fn optional_text(json: &Json, key: &str) -> Result<Option<String>, ToolError> {
    match json.get(key).map_err(|_| unreadable())? {
        Json::Null => Ok(None),
        value => Ok(Some(value.string().map_err(|_| unreadable())?.to_owned())),
    }
}

fn optional_uint(json: &Json, key: &str) -> Option<u64> {
    json.get(key).ok().and_then(|v| v.unsigned().ok())
}

/// Reads the reply of `show-settings` or `set-settings`.
pub fn parse_settings(exit_code: Option<i32>, stdout: &[u8]) -> Result<SettingsView, ToolError> {
    let reply = match reply(exit_code, stdout, "settings") {
        Err(e) if e.code == "UNREADABLE_REPLY" => reply(exit_code, stdout, "settings_saved")?,
        other => other?,
    };
    let settings = reply.get("settings").map_err(|_| unreadable())?;
    let profile = match settings.get("profile").map_err(|_| unreadable())? {
        Json::Null => None,
        p => Some(Profile {
            actor_id: text(p, "actor_id").ok_or_else(unreadable)?,
            name: text(p, "name").ok_or_else(unreadable)?,
            email: text(p, "email").ok_or_else(unreadable)?,
        }),
    };
    Ok(SettingsView {
        config_dir: text(&reply, "config_dir").ok_or_else(unreadable)?,
        profile,
        environment: optional_text(settings, "environment")?,
        skip_launcher: settings
            .get("skip_launcher")
            .and_then(Json::boolean)
            .map_err(|_| unreadable())?,
        models_dir: optional_text(settings, "models_dir")?,
    })
}

/// Reads the reply of `list-environment`.
pub fn parse_environments(
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<Vec<EnvironmentView>, ToolError> {
    let reply = reply(exit_code, stdout, "environments")?;
    let list = reply
        .get("environments")
        .and_then(Json::array)
        .map_err(|_| unreadable())?;
    list.iter()
        .map(|e| {
            Ok(EnvironmentView {
                id: optional_text(e, "id")?,
                title: optional_text(e, "title")?,
                origin: text(e, "origin").ok_or_else(unreadable)?,
                shadowed: e
                    .get("shadowed")
                    .and_then(Json::boolean)
                    .map_err(|_| unreadable())?,
                refused: match e.get("refused").map_err(|_| unreadable())? {
                    Json::Null => None,
                    r => Some(format!(
                        "{}: {}",
                        text(r, "code").unwrap_or_default(),
                        text(r, "message").unwrap_or_default()
                    )),
                },
            })
        })
        .collect()
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

/// Reads the reply of `list-project`.
pub fn parse_projects(
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<Vec<ProjectView>, ToolError> {
    let reply = reply(exit_code, stdout, "projects")?;
    reply
        .get("projects")
        .and_then(Json::array)
        .map_err(|_| unreadable())?
        .iter()
        .map(project_of)
        .collect()
}

/// Reads the reply of `list-recent`.
pub fn parse_recent(exit_code: Option<i32>, stdout: &[u8]) -> Result<Vec<ProjectView>, ToolError> {
    let reply = reply(exit_code, stdout, "recent")?;
    reply
        .get("recent")
        .and_then(Json::array)
        .map_err(|_| unreadable())?
        .iter()
        .map(|r| project_of(r.get("project").map_err(|_| unreadable())?))
        .collect()
}

/// Reads the reply of `create-project` or `add-recent`: the project.
pub fn parse_project(
    exit_code: Option<i32>,
    stdout: &[u8],
    outcome: &str,
) -> Result<ProjectView, ToolError> {
    let reply = reply(exit_code, stdout, outcome)?;
    project_of(reply.get("project").map_err(|_| unreadable())?)
}

fn with_config(mut args: Vec<String>, config_dir: Option<&Path>) -> Vec<String> {
    if let Some(dir) = config_dir {
        args.push("--config-dir".into());
        args.push(dir.to_string_lossy().into_owned());
    }
    args
}

/// The `set-settings` arguments that save the start window's choices.
pub fn settings_set_args(
    config_dir: Option<&Path>,
    name: &str,
    email: &str,
    environment: &str,
    skip_launcher: bool,
) -> Vec<String> {
    let args = vec![
        "set-settings".to_owned(),
        "--name".into(),
        name.to_owned(),
        "--email".into(),
        email.to_owned(),
        "--environment".into(),
        environment.to_owned(),
        "--skip-launcher".into(),
        skip_launcher.to_string(),
    ];
    with_config(args, config_dir)
}

/// The `create-project` arguments of the Create tab.
pub fn project_create_args(
    config_dir: Option<&Path>,
    parent: &str,
    name: &str,
    actor_id: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "create-project".to_owned(),
        "--parent".into(),
        parent.to_owned(),
        "--name".into(),
        name.to_owned(),
    ];
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
}

/// The start window and the start page.
pub struct StartPage {
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
            new_name: String::new(),
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

    fn spawn(&mut self, work: impl FnOnce(&Path, &Stats) -> Reply + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        let cli = self.cli.clone();
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-start".into())
            .spawn(move || {
                let _ = tx.send(work(&cli, &stats));
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
        self.spawn(move |cli, stats| {
            let run = |args: Vec<String>| {
                command::run(cli, &with_config(args, config.as_deref()), DEADLINE, stats)
            };
            let loaded = (|| {
                let out = run(vec!["show-settings".into()])?;
                let settings = parse_settings(out.code, &out.stdout)?;
                let out = run(vec!["list-environment".into()])?;
                let environments = parse_environments(out.code, &out.stdout)?;
                let out = run(vec!["list-recent".into()])?;
                let recent = parse_recent(out.code, &out.stdout)?;
                Ok((settings, environments, recent))
            })();
            Reply::Loaded(loaded)
        });
    }

    fn save_launcher(&mut self) {
        let Some(environment) = self.environment.clone() else {
            self.note = Some(Err("choose an environment".into()));
            return;
        };
        let args = settings_set_args(
            self.config_dir.as_deref(),
            self.name.trim(),
            self.email.trim(),
            &environment,
            self.skip,
        );
        let config = self.config_dir.clone();
        self.spawn(move |cli, stats| {
            let saved = (|| {
                let out = command::run(cli, &args, DEADLINE, stats)?;
                let settings = parse_settings(out.code, &out.stdout)?;
                let out = command::run(
                    cli,
                    &with_config(vec!["list-recent".into()], config.as_deref()),
                    DEADLINE,
                    stats,
                )?;
                Ok((settings, parse_recent(out.code, &out.stdout)?))
            })();
            Reply::Saved(saved)
        });
    }

    fn list_folder(&mut self) {
        let folder = self.folder.trim().to_owned();
        if folder.is_empty() {
            self.note = Some(Err("enter a folder".into()));
            return;
        }
        // The folder is remembered for the next start; the list does not wait for that.
        let remember = with_config(
            vec!["set-settings".into(), "--models-dir".into(), folder.clone()],
            self.config_dir.as_deref(),
        );
        self.spawn(move |cli, stats| {
            let _ = command::run(cli, &remember, DEADLINE, stats);
            let args = vec!["list-project".to_owned(), "--dir".into(), folder];
            Reply::Listed(
                command::run(cli, &args, DEADLINE, stats)
                    .and_then(|out| parse_projects(out.code, &out.stdout)),
            )
        });
    }

    fn open_project(&mut self, path: String) {
        let args = with_config(
            vec!["add-recent".into(), "--store".into(), path],
            self.config_dir.as_deref(),
        );
        self.spawn(move |cli, stats| {
            Reply::Opened(
                command::run(cli, &args, DEADLINE, stats)
                    .and_then(|out| parse_project(out.code, &out.stdout, "recent_added")),
            )
        });
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
            self.new_name.trim(),
            self.actor_id().as_deref(),
        );
        self.spawn(move |cli, stats| {
            Reply::Opened(
                command::run(cli, &args, DEADLINE, stats)
                    .and_then(|out| parse_project(out.code, &out.stdout, "project_created")),
            )
        });
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
            Reply::Loaded(Ok((settings, environments, recent))) => {
                self.loaded = true;
                self.environments = environments;
                let usable = |id: &str| self.environments.iter().any(|e| e.usable_id() == Some(id));
                let saved = settings.environment.clone().filter(|id| usable(id));
                self.environment = saved.clone().or_else(|| {
                    self.environments
                        .iter()
                        .find_map(|e| e.usable_id().map(str::to_owned))
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
            Reply::Opened(Ok(project)) => self.open = Some(PathBuf::from(project.path)),
            Reply::Loaded(Err(e)) | Reply::Saved(Err(e)) => self.note = Some(Err(e.to_string())),
            Reply::Listed(Err(e)) | Reply::Opened(Err(e)) => {
                self.note = Some(Err(e.to_string()));
            }
        }
    }

    /// Draws the page and returns the store to open, once one was chosen or created.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        self.poll();
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
                        .inner_margin(egui::Margin::symmetric(40, 30)),
                )
                .show(ui, |ui| {
                    if self.loaded {
                        self.launcher_ui(ui, busy);
                        // The start window is as tall as its content: no empty band below the
                        // buttons, and room for a note when one appears.
                        // The panel's own rect fills the window; the cursor is where the
                        // content ends.
                        let needed = (ui.cursor().top() + 30.0).ceil();
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
            theme::logo(ui, self.logo.as_ref(), 88.0);
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
            theme::logo(ui, self.logo.as_ref(), 88.0);
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
        let row = egui::vec2(ui.available_width(), 48.0);
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
            .exact_size(248.0)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 20)))
            .show(ui, |ui| {
                let rect = ui.max_rect().expand2(egui::vec2(16.0, 20.0));
                theme::grid(ui.painter(), rect, palette.side, palette.side_grid, 34.0);
                ui.painter().line_segment(
                    [rect.right_top(), rect.right_bottom()],
                    egui::Stroke::new(2.0, palette.ink),
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
            theme::logo(ui, self.logo.as_ref(), 44.0);
            ui.label(theme::bold("BeamCraft", 19.0).color(palette.side_text()));
        });
        ui.add_space(14.0);
        let counts = [
            (
                Tab::Recent,
                "Recent",
                Icon::Recent,
                self.recent.len().to_string(),
            ),
            (
                Tab::All,
                "All models",
                Icon::Folder,
                self.all.len().to_string(),
            ),
            (Tab::Shared, "Shared", Icon::Shared, "soon".to_owned()),
        ];
        for (tab, label, icon, count) in counts {
            let active = self.tab == tab;
            if nav_item(ui, label, icon, &count, active).clicked() {
                self.tab = tab;
                self.selected = None;
                if tab == Tab::All && self.all.is_empty() && !self.folder.trim().is_empty() && !busy
                {
                    self.list_folder();
                }
            }
        }
        // The profile card sits at the bottom of the panel.
        ui.add_space((ui.available_height() - PROFILE_CARD_HEIGHT).max(0.0));
        ui.scope(|ui| {
            theme::card(palette, 3)
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        let caption = |ui: &mut egui::Ui, text: &str| {
                            ui.label(
                                egui::RichText::new(text)
                                    .small()
                                    .strong()
                                    .color(palette.muted),
                            );
                        };
                        caption(ui, "ENVIRONMENT");
                        ui.label(theme::bold(
                            self.environment().unwrap_or("none").to_owned(),
                            14.0,
                        ));
                        caption(ui, "AUTHOR");
                        if let Some(profile) = &self.settings.profile {
                            ui.label(theme::bold(profile.name.clone(), 14.0));
                            ui.label(
                                egui::RichText::new(&profile.email)
                                    .small()
                                    .color(palette.muted),
                            );
                        }
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Link::new(theme::bold("Change…", 13.0).color(palette.link)),
                            )
                            .clicked()
                        {
                            self.launcher = true;
                        }
                    });
                });
        });
    }

    fn header(&mut self, ui: &mut egui::Ui, title: &str, back: bool, busy: bool) {
        let palette = Palette::current(ui);
        egui::Frame::new()
            .fill(palette.surface)
            .inner_margin(egui::Margin::symmetric(32, 18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if back && theme::button(ui, Kind::Secondary, "‹ Back", !busy).clicked() {
                        self.tab = Tab::Recent;
                    }
                    ui.label(theme::bold(title, 24.0));
                    if back {
                        return;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::button(ui, Kind::Primary, "+  New model", !busy).clicked() {
                            self.tab = Tab::Create;
                            self.note = None;
                        }
                        if self.tab != Tab::Shared {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.search)
                                    .hint_text("Search models")
                                    .desired_width(280.0)
                                    .min_size(egui::vec2(0.0, 40.0))
                                    .align(egui::Align2::LEFT_CENTER)
                                    .char_limit(MAX_FIELD),
                            );
                        }
                    });
                });
            });
        let y = ui.min_rect().bottom();
        ui.painter().line_segment(
            [
                egui::pos2(ui.max_rect().left(), y),
                egui::pos2(ui.max_rect().right(), y),
            ],
            egui::Stroke::new(2.0, palette.ink),
        );
    }

    fn list_ui(&mut self, ui: &mut egui::Ui, busy: bool) {
        let palette = Palette::current(ui);
        let title = match self.tab {
            Tab::All => "All models",
            Tab::Shared => "Shared models",
            Tab::Recent | Tab::Create => "Recent models",
        };
        self.header(ui, title, false, busy);
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(32, 22))
            .show(ui, |ui| {
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
                let needle = self.search.trim().to_lowercase();
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
                    self.project_table(ui, &shown, busy);
                }
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Double-click a model to open it.")
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
        theme::card(palette, 4)
            .corner_radius(egui::CornerRadius::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 6.0);
                let width = ui.available_width();
                let columns = [0.0, width * 0.52, width * 0.70, width * 0.86];
                let head = ui
                    .allocate_exact_size(egui::vec2(width, 40.0), egui::Sense::hover())
                    .0;
                ui.painter().rect_filled(
                    head,
                    egui::CornerRadius {
                        nw: 12,
                        ne: 12,
                        sw: 0,
                        se: 0,
                    },
                    palette.head,
                );
                for (x, text) in columns.iter().zip(["NAME", "MODIFIED", "CREATED", ""]) {
                    ui.painter().text(
                        egui::pos2(head.left() + 20.0 + x, head.center().y),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::new(11.0, theme::bold_family()),
                        palette.text,
                    );
                }
                ui.painter().line_segment(
                    [head.left_bottom(), head.right_bottom()],
                    egui::Stroke::new(2.0, palette.ink),
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
                                palette.surface
                            };
                            let painter = ui.painter();
                            painter.rect_filled(rect, 0.0, fill);
                            painter.line_segment(
                                [rect.left_bottom(), rect.right_bottom()],
                                egui::Stroke::new(1.5, palette.rule),
                            );
                            let preview = egui::Rect::from_min_size(
                                rect.left_top() + egui::vec2(20.0, 10.0),
                                egui::vec2(132.0, 84.0),
                            );
                            theme::paint_preview(painter, preview);
                            let text_x = preview.right() + 16.0;
                            // Name and folder stop before the date columns.
                            let names = painter.with_clip_rect(egui::Rect::from_x_y_ranges(
                                text_x..=rect.left() + 4.0 + columns[1],
                                rect.y_range(),
                            ));
                            names.text(
                                egui::pos2(text_x, rect.center().y - 10.0),
                                egui::Align2::LEFT_CENTER,
                                &project.name,
                                egui::FontId::new(15.0, theme::bold_family()),
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
                            let when = |ms: Option<u64>| {
                                ms.map(|ms| format_when(ms, now)).unwrap_or_default()
                            };
                            for (x, text, color) in [
                                (columns[1], when(project.modified_ms), palette.text),
                                (columns[2], when(project.created_ms), palette.muted),
                            ] {
                                painter.text(
                                    egui::pos2(rect.left() + 20.0 + x, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    text,
                                    egui::FontId::proportional(14.0),
                                    color,
                                );
                            }
                            if response.clicked() {
                                self.selected = Some(*index);
                            }
                            if response.double_clicked() && project.ready() && !busy {
                                open = Some(project.path.clone());
                            }
                            if selected && project.ready() {
                                let at = egui::Rect::from_min_size(
                                    egui::pos2(rect.right() - 120.0, rect.center().y - 22.0),
                                    egui::vec2(110.0, 45.0),
                                );
                                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(at));
                                if theme::button(&mut child, Kind::Primary, "Open", !busy).clicked()
                                {
                                    open = Some(project.path.clone());
                                }
                            }
                        }
                    });
            });
        if let Some(path) = open {
            self.open_project(path);
        }
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
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_parent)
                            .hint_text("Choose a folder with Browse…")
                            .desired_width(ui.available_width() - 130.0)
                            .min_size(egui::vec2(0.0, 42.0))
                            .align(egui::Align2::LEFT_CENTER)
                            .margin(egui::Margin::symmetric(12, 8))
                            .char_limit(MAX_FIELD),
                    );
                    if theme::button(ui, Kind::Secondary, "Browse…", !busy).clicked()
                        && let Some(dir) = self.browse("Where to create the model")
                    {
                        self.new_parent = dir;
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
pub const LAUNCHER_SIZE: egui::Vec2 = egui::vec2(500.0, 640.0);
/// Height of the profile card at the bottom of the start page's side panel.
const PROFILE_CARD_HEIGHT: f32 = 190.0;
/// Height of one row of the model list; tall enough for a model preview.
const ROW_HEIGHT: f32 = 104.0;

fn field(ui: &mut egui::Ui, text: &mut String, limit: usize) {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(ui.available_width())
            .min_size(egui::vec2(0.0, 42.0))
            .align(egui::Align2::LEFT_CENTER)
            .margin(egui::Margin::symmetric(12, 8))
            .char_limit(limit),
    );
}

fn nav_item(
    ui: &mut egui::Ui,
    label: &str,
    icon: Icon,
    count: &str,
    active: bool,
) -> egui::Response {
    let palette = Palette::current(ui);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width() - 4.0, 46.0),
        egui::Sense::click(),
    );
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(10);
    if active {
        painter.rect_filled(rect.translate(egui::vec2(3.0, 3.0)), radius, palette.ink);
        painter.rect_filled(rect, radius, palette.surface);
        painter.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(2.0, palette.ink),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, radius, palette.side_grid);
    }
    let text = if active {
        palette.text
    } else {
        palette.side_text()
    };
    theme::paint_icon(
        painter,
        egui::Rect::from_center_size(
            rect.left_center() + egui::vec2(24.0, 0.0),
            egui::Vec2::splat(24.0),
        ),
        icon,
        if active { palette.line } else { theme::INK },
    );
    painter.text(
        rect.left_center() + egui::vec2(48.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::new(14.0, theme::bold_family()),
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
    painter.rect_filled(pill, egui::CornerRadius::same(255), palette.surface);
    painter.rect_stroke(
        pill,
        egui::CornerRadius::same(255),
        egui::Stroke::new(1.5, palette.ink),
        egui::StrokeKind::Inside,
    );
    painter.galley(pill.center() - badge.size() / 2.0, badge, palette.text);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn template_card(ui: &mut egui::Ui, title: &str, text: &str, available: bool) {
    let palette = Palette::current(ui);
    let frame = if available {
        theme::card(palette, 5).corner_radius(egui::CornerRadius::same(14))
    } else {
        egui::Frame::new()
            .fill(palette.head)
            .stroke(egui::Stroke::new(2.0, palette.rule))
            .corner_radius(egui::CornerRadius::same(14))
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

    #[test]
    fn reads_settings_replies() {
        let out = br#"{"config_dir":"/c","outcome":"settings","settings":{"environment":"blank","models_dir":null,"profile":{"actor_id":"0199e7a0-0000-7000-8000-000000000001","email":"a@b.cl","name":"Ana"},"recent":[],"skip_launcher":true,"version":1}}"#;
        let s = parse_settings(Some(0), out).unwrap();
        assert_eq!(s.environment.as_deref(), Some("blank"));
        assert_eq!(s.profile.unwrap().name, "Ana");
        assert!(s.skip_launcher && s.models_dir.is_none());
        let saved = String::from_utf8(out.to_vec())
            .unwrap()
            .replace("\"settings\",", "\"settings_saved\",");
        assert!(parse_settings(Some(0), saved.as_bytes()).is_ok());
        let refused = br#"{"code":"INVALID_ARGUMENT","diagnostics":[],"message":"bad e-mail","observed_context":null,"outcome":"rejected"}"#;
        let e = parse_settings(Some(2), refused).unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("INVALID_ARGUMENT", "bad e-mail")
        );
    }

    #[test]
    fn reads_environments_and_projects() {
        let out = br#"{"environments":[{"id":"blank","origin":"builtin","path":"/e","refused":null,"shadowed":false,"title":"Blank"},{"id":null,"origin":"user","path":"/u","refused":{"code":"INVALID_ARGUMENT","message":"no id"},"shadowed":false,"title":null}],"outcome":"environments","user_dir":null}"#;
        let envs = parse_environments(Some(0), out).unwrap();
        assert_eq!(envs[0].label(), "Blank (blank)");
        assert_eq!(envs[0].usable_id(), Some("blank"));
        assert_eq!(envs[1].usable_id(), None);
        assert!(envs[1].refused.as_deref().unwrap().contains("no id"));
        let out = br#"{"dir":"/m","outcome":"projects","projects":[{"created_ms":1792799652530,"modified_ms":1792799652605,"name":"A","path":"/m/A","project_id":"x","revision":null,"state":"ready"},{"created_ms":null,"modified_ms":null,"name":"B","path":"/m/B","project_id":null,"revision":null,"state":{"unreadable":"INTEGRITY_ERROR"}}],"truncated":false}"#;
        let projects = parse_projects(Some(0), out).unwrap();
        assert!(projects[0].ready());
        assert_eq!(projects[1].state, "INTEGRITY_ERROR");
        assert!(!projects[1].ready());
    }

    #[test]
    fn builds_arguments() {
        let args = project_create_args(Some(Path::new("/c")), "/m", "Nave 1", Some("id"));
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
        let args = settings_set_args(None, "Ana", "a@b.cl", "blank", false);
        assert_eq!(args.last().map(String::as_str), Some("false"));
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
