// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The ribbon Fit end button: the part to trim, then the plate it fits to, commit through the
//! same `fit-end-to-part` command a user types (A13). The end nearer the plate is trimmed flush
//! with the plate's nearest face; a beam's end on an end plate, a column's foot on a base plate.

use super::App;
use crate::command;
use crate::edit::{Ray, pick_in};
use crate::tools::{ToolError, decode, rejection};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// The Fit end bench's state between frames.
pub(crate) struct Panel {
    /// The part to trim: a beam, a column or a plate.
    pub(crate) part: Option<String>,
    /// The plate the end is fitted to.
    pub(crate) tool: Option<String>,
    pub(super) commit_job: Option<Receiver<Result<(), ToolError>>>,
    /// The last outcome, shown in the status bar.
    pub(crate) note: Option<Result<String, String>>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self {
            part: None,
            tool: None,
            commit_job: None,
            note: None,
        }
    }

    fn busy(&self) -> bool {
        self.commit_job.is_some()
    }

    /// Forgets the picked pair.
    pub(super) fn clear_placement(&mut self) {
        self.part = None;
        self.tool = None;
    }
}

/// The `fit-end-to-part` arguments, exactly as the command takes them. `operation` is the fresh
/// identity of the fitting, which later edits and removals address.
pub(crate) fn fit_args(
    store: &Path,
    actor: Option<&str>,
    part: &str,
    tool: &str,
    operation: &str,
) -> Result<Vec<String>, String> {
    let (part, tool) = (part.trim(), tool.trim());
    if part.is_empty() || tool.is_empty() {
        return Err("Pick the part to trim, then the plate it fits to.".into());
    }
    if part == tool {
        return Err("A part cannot be fitted to itself; pick the plate.".into());
    }
    let mut args = vec![
        "fit-end-to-part".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        part.to_owned(),
        "--operation-id".into(),
        operation.to_owned(),
        "--tool".into(),
        tool.to_owned(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

impl App {
    /// The first click picks the part to trim, the second the plate. Only what the active view
    /// shows can be picked, and the selection switches apply, as they do to selection.
    pub(crate) fn fit_click(&mut self, ray: Ray) {
        if self.other_command_running() || self.fit.busy() {
            return;
        }
        let Some(loaded) = &self.loaded else {
            return;
        };
        let picked = match self.views.active_view() {
            Some(view) => beamcraft_engine::views::pick_visible(
                &loaded.snapshot,
                view,
                ray.origin,
                ray.direction,
                self.selection.filter(),
                false,
            )
            .map(|hit| hit.map(|part| part.element_id.to_string()))
            .map_err(|e| ToolError::local("PICK_REFUSED", e.to_string())),
            None => pick_in(&loaded.snapshot, &ray, self.selection.filter()),
        };
        match picked {
            Ok(Some(id)) => {
                if self.fit.part.as_deref() == Some(id.as_str()) {
                    self.fit.note = Some(Err(
                        "That is the part to trim; pick the plate it fits to.".into(),
                    ));
                    return;
                }
                if self.fit.part.is_none() {
                    self.fit.part = Some(id);
                } else {
                    self.fit.tool = Some(id);
                    // Both picked: the fit is one gesture, as the weld's two parts are not,
                    // since nothing more is asked.
                    self.start_fit_commit();
                    return;
                }
                self.fit.note = None;
            }
            Ok(None) => self.fit.note = Some(Err("No part was hit; aim at a model face.".into())),
            Err(error) => self.fit.note = Some(Err(error.to_string())),
        }
    }

    /// Starts the committed `fit-end-to-part` on a worker.
    pub(crate) fn start_fit_commit(&mut self) {
        if self.other_command_running() {
            self.fit.note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let Some(operation) = crate::sheet::fresh_id() else {
            self.fit.note = Some(Err("No identity could be made for the fitting.".into()));
            return;
        };
        let args = match fit_args(
            &self.source.store,
            self.source.actor.as_deref(),
            self.fit.part.as_deref().unwrap_or(""),
            self.fit.tool.as_deref().unwrap_or(""),
            &operation,
        ) {
            Ok(args) => args,
            Err(message) => {
                self.fit.note = Some(Err(message));
                return;
            }
        };
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-fit-end".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                    let reply = decode(&out.stdout)?;
                    rejection(&reply, out.code).map_or(Ok(()), Err)
                });
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.fit.commit_job = Some(rx);
                self.fit.note = None;
            }
            Err(_) => self.fit.note = Some(Err("The fit command could not be started.".into())),
        }
    }

    /// Drains the fit commit; a success reloads the model and readies the next pair.
    pub(crate) fn poll_fit(&mut self) {
        let Some(rx) = &self.fit.commit_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The fit command ended without a result.",
            )),
        };
        self.fit.commit_job = None;
        self.fit.clear_placement();
        match result {
            Ok(()) => {
                self.fit.note = Some(Ok(
                    "End fitted. Pick the next part, or press Esc to finish.".into(),
                ));
                self.reload();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                }
                self.fit.note = Some(Err(error.to_string()));
            }
        }
    }

    /// The inspector panel of the Fit end bench.
    pub(crate) fn fit_panel(&mut self, ui: &mut egui::Ui) {
        use crate::kit;
        let palette = crate::theme::Palette::current(ui);
        ui.set_max_width(ui.available_width());
        kit::type_box(ui, crate::theme::Icon::End, "Fit end to part");
        kit::section(ui, "fit", "Parts", true, |ui| {
            kit::prop_info(ui, "Part to trim", self.fit.part.as_deref(), "Click a part");
            kit::prop_info(
                ui,
                "Fit to plate",
                self.fit.tool.as_deref(),
                "Click a plate",
            );
            kit::prop_row(ui, "", |ui| {
                ui.label(
                    egui::RichText::new(if self.fit.part.is_none() {
                        "Click the beam, column or plate whose end is trimmed."
                    } else {
                        "Click the plate the end fits to; the end nearer the plate is trimmed \
                         flush with its nearest face."
                    })
                    .size(11.0)
                    .color(palette.muted),
                );
            });
            kit::prop_row(ui, "", |ui| {
                if ui
                    .add_enabled(
                        self.fit.part.is_some() && !self.fit.busy(),
                        egui::Button::new("Clear picks"),
                    )
                    .clicked()
                {
                    self.fit.clear_placement();
                }
            });
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::fit_args;
    use std::path::Path;

    #[test]
    fn the_fit_runs_the_cli_command_with_both_parts_and_a_fresh_operation() {
        let args = fit_args(Path::new("m"), Some("actor"), " a ", "b", "op").unwrap();
        assert_eq!(
            args,
            [
                "fit-end-to-part",
                "--store",
                "m",
                "--id",
                "a",
                "--operation-id",
                "op",
                "--tool",
                "b",
                "--actor-id",
                "actor"
            ]
        );
        assert!(fit_args(Path::new("m"), None, "", "b", "op").is_err());
        assert!(fit_args(Path::new("m"), None, "a", "a", "op").is_err());
    }
}
