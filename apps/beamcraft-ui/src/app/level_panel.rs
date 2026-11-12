// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The levels bench: the levels of the displayed revision, a form that creates several levels
//! from one spacing expression, and one that edits the selected level, through the engine
//! commands. The list is the pinned `definitions` reply, so it follows every revision, undo and
//! redo included.

use super::App;
use crate::levels::{self, LevelForm, NewLevelsForm};
use crate::theme;
use crate::tools::{FormError, LevelChoice, ToolError};
use egui::Ui;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// What a running command will report when it succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gesture {
    Create,
    Update,
    Delete,
}

impl Gesture {
    const fn done(self) -> &'static str {
        match self {
            Self::Create => "Levels created.",
            Self::Update => "Level updated.",
            Self::Delete => "Level deleted.",
        }
    }
}

type Job = Receiver<Result<usize, ToolError>>;

/// The state of the levels bench.
#[derive(Default)]
pub(super) struct LevelPanel {
    form: LevelForm,
    /// The new levels, shown when no level is selected.
    batch: NewLevelsForm,
    /// The identity of the level being edited; new levels otherwise.
    selected: Option<String>,
    job: Option<(Gesture, Job)>,
    pub(super) note: Option<Result<String, String>>,
}

impl LevelPanel {
    pub(super) const fn running(&self) -> bool {
        self.job.is_some()
    }

    pub(super) fn selected_id(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    fn select(&mut self, level: &LevelChoice) {
        self.form = LevelForm::of(level);
        self.selected = Some(level.id.clone());
        self.note = None;
    }

    fn clear(&mut self) {
        self.form = LevelForm::default();
        self.selected = None;
    }
}

impl App {
    fn selected_level(&self) -> Option<LevelChoice> {
        let id = self.level_panel.selected.as_ref()?;
        self.tools
            .definitions
            .levels
            .iter()
            .find(|level| &level.id == id)
            .cloned()
    }

    pub(super) fn level_panel(&mut self, ui: &mut Ui) {
        // An undo can remove the level being edited.
        let selected = self.selected_level();
        if selected.is_none() && self.level_panel.selected.is_some() {
            self.level_panel.clear();
        }
        let busy = self.other_command_running();
        crate::kit::type_box(ui, theme::Icon::Table, "Levels");
        let all = self.tools.definitions.levels.clone();
        if all.is_empty() {
            ui.label("No levels yet. Enter their elevations below.");
        }
        // The form below keeps its height; a new batch adds its preview.
        let footer = if selected.is_some() { 230.0 } else { 330.0 };
        let mut picked = None;
        egui::ScrollArea::vertical()
            .id_salt("level-list")
            .max_height((ui.available_height() - footer).max(0.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for level in levels::sorted(&all) {
                    let chosen = selected.as_ref().is_some_and(|s| s.id == level.id);
                    let row = format!("{}   {:+} mm", level.name, level.elevation_mm);
                    if ui.selectable_label(chosen, row).clicked() {
                        picked = Some(level.clone());
                    }
                }
            });
        if let Some(level) = picked {
            self.level_panel.select(&level);
        }
        ui.separator();
        match &selected {
            Some(level) => self.level_edit_form(ui, level, busy),
            None => self.new_levels_form(ui, &all, busy),
        }
    }

    /// The form of the selected level: its name and elevation, and Delete.
    fn level_edit_form(&mut self, ui: &mut Ui, level: &LevelChoice, busy: bool) {
        ui.add_enabled_ui(!busy, |ui| {
            crate::kit::prop_row(ui, "Name", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.level_panel.form.name)
                        .hint_text("Level name")
                        .char_limit(256),
                );
            });
            crate::kit::prop_row(ui, "Elevation (mm)", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.level_panel.form.elevation)
                        .hint_text("0")
                        .char_limit(64),
                );
            });
            ui.checkbox(
                &mut self.level_panel.form.move_elements,
                "Beams and columns on the level follow it",
            )
            .on_hover_text(
                "Axis ends of straight beams and columns at the old elevation move to the new \
                 one in the same undo step.",
            );
            if ui.button("Delete level").clicked() {
                let prepared =
                    levels::prepare_delete(level, &self.source.store, self.source.actor.as_deref());
                self.start_level(Gesture::Delete, prepared);
            }
        });
        let enabled = self.level_panel.form.differs(level);
        let done = crate::kit::actions(ui, "Apply changes", enabled, "New level", busy);
        if done.primary {
            let prepared = self.level_panel.form.prepare_update(
                level,
                &self.source.store,
                self.source.actor.as_deref(),
            );
            self.start_level(Gesture::Update, prepared);
        }
        if done.cancel {
            self.level_panel.note = None;
            self.level_panel.clear();
        }
    }

    /// The form of new levels: the elevations as a spacing expression, the names, and a preview
    /// of what Create makes.
    fn new_levels_form(&mut self, ui: &mut Ui, existing: &[LevelChoice], busy: bool) {
        ui.add_enabled_ui(!busy, |ui| {
            crate::kit::prop_row(ui, "Elevations (mm)", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.level_panel.batch.elevations)
                        .hint_text("0 4*3000")
                        .char_limit(4096),
                );
            });
            ui.small("The first elevation, then spacings: -1000 1000 6*3500.");
            crate::kit::prop_row(ui, "Names", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.level_panel.batch.names)
                        .hint_text("Generated: Level 1, Level 2, …")
                        .char_limit(4096),
                );
            });
            ui.small("Names separated by commas; the rest continue from the last one.");
        });
        let preview = if self.level_panel.batch.elevations.trim().is_empty() {
            None
        } else {
            Some(self.level_panel.batch.preview(existing))
        };
        match &preview {
            Some(Ok(rows)) => {
                ui.label(format!(
                    "{} new level{}:",
                    rows.len(),
                    if rows.len() == 1 { "" } else { "s" }
                ));
                egui::ScrollArea::vertical()
                    .id_salt("level-preview")
                    .max_height(110.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (name, elevation_mm) in rows.iter().rev() {
                            ui.small(format!("{name}   {elevation_mm:+} mm"));
                        }
                    });
            }
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
            }
            None => {}
        }
        let enabled = matches!(preview, Some(Ok(_)));
        let label = match &preview {
            Some(Ok(rows)) if rows.len() > 1 => "Create levels",
            _ => "Create level",
        };
        let done = crate::kit::actions(ui, label, enabled, "Close", busy);
        if done.primary {
            let prepared = self
                .level_panel
                .batch
                .prepare(&self.source.store, self.source.actor.as_deref());
            self.start_level(Gesture::Create, prepared);
        }
        if done.cancel {
            self.level_panel.note = None;
            self.set_bench(super::workbench::Bench::Select);
        }
    }
    fn start_level(&mut self, gesture: Gesture, prepared: Result<Vec<String>, FormError>) {
        if self.other_command_running() {
            return;
        }
        let args = match prepared {
            Ok(args) => args,
            Err(error) => {
                self.level_panel.note = Some(Err(error.to_string()));
                return;
            }
        };
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("beamcraft-level".into())
            .spawn(move || {
                let _ = tx.send(levels::commit(&cli, &args, &stats));
            }) {
            Ok(_) => {
                self.level_panel.job = Some((gesture, rx));
                self.level_panel.note = None;
            }
            Err(_) => {
                self.level_panel.note = Some(Err("The level command could not be started.".into()));
            }
        }
    }

    pub(super) fn poll_levels(&mut self) {
        let Some((gesture, rx)) = &self.level_panel.job else {
            return;
        };
        let gesture = *gesture;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The level command ended without a result.",
            )),
        };
        self.level_panel.job = None;
        match result {
            Ok(moved) => {
                if gesture == Gesture::Create {
                    self.level_panel.batch = NewLevelsForm::default();
                }
                if gesture != Gesture::Update {
                    self.level_panel.clear();
                }
                let note = match moved {
                    0 => gesture.done().to_owned(),
                    1 => format!("{} One beam or column followed it.", gesture.done()),
                    n => format!("{} {n} beams and columns followed it.", gesture.done()),
                };
                self.level_panel.note = Some(Ok(note));
                self.reload();
                self.refresh_status();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                    self.refresh_status();
                }
                self.level_panel.note = Some(Err(error.to_string()));
            }
        }
    }
}
