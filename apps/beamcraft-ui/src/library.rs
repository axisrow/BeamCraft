// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The model-library pieces shared by the start page and the in-model Menu.
//!
//! `NewModelForm` is the new-model form: the name, the location, the template and the
//! `create-project` command. It owns its fields, its worker and its note, so any window can host
//! it; the host draws the frame and the header around it. The next free name in the location is
//! read with `session::project::next_project_name` on a worker thread; creating a project runs
//! `create-project` in a separate process, read with `crate::start::parse_project`.
//!
//! `ModelPicker` is the menu's Open page: the recent models and "Browse…". The recent list is
//! read, and a chosen model remembered as opened, with the `session` crate on a worker thread.
//!
//! Each piece runs one job at a time and knows what kind of job it is, so a host can wait for the
//! jobs that open a model (a create, a remember) without waiting for the background reads.
//!
//! `project_facts` reads what the store's head says about the open model's project, for the
//! menu's read-only Project page; the host runs it on a worker thread.

use crate::command;
use crate::start::{
    DEADLINE, MAX_FIELD, ProjectView, format_when, load_recent, now_ms, parse_project,
    project_create_args, refusal, remember,
};
use crate::stats::Stats;
use crate::theme::{self, Icon, Kind, Palette};
use crate::tools::ToolError;
use session::project::{ProjectState, next_project_name, project_info};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

/// Initial editable model name; translate with the UI when other languages are added.
pub(crate) const DEFAULT_MODEL_NAME: &str = "New Model";

/// A one-line text field as wide as the row, at most `limit` characters.
pub(crate) fn field(ui: &mut egui::Ui, text: &mut String, limit: usize) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(ui.available_width())
            .min_size(egui::vec2(0.0, 36.0))
            .align(egui::Align2::LEFT_CENTER)
            .margin(egui::Margin::symmetric(12, 8))
            .char_limit(limit),
    )
}

/// Shows the system's folder dialog titled `title`, starting at `start`: the chosen folder,
/// `None` when it was cancelled, or the reason it could not be shown or used.
pub(crate) fn browse_folder(title: &str, start: &str) -> Result<Option<String>, String> {
    match crate::dialog::pick_folder(title, Some(Path::new(start))) {
        Ok(Some(dir)) => match dir.to_str() {
            Some(text) => Ok(Some(text.to_owned())),
            None => Err("the chosen folder's path is not UTF-8 text".into()),
        },
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// What the form needs from its host window.
pub(crate) struct Host<'a> {
    /// The CLI that runs `create-project`.
    pub(crate) cli: &'a Path,
    /// The configuration directory replacing the user's, if any.
    pub(crate) config_dir: Option<&'a Path>,
    /// The author identity recorded as the creator.
    pub(crate) actor: Option<&'a str>,
    /// The environment the new model is seeded from.
    pub(crate) environment: Option<&'a str>,
    /// The window's process statistics.
    pub(crate) stats: &'a Arc<Stats>,
}

/// What the user did with the form.
#[derive(Debug)]
pub(crate) enum FormEvent {
    /// Cancel was pressed.
    Cancelled,
    /// `create-project` created this project.
    Created(ProjectView),
}

enum FormReply {
    Suggested {
        parent: String,
        previous: String,
        name: Result<String, ToolError>,
    },
    Created(Result<ProjectView, ToolError>),
}

/// What the form's running job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormKind {
    /// Reads the next free name in the location.
    Suggesting,
    /// Runs `create-project`.
    Creating,
}

/// The new-model form: name, location, template, and the create command.
pub(crate) struct NewModelForm {
    name: String,
    suggested: String,
    parent: String,
    note: Option<Result<String, String>>,
    job: Option<(FormKind, Receiver<FormReply>)>,
    /// The sending end of a job a test installed with `hold_job` or `hold_create`.
    #[cfg(test)]
    sender: Option<std::sync::mpsc::Sender<FormReply>>,
}

impl NewModelForm {
    /// An empty form with the default name and no location.
    pub(crate) fn new() -> Self {
        Self {
            name: DEFAULT_MODEL_NAME.to_owned(),
            suggested: DEFAULT_MODEL_NAME.to_owned(),
            parent: String::new(),
            note: None,
            job: None,
            #[cfg(test)]
            sender: None,
        }
    }

    /// Sets the location to `parent` unless one was chosen or typed already.
    pub(crate) fn seed_parent(&mut self, parent: &str) {
        if self.parent.is_empty() {
            parent.clone_into(&mut self.parent);
        }
    }

    /// The location as typed.
    pub(crate) fn parent(&self) -> &str {
        &self.parent
    }

    /// Whether a suggestion or a create command is running.
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Whether a create command is running; a name suggestion does not count.
    pub(crate) fn creating(&self) -> bool {
        matches!(self.job, Some((FormKind::Creating, _)))
    }

    /// Prepares the form to be shown: clears its note and suggests a name.
    pub(crate) fn show(&mut self) {
        self.note = None;
        self.suggest();
    }

    /// Asks for the next free model name in the location, unless the name was edited, no
    /// location is set or a job is running.
    pub(crate) fn suggest(&mut self) {
        if self.job.is_some() || self.name != self.suggested || self.parent.trim().is_empty() {
            return;
        }
        let parent = self.parent.trim().to_owned();
        let previous = self.name.clone();
        self.run_worker(FormKind::Suggesting, move || {
            let name = next_project_name(Path::new(&parent), DEFAULT_MODEL_NAME).map_err(refusal);
            FormReply::Suggested {
                parent,
                previous,
                name,
            }
        });
    }

    /// Runs `create-project` for the form's name and location.
    pub(crate) fn create(&mut self, host: &Host<'_>) {
        // A typed location is checked here, so the refusal names the path that was typed.
        let parent = self.parent.trim();
        if !Path::new(parent).is_dir() {
            self.note = Some(Err(format!(
                "The folder \"{parent}\" does not exist. Choose an existing folder with Browse…"
            )));
            return;
        }

        let args = project_create_args(
            host.config_dir,
            self.parent.trim(),
            if self.name == self.suggested {
                DEFAULT_MODEL_NAME
            } else {
                self.name.trim()
            },
            host.actor,
            host.environment,
        );
        let cli = host.cli.to_path_buf();
        let stats = Arc::clone(host.stats);
        self.run_worker(FormKind::Creating, move || {
            FormReply::Created(
                command::run(&cli, &args, DEADLINE, &stats)
                    .and_then(|out| parse_project(out.code, &out.stdout, "project_created")),
            )
        });
    }

    /// Reads the reply of the running job, if it has arrived: a created project is the event,
    /// a suggestion or a failure only changes the form.
    pub(crate) fn poll(&mut self) -> Option<FormEvent> {
        let (_, rx) = self.job.as_ref()?;
        let reply = match rx.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                self.note = Some(Err("the command thread ended without an answer".into()));
                return None;
            }
        };
        self.job = None;
        match reply {
            FormReply::Suggested {
                parent,
                previous,
                name,
            } => {
                if self.parent.trim() == parent
                    && self.name == previous
                    && let Ok(name) = name
                {
                    self.name = name.clone();
                    self.suggested = name;
                }
                None
            }
            FormReply::Created(Ok(project)) => Some(FormEvent::Created(project)),
            FormReply::Created(Err(e)) => {
                self.note = Some(Err(e.to_string()));
                None
            }
        }
    }

    /// Draws the form's fields and buttons, without a frame or a header; `outer_busy` disables
    /// them while the host runs a command of its own.
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        host: &Host<'_>,
        outer_busy: bool,
    ) -> Option<FormEvent> {
        let mut event = self.poll();
        if self.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        let busy = self.busy() || outer_busy;
        let palette = Palette::current(ui);
        ui.label(theme::bold("Name", 14.0));
        let name_field = field(ui, &mut self.name, 100);
        ui.label(
            egui::RichText::new("Also the name of the model's folder.")
                .small()
                .color(palette.muted),
        );
        ui.add_space(8.0);
        ui.label(theme::bold("Location", 14.0));
        let location = |ui: &mut egui::Ui| -> bool {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.parent)
                    .hint_text("Choose a folder with Browse…")
                    .desired_width(ui.available_width() - 130.0)
                    .min_size(egui::vec2(0.0, 42.0))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(egui::Margin::symmetric(12, 8))
                    .char_limit(MAX_FIELD),
            );
            let changed = response.changed();
            if response.lost_focus() && !outer_busy {
                self.suggest();
            }
            if theme::button(ui, Kind::Secondary, "Browse…", !busy).clicked() {
                match browse_folder("Where to create the model", self.parent.trim()) {
                    Ok(Some(dir)) => {
                        self.parent = dir;
                        self.suggest();
                    }
                    Ok(None) => {}
                    Err(e) => self.note = Some(Err(e)),
                }
            }
            changed
        };
        let location_changed = ui.horizontal(location).inner;
        // An error names what was typed; editing either field drops it, so a cleared name
        // never keeps the previous refusal on show (issue #562).
        if name_field.changed() || location_changed {
            self.note = None;
        }
        ui.add_space(8.0);
        ui.label(theme::bold("Template", 14.0));
        ui.horizontal(|ui| {
            template_card(ui, "Blank", "Empty: no grids or levels.", true);
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
            let ready = !self.name.trim().is_empty() && !self.parent.trim().is_empty();
            if theme::button(ui, Kind::Primary, "Create and open", ready && !busy).clicked() {
                self.create(host);
            }
            if theme::button(ui, Kind::Secondary, "Cancel", !busy).clicked() && event.is_none() {
                event = Some(FormEvent::Cancelled);
            }
            if busy {
                ui.spinner();
            }
        });
        match &self.note {
            Some(Ok(note)) => {
                ui.label(egui::RichText::new(note).color(palette.muted));
            }
            Some(Err(error)) => {
                ui.label(egui::RichText::new(error).color(palette.error).strong());
            }
            None => {}
        }
        event
    }

    /// Runs `work` of `kind` on a worker thread; the form polls for its reply. A thread that
    /// cannot start installs no job, so the form is never left busy.
    fn run_worker(&mut self, kind: FormKind, work: impl FnOnce() -> FormReply + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-start".into())
            .spawn(move || {
                let _ = tx.send(work());
            });
        if spawned.is_ok() {
            self.job = Some((kind, rx));
            self.note = None;
        } else {
            self.note = Some(Err("the command thread could not be started".into()));
        }
    }

    /// Installs a pending job of `kind` whose reply the test delivers.
    #[cfg(test)]
    fn hold(&mut self, kind: FormKind) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.job = Some((kind, rx));
        self.sender = Some(tx);
    }

    /// Installs a pending name suggestion whose reply the test delivers.
    #[cfg(test)]
    pub(crate) fn hold_job(&mut self) {
        self.hold(FormKind::Suggesting);
    }

    /// Installs a pending create command whose reply the test delivers.
    #[cfg(test)]
    pub(crate) fn hold_create(&mut self) {
        self.hold(FormKind::Creating);
    }

    /// Delivers a created project to the job installed with `hold_job` or `hold_create`.
    #[cfg(test)]
    pub(crate) fn deliver_created(&mut self, project: ProjectView) {
        if let Some(tx) = &self.sender {
            let _ = tx.send(FormReply::Created(Ok(project)));
        }
    }

    /// Delivers a name suggestion to the job installed with `hold_job`.
    #[cfg(test)]
    pub(crate) fn deliver_suggested(
        &mut self,
        parent: &str,
        previous: &str,
        name: Result<String, ToolError>,
    ) {
        if let Some(tx) = &self.sender {
            let _ = tx.send(FormReply::Suggested {
                parent: parent.to_owned(),
                previous: previous.to_owned(),
                name,
            });
        }
    }

    /// Ends the job installed with `hold_job` without an answer.
    #[cfg(test)]
    pub(crate) fn disconnect(&mut self) {
        self.sender = None;
    }
}

/// What the user did with the picker.
#[derive(Debug)]
pub(crate) enum PickEvent {
    /// Cancel was pressed.
    Cancelled,
    /// This model was chosen and remembered as opened now.
    Opened(PathBuf),
}

enum PickReply {
    Listed(Result<Vec<ProjectView>, ToolError>),
    Opened(Result<ProjectView, ToolError>),
}

/// What the picker's running job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickKind {
    /// Reads the recent list.
    Listing,
    /// Remembers the chosen model as opened now.
    Opening,
}

/// The menu's Open page: the recent models and "Browse…", read and remembered on a worker.
pub(crate) struct ModelPicker {
    recent: Option<Result<Vec<ProjectView>, String>>,
    note: Option<String>,
    job: Option<(PickKind, Receiver<PickReply>)>,
    /// The sending end of a job a test installed with `hold_open`.
    #[cfg(test)]
    sender: Option<std::sync::mpsc::Sender<PickReply>>,
}

impl ModelPicker {
    /// A picker that has not read the recent list yet.
    pub(crate) fn new() -> Self {
        Self {
            recent: None,
            note: None,
            job: None,
            #[cfg(test)]
            sender: None,
        }
    }

    /// Whether the recent list is being read or a model is being remembered.
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Whether a chosen model is being remembered; reading the recent list does not count.
    pub(crate) fn opening(&self) -> bool {
        matches!(self.job, Some((PickKind::Opening, _)))
    }

    /// Reads the recent list again, as the page is entered. Nothing happens while a job runs,
    /// so a pending remember is never replaced. Reading the list may rewrite the settings file:
    /// it drops models whose folder has gone.
    pub(crate) fn refresh(&mut self, config_dir: Option<&Path>) {
        if self.job.is_some() {
            return;
        }
        let config = config_dir.map(Path::to_path_buf);
        self.run_worker(PickKind::Listing, move || {
            PickReply::Listed(load_recent(config.as_deref()))
        });
    }

    /// Remembers the model at `path` as opened now; its reply is the event.
    fn open(&mut self, config_dir: Option<&Path>, path: String) {
        if self.job.is_some() {
            return;
        }
        let config = config_dir.map(Path::to_path_buf);
        self.run_worker(PickKind::Opening, move || {
            PickReply::Opened(remember(config.as_deref(), &path))
        });
    }

    /// Reads the reply of the running job, if it has arrived: a remembered model is the event, a
    /// list or a refusal only changes the page.
    pub(crate) fn poll(&mut self) -> Option<PickEvent> {
        let (_, rx) = self.job.as_ref()?;
        let reply = match rx.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                self.note = Some("the command thread ended without an answer".into());
                return None;
            }
        };
        self.job = None;
        match reply {
            PickReply::Listed(listed) => {
                self.recent = Some(listed.map_err(|e| e.to_string()));
                None
            }
            PickReply::Opened(Ok(project)) => Some(PickEvent::Opened(PathBuf::from(project.path))),
            PickReply::Opened(Err(e)) => {
                self.note = Some(e.to_string());
                None
            }
        }
    }

    /// Draws the recent models, "Browse…" and Cancel. `current` is the open model, shown as
    /// "Open now"; `outer_busy` disables the page while the host runs a command of its own.
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        config_dir: Option<&Path>,
        current: &Path,
        outer_busy: bool,
    ) -> Option<PickEvent> {
        let mut event = self.poll();
        if self.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        let busy = self.busy() || outer_busy;
        let palette = Palette::current(ui);
        let now = now_ms();
        ui.label(theme::bold("Recent models", 14.0));
        ui.add_space(8.0);
        let mut chosen = None;
        match &self.recent {
            Some(Ok(projects)) if projects.is_empty() => {
                ui.label(egui::RichText::new("No recent models.").color(palette.muted));
            }
            Some(Ok(projects)) => {
                egui::ScrollArea::vertical()
                    .id_salt("menu-recent-models")
                    .max_height(360.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for project in projects {
                            let open_now = Path::new(&project.path) == current;
                            let enabled = project.ready() && !open_now && !busy;
                            if model_row(ui, project, open_now, enabled, now, palette).clicked() {
                                chosen = Some(project.path.clone());
                            }
                        }
                    });
            }
            Some(Err(error)) => {
                ui.label(egui::RichText::new(error).color(palette.error));
            }
            None => {}
        }
        if let Some(path) = chosen {
            self.open(config_dir, path);
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if theme::icon_text_button(ui, Icon::Folder, "Browse…", !busy, 140.0).clicked() {
                let start = current
                    .parent()
                    .and_then(Path::to_str)
                    .unwrap_or_default()
                    .to_owned();
                match browse_folder("Open a BeamCraft model folder", &start) {
                    Ok(Some(dir)) => self.open(config_dir, dir),
                    Ok(None) => {}
                    Err(e) => self.note = Some(e),
                }
            }
            if theme::button(ui, Kind::Secondary, "Cancel", !busy).clicked() && event.is_none() {
                event = Some(PickEvent::Cancelled);
            }
            if self.busy() {
                ui.spinner();
            }
        });
        if let Some(note) = &self.note {
            ui.label(egui::RichText::new(note).color(palette.error).strong());
        }
        event
    }

    /// Runs `work` of `kind` on a worker thread; the picker polls for its reply. A thread that
    /// cannot start installs no job, so the picker is never left busy.
    fn run_worker(&mut self, kind: PickKind, work: impl FnOnce() -> PickReply + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-start".into())
            .spawn(move || {
                let _ = tx.send(work());
            });
        if spawned.is_ok() {
            self.job = Some((kind, rx));
            self.note = None;
        } else {
            self.note = Some("the command thread could not be started".into());
        }
    }

    /// Shows `projects` as the recent list, with no job pending.
    #[cfg(test)]
    pub(crate) fn set_recent(&mut self, projects: Vec<ProjectView>) {
        self.recent = Some(Ok(projects));
        self.job = None;
        self.sender = None;
    }

    /// Installs a pending remember whose reply the test delivers.
    #[cfg(test)]
    pub(crate) fn hold_open(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.job = Some((PickKind::Opening, rx));
        self.sender = Some(tx);
    }

    /// Delivers the answer of the remember installed with `hold_open`.
    #[cfg(test)]
    pub(crate) fn deliver_opened(&mut self, project: Result<ProjectView, ToolError>) {
        if let Some(tx) = &self.sender {
            let _ = tx.send(PickReply::Opened(project));
        }
    }
}

/// One recent model as a full-width row: its name, its folder below and when it changed (or
/// "Open now" for the open model) on the right.
fn model_row(
    ui: &mut egui::Ui,
    project: &ProjectView,
    open_now: bool,
    enabled: bool,
    now: u64,
    palette: Palette,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("").min_size(egui::vec2(ui.available_width(), 54.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, enabled, project.name.as_str())
    });
    let rect = response.rect;
    let painter = ui.painter();
    let right = if open_now {
        "Open now".to_owned()
    } else {
        project
            .modified_ms
            .map(|ms| format!("Changed {}", format_when(ms, now)))
            .unwrap_or_default()
    };
    let side = painter.layout_no_wrap(right, egui::FontId::proportional(11.0), palette.muted);
    let side_width = side.size().x;
    painter.galley(
        egui::pos2(
            rect.right() - 12.0 - side_width,
            rect.center().y - side.size().y / 2.0,
        ),
        side,
        palette.muted,
    );
    // Name and folder stop before the text on the right.
    let names = painter.with_clip_rect(egui::Rect::from_x_y_ranges(
        rect.left()..=(rect.right() - 24.0 - side_width).max(rect.left()),
        rect.y_range(),
    ));
    names.text(
        egui::pos2(rect.left() + 12.0, rect.center().y - 9.0),
        egui::Align2::LEFT_CENTER,
        &project.name,
        egui::FontId::new(13.0, theme::bold_family()),
        if enabled || open_now {
            palette.text
        } else {
            palette.muted
        },
    );
    let (meta, color) = if project.ready() {
        (
            Path::new(&project.path)
                .parent()
                .and_then(Path::to_str)
                .unwrap_or_default()
                .to_owned(),
            palette.muted,
        )
    } else {
        (
            format!("cannot be opened: {}", project.state),
            palette.error,
        )
    };
    names.text(
        egui::pos2(rect.left() + 12.0, rect.center().y + 11.0),
        egui::Align2::LEFT_CENTER,
        meta,
        egui::FontId::proportional(11.0),
        color,
    );
    response
}

/// What the store's head says about the project, as the Project page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectFacts {
    /// The folder name; the project's name.
    pub(crate) name: String,
    /// The store directory.
    pub(crate) path: String,
    /// `ready`, `empty`, `missing` or the code of an unreadable store.
    pub(crate) state: String,
    /// The ProjectId the head records.
    pub(crate) project_id: Option<String>,
    /// Creation time, Unix milliseconds: the timestamp of the ProjectId.
    pub(crate) created_ms: Option<u64>,
    /// Time of the last change, Unix milliseconds: the timestamp of the head revision id.
    pub(crate) modified_ms: Option<u64>,
}

/// Reads the project of `store` (`session::project::project_info`); nothing is written.
pub(crate) fn project_facts(store: &Path) -> Result<ProjectFacts, String> {
    let info = project_info(store).map_err(|e| format!("{}: {}", e.code.as_str(), e.message))?;
    let (state, project_id, created_ms, modified_ms) = match info.state {
        ProjectState::Ready {
            project_id,
            created_ms,
            modified_ms,
            ..
        } => (
            "ready".to_owned(),
            Some(project_id),
            Some(created_ms),
            Some(modified_ms),
        ),
        ProjectState::Empty => ("empty".to_owned(), None, None, None),
        ProjectState::Missing => ("missing".to_owned(), None, None, None),
        ProjectState::Unreadable(code) => (code.as_str().to_owned(), None, None, None),
    };
    Ok(ProjectFacts {
        name: info.name,
        path: info.path.to_string_lossy().into_owned(),
        state,
        project_id,
        created_ms,
        modified_ms,
    })
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use std::path::PathBuf;

    fn host_parts() -> (PathBuf, Arc<Stats>) {
        (
            PathBuf::from("missing-test-cli"),
            Arc::new(Stats::default()),
        )
    }

    #[test]
    fn new_model_entry_suggests_a_name_and_keeps_user_edits_during_the_reply() {
        let mut form = NewModelForm::new();
        form.show();
        assert!(form.job.is_none());
        assert_eq!(form.name, DEFAULT_MODEL_NAME);
        form.hold_job();
        form.name = "My structure".into();
        let parent = form.parent.clone();
        form.deliver_suggested(&parent, DEFAULT_MODEL_NAME, Ok("New model 3".into()));
        assert!(form.poll().is_none());
        assert_eq!(form.name, "My structure");
        assert!(form.job.is_none());
    }

    #[test]
    fn a_suggestion_replaces_an_untouched_name() {
        let mut form = NewModelForm::new();
        form.seed_parent("/m");
        form.seed_parent("/other");
        assert_eq!(form.parent(), "/m");
        form.hold_job();
        form.deliver_suggested("/m", DEFAULT_MODEL_NAME, Ok("New Model 3".into()));
        assert!(form.poll().is_none());
        assert_eq!(form.name, "New Model 3");
        assert_eq!(form.suggested, "New Model 3");
    }

    #[test]
    fn a_missing_location_is_refused_without_a_command() {
        let (cli, stats) = host_parts();
        let host = Host {
            cli: &cli,
            config_dir: None,
            actor: None,
            environment: None,
            stats: &stats,
        };
        let mut form = NewModelForm::new();
        form.parent = std::env::temp_dir()
            .join(format!("beamcraft-library-missing-{}", std::process::id()))
            .to_string_lossy()
            .into_owned();
        form.create(&host);
        assert!(form.job.is_none());
        let note = form.note.clone().unwrap().unwrap_err();
        assert!(note.contains("does not exist"), "{note}");
    }

    #[test]
    fn create_marks_the_form_as_creating() {
        let (cli, stats) = host_parts();
        let host = Host {
            cli: &cli,
            config_dir: None,
            actor: None,
            environment: None,
            stats: &stats,
        };
        let parent =
            std::env::temp_dir().join(format!("beamcraft-library-creating-{}", std::process::id()));
        std::fs::create_dir_all(&parent).unwrap();
        let mut form = NewModelForm::new();
        form.parent = parent.to_string_lossy().into_owned();
        assert!(!form.creating());
        form.create(&host);
        // The missing CLI fails at once, but the job is installed before any reply is read.
        assert!(form.creating());
        for _ in 0..250 {
            if form.poll().is_some() || !form.busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!form.creating());
        assert!(matches!(form.note, Some(Err(_))), "{:?}", form.note);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn a_suggestion_is_not_a_create() {
        let mut form = NewModelForm::new();
        form.hold_job();
        assert!(form.busy() && !form.creating());
        form.hold_create();
        assert!(form.busy() && form.creating());
    }

    #[test]
    fn an_opened_reply_reports_the_path() {
        let mut picker = ModelPicker::new();
        picker.hold_open();
        assert!(picker.opening());
        picker.deliver_opened(Ok(ProjectView {
            name: "Nave 1".into(),
            path: "/m/Nave 1".into(),
            state: "ready".into(),
            created_ms: Some(0),
            modified_ms: Some(0),
        }));
        assert!(
            matches!(picker.poll(), Some(PickEvent::Opened(path)) if path == Path::new("/m/Nave 1"))
        );
        assert!(!picker.busy());
        assert_eq!(picker.note, None);
    }

    #[test]
    fn a_refused_open_is_a_note() {
        let mut picker = ModelPicker::new();
        picker.hold_open();
        picker.deliver_opened(Err(ToolError::local(
            "NOT_FOUND",
            "the folder is not a project with a readable head",
        )));
        assert!(picker.poll().is_none());
        assert!(!picker.opening());
        assert_eq!(
            picker.note.as_deref(),
            Some("the folder is not a project with a readable head")
        );
    }

    #[test]
    fn a_refresh_never_replaces_a_pending_open() {
        let mut picker = ModelPicker::new();
        picker.hold_open();
        let scratch = std::env::temp_dir().join(format!(
            "beamcraft-library-picker-config-{}",
            std::process::id()
        ));
        picker.refresh(Some(&scratch));
        assert!(picker.opening());
    }

    #[test]
    fn a_disconnected_worker_is_a_note() {
        let mut form = NewModelForm::new();
        form.hold_job();
        form.disconnect();
        assert!(form.poll().is_none());
        assert!(!form.busy());
        assert_eq!(
            form.note,
            Some(Err("the command thread ended without an answer".into()))
        );
    }

    #[test]
    fn a_created_reply_is_the_event() {
        let mut form = NewModelForm::new();
        form.hold_job();
        form.deliver_created(ProjectView {
            name: "Nave 1".into(),
            path: "/m/Nave 1".into(),
            state: "ready".into(),
            created_ms: Some(0),
            modified_ms: Some(0),
        });
        assert!(
            matches!(form.poll(), Some(FormEvent::Created(project)) if project.path == "/m/Nave 1")
        );
        assert!(!form.busy());
    }

    /// A fresh scratch folder of this test.
    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "beamcraft-library-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_folder_without_a_store_reads_as_missing() {
        let dir = scratch("facts-missing");
        let facts = project_facts(&dir).unwrap();
        assert_eq!(facts.state, "missing");
        assert_eq!(facts.project_id, None);
        assert_eq!(facts.created_ms, None);
        assert_eq!(facts.modified_ms, None);
        assert_eq!(
            Some(facts.name.as_str()),
            dir.file_name().and_then(|n| n.to_str())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_store_without_revisions_reads_as_empty() {
        let dir = scratch("facts-empty");
        std::fs::create_dir_all(dir.join("revisions")).unwrap();
        std::fs::create_dir_all(dir.join("blobs")).unwrap();
        let facts = project_facts(&dir).unwrap();
        assert_eq!(facts.state, "empty");
        assert_eq!(facts.project_id, None);
        assert_eq!(facts.created_ms, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn template_cards_share_one_height() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let (cli, stats) = host_parts();
        let host = Host {
            cli: &cli,
            config_dir: None,
            actor: None,
            environment: None,
            stats: &stats,
        };
        let mut form = NewModelForm::new();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                let _ = form.ui(ui, &host, false);
            },
        );
        // One card frame per template, drawn as a filled stroked rectangle ~230 wide.
        let mut heights: Vec<f32> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Rect(rect) = &shape.shape
                    && (220.0..=240.0).contains(&rect.rect.width())
                    && rect.rect.height() > 150.0
                {
                    Some(rect.rect.height())
                } else {
                    None
                }
            })
            .collect();
        output.drop_without_applying_deltas();
        heights.sort_by(|a, b| a.total_cmp(b));
        heights.dedup_by(|a, b| (*a - *b).abs() < 1.0);
        assert_eq!(
            heights,
            vec![heights.first().copied().unwrap_or(0.0)],
            "the template cards must share one height: {heights:?}"
        );
    }

    #[test]
    fn editing_a_field_drops_the_previous_refusal() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let (cli, stats) = host_parts();
        let host = Host {
            cli: &cli,
            config_dir: None,
            actor: None,
            environment: None,
            stats: &stats,
        };
        let mut form = NewModelForm::new();
        form.note = Some(Err("The folder \"/gone\" does not exist.".into()));
        let mut field = None;
        let mut frame = |events: Vec<egui::Event>| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let _ = form.ui(ui, &host, false);
                },
            );
            if field.is_none() {
                // The name field shows the default name until it is typed into.
                field = output.shapes.iter().find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text() == DEFAULT_MODEL_NAME
                    {
                        return Some(text.galley.rect.translate(text.pos.to_vec2()).center());
                    }
                    None
                });
            }
            output.drop_without_applying_deltas();
            field
        };
        let field = frame(vec![]).expect("the name field is drawn");
        let click = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![
            egui::Event::PointerMoved(field),
            click(field, true),
            click(field, false),
        ]);
        frame(vec![egui::Event::Text("x".into())]);
        assert!(
            form.note.is_none(),
            "editing the name drops the old refusal"
        );
        assert!(form.name.contains('x') && form.name != DEFAULT_MODEL_NAME);
    }

    #[test]
    fn cancel_reports_cancelled() {
        let ctx = egui::Context::default();
        assert!(crate::app::inter(&ctx));
        theme::install(&ctx);
        let (cli, stats) = host_parts();
        let host = Host {
            cli: &cli,
            config_dir: None,
            actor: None,
            environment: None,
            stats: &stats,
        };
        let mut form = NewModelForm::new();
        let mut frame = |events: Vec<egui::Event>| {
            let mut event = None;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| event = form.ui(ui, &host, false),
            );
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some((
                            text.galley.text().to_owned(),
                            text.pos + text.galley.size() / 2.0,
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            output.drop_without_applying_deltas();
            (labels, event)
        };
        let (labels, event) = frame(vec![]);
        assert!(event.is_none());
        let cancel = labels.iter().find(|(text, _)| text == "Cancel").unwrap().1;
        let (_, event) = frame(vec![
            egui::Event::PointerMoved(cancel),
            egui::Event::PointerButton {
                pos: cancel,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: cancel,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(matches!(event, Some(FormEvent::Cancelled)), "{event:?}");
    }
}
