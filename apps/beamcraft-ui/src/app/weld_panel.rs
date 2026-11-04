// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The ribbon Weld button (V10): two picked parts commit through the same `weld create-weld`
//! command a user types (A06, fillet welds). The reply is the CLI's; a refusal publishes
//! nothing and is shown next to the form.

use super::{App, field};
use crate::command;
use crate::edit::{Ray, pick_in};
use crate::tools::{MAX_TEXT, ToolError, decode, rejection};
use egui::RichText;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// Where a weld is made, as `--location` spells it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Location {
    #[default]
    Workshop,
    Site,
}

impl Location {
    /// The CLI token.
    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::Workshop => "workshop",
            Self::Site => "site",
        }
    }

    /// The form label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Workshop => "Workshop",
            Self::Site => "Site",
        }
    }
}

/// Which side of the joint the arrow points at, as `--side` spells it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Side {
    #[default]
    Arrow,
    Other,
    Both,
}

impl Side {
    /// The CLI token.
    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::Arrow => "arrow",
            Self::Other => "other",
            Self::Both => "both",
        }
    }

    /// The form label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Arrow => "Arrow",
            Self::Other => "Other",
            Self::Both => "Both",
        }
    }
}

/// The Weld bench's state between frames; everything else is on [`App`].
pub(crate) struct Panel {
    /// The part the first click picked, as the engine reported its id.
    pub(crate) main: Option<String>,
    /// The part the second click picked.
    pub(crate) secondary: Option<String>,
    /// The weld's label (`--name`, required by the command).
    pub(crate) name: String,
    /// The fillet leg, as typed (`--size`, in millimetres).
    pub(crate) size: String,
    /// Workshop or site.
    pub(crate) location: Location,
    /// Arrow, other or both.
    pub(crate) side: Side,
    pub(super) commit_job: Option<Receiver<Result<(), ToolError>>>,
    /// The last outcome, shown under the form.
    pub(crate) note: Option<Result<String, String>>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self {
            main: None,
            secondary: None,
            name: "Weld".into(),
            size: String::new(),
            location: Location::default(),
            side: Side::default(),
            commit_job: None,
            note: None,
        }
    }

    fn busy(&self) -> bool {
        self.commit_job.is_some()
    }

    /// Forgets the picked pair; the form keeps its typed values.
    fn clear_placement(&mut self) {
        self.main = None;
        self.secondary = None;
    }
}

/// The size a typed text stands for: a finite decimal, as the command's `--size` takes it.
fn size_mm(text: &str) -> Result<f64, String> {
    let text = text.trim();
    if text.is_empty() || text.len() > 32 {
        return Err("Size: enter a number of millimetres".into());
    }
    text.parse::<f64>().map_or_else(
        |_| {
            Err(format!(
                "Size: \"{text}\" is not a number; use a period as the decimal separator"
            ))
        },
        |value| {
            if value.is_finite() {
                Ok(value)
            } else {
                Err(format!("Size: \"{text}\" is not a finite number"))
            }
        },
    )
}

/// The `weld create-weld` arguments of the form, exactly as the command takes them. The fillet
/// type is the engine's only supported one, so it is stated rather than offered as a choice.
#[allow(clippy::too_many_arguments)] // one argument per command option, as the command takes them
pub(crate) fn create_args(
    store: &Path,
    actor: Option<&str>,
    name: &str,
    main: &str,
    secondary: &str,
    size: &str,
    location: Location,
    side: Side,
) -> Result<Vec<String>, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name: enter a label for the weld".into());
    }
    if name.len() > MAX_TEXT {
        return Err(format!("Name: at most {MAX_TEXT} bytes"));
    }
    let main = main.trim();
    let secondary = secondary.trim();
    if main.is_empty() || secondary.is_empty() {
        return Err("Pick the main part, then the secondary part, in the model view.".into());
    }
    if main == secondary {
        return Err("A weld connects two distinct parts; pick a different secondary part.".into());
    }
    let size = size_mm(size)?;
    let mut args = vec![
        "create-weld".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--name".into(),
        name.to_owned(),
        "--main".into(),
        main.to_owned(),
        "--secondary".into(),
        secondary.to_owned(),
        "--type".into(),
        "fillet".into(),
        "--size".into(),
        size.to_string(),
        "--location".into(),
        location.token().into(),
        "--side".into(),
        side.token().into(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

impl App {
    /// Resolves a placement click on the viewport: the first click picks the main part, the
    /// second the secondary part. The switches apply, as they do to selection, and with an
    /// active model view only what the view shows can be picked (V12), as selection does.
    pub(crate) fn weld_click(&mut self, ray: Ray) {
        if self.other_command_running() || self.weld.busy() {
            return;
        }
        let Some(loaded) = &self.loaded else {
            return;
        };
        let view = self.views.active_view().cloned();
        let picked = match view.as_ref() {
            Some(view) => beamcraft_engine::views::pick_visible(
                &loaded.snapshot,
                view,
                ray.origin,
                ray.direction,
                self.selection.filter(),
            )
            .map(|hit| hit.map(|part| part.element_id.to_string()))
            .map_err(|e| ToolError::local("PICK_REFUSED", e.to_string())),
            None => pick_in(&loaded.snapshot, &ray, self.selection.filter()),
        };
        match picked {
            Ok(Some(id)) => {
                let slot = if self.weld.main.is_none() {
                    &mut self.weld.main
                } else {
                    &mut self.weld.secondary
                };
                if slot.as_deref() == Some(id.as_str()) {
                    self.weld.note = Some(Err(
                        "Already picked; pick a different part for the other side.".into(),
                    ));
                    return;
                }
                *slot = Some(id);
                self.weld.note = None;
            }
            Ok(None) => {
                self.weld.note = Some(Err("No part was hit; aim at a model face.".into()));
            }
            Err(error) => self.weld.note = Some(Err(error.to_string())),
        }
    }

    /// Starts the committed `weld create-weld` on a worker: the same command a user types.
    pub(crate) fn start_weld_commit(&mut self) {
        if self.other_command_running() {
            self.weld.note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let args = match create_args(
            &self.source.store,
            self.source.actor.as_deref(),
            &self.weld.name,
            self.weld.main.as_deref().unwrap_or(""),
            self.weld.secondary.as_deref().unwrap_or(""),
            &self.weld.size,
            self.weld.location,
            self.weld.side,
        ) {
            Ok(args) => args,
            Err(message) => {
                self.weld.note = Some(Err(message));
                return;
            }
        };
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-weld-create".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                    let reply = decode(&out.stdout)?;
                    rejection(&reply, out.code).map_or(Ok(()), Err)
                });
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.weld.commit_job = Some(rx);
                self.weld.note = None;
            }
            Err(_) => self.weld.note = Some(Err("The weld command could not be started.".into())),
        }
    }

    /// Drains the weld commit; a success reloads the model the viewport shows.
    pub(crate) fn poll_weld(&mut self) {
        let Some(rx) = &self.weld.commit_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The weld command ended without a result.",
            )),
        };
        self.weld.commit_job = None;
        match result {
            Ok(()) => {
                self.weld.clear_placement();
                self.weld.note = Some(Ok(
                    "Created weld. Pick the next pair, or press Esc to finish.".into(),
                ));
                self.reload();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                }
                self.weld.note = Some(Err(error.to_string()));
            }
        }
    }

    /// The inspector panel of the Weld bench.
    pub(crate) fn weld_panel(&mut self, ui: &mut egui::Ui) {
        let palette = crate::theme::Palette::current(ui);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.set_max_width(ui.available_width());
        let busy = self.weld.busy() || self.other_command_running();
        ui.label(crate::theme::bold("Weld properties", 16.0));
        ui.label("Fillet welds of two picked parts, through the engine's create-weld command.");
        let row = |ui: &mut egui::Ui, label: &str, value: Option<&String>| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(11.0));
                match value {
                    Some(id) => ui.label(RichText::new(id.as_str()).size(11.0)),
                    None => ui.label(
                        RichText::new("— click a part")
                            .size(11.0)
                            .color(palette.muted),
                    ),
                };
            });
        };
        row(ui, "Main", self.weld.main.as_ref());
        row(ui, "Secondary", self.weld.secondary.as_ref());
        ui.label(
            match (self.weld.main.is_some(), self.weld.secondary.is_some()) {
                (false, _) => "Click the main part in the model view.",
                (true, false) => "Click the secondary part in the model view.",
                (true, true) => "Both parts are picked; fill the form and press Create weld.",
            },
        );
        if ui
            .add_enabled(
                self.weld.main.is_some() || self.weld.secondary.is_some(),
                egui::Button::new("Clear picks"),
            )
            .clicked()
        {
            self.weld.clear_placement();
        }
        field(ui, "Name", &mut self.weld.name, 160.0);
        field(ui, "Size (mm)", &mut self.weld.size, 80.0);
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(70.0, 28.0),
                egui::Label::new(egui::RichText::new("Location").size(11.0)),
            );
            egui::ComboBox::from_id_salt("weld-location")
                .width(ui.available_width().max(80.0))
                .selected_text(self.weld.location.label())
                .show_ui(ui, |ui| {
                    for choice in [Location::Workshop, Location::Site] {
                        if ui
                            .selectable_label(self.weld.location == choice, choice.label())
                            .clicked()
                        {
                            self.weld.location = choice;
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(70.0, 28.0),
                egui::Label::new(egui::RichText::new("Side").size(11.0)),
            );
            egui::ComboBox::from_id_salt("weld-side")
                .width(ui.available_width().max(80.0))
                .selected_text(self.weld.side.label())
                .show_ui(ui, |ui| {
                    for choice in [Side::Arrow, Side::Other, Side::Both] {
                        if ui
                            .selectable_label(self.weld.side == choice, choice.label())
                            .clicked()
                        {
                            self.weld.side = choice;
                        }
                    }
                });
        });
        ui.separator();
        ui.add_enabled_ui(!busy, |ui| {
            if ui.button("Create weld").clicked() {
                self.start_weld_commit();
            }
            if ui.button("Cancel").clicked() {
                self.weld.clear_placement();
                self.weld.note = None;
                self.set_bench(super::workbench::Bench::Select);
            }
        });
        if busy {
            ui.spinner();
            ui.label("Saving weld…");
        }
        match &self.weld.note {
            Some(Ok(message)) => {
                ui.label(message);
            }
            Some(Err(message)) => {
                ui.colored_label(palette.error, message);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "0197aabb-ccdd-7ebb-9beb-59df63a5fc11";
    const OTHER: &str = "0197aabb-ccdd-7ebb-9beb-59df63a5fc12";

    fn args(size: &str) -> Result<Vec<String>, String> {
        create_args(
            Path::new("S"),
            None,
            "Weld 1",
            MAIN,
            OTHER,
            size,
            Location::Workshop,
            Side::Both,
        )
    }

    #[test]
    fn the_form_builds_the_exact_create_weld_command() {
        assert_eq!(
            args("6").unwrap(),
            vec![
                "create-weld",
                "--store",
                "S",
                "--name",
                "Weld 1",
                "--main",
                MAIN,
                "--secondary",
                OTHER,
                "--type",
                "fillet",
                "--size",
                "6",
                "--location",
                "workshop",
                "--side",
                "both",
            ]
        );
        let with_actor = create_args(
            Path::new("S"),
            Some("actor"),
            "Weld",
            MAIN,
            OTHER,
            "6",
            Location::Site,
            Side::Arrow,
        )
        .unwrap();
        assert_eq!(
            with_actor[with_actor.len() - 2..],
            ["--actor-id".to_owned(), "actor".to_owned()]
        );
        assert!(
            with_actor
                .windows(2)
                .any(|pair| pair == ["--location", "site"])
        );
        assert!(
            with_actor
                .windows(2)
                .any(|pair| pair == ["--side", "arrow"])
        );
    }

    #[test]
    fn an_unfilled_or_self_connected_form_refuses_without_a_command() {
        let refused = |name: &str, main: &str, secondary: &str, size: &str| {
            create_args(
                Path::new("S"),
                None,
                name,
                main,
                secondary,
                size,
                Location::Workshop,
                Side::Arrow,
            )
        };
        assert!(refused("", MAIN, OTHER, "6").unwrap_err().contains("Name"));
        assert!(
            refused(&"x".repeat(MAX_TEXT + 1), MAIN, OTHER, "6")
                .unwrap_err()
                .contains("Name")
        );
        assert!(
            refused("Weld", "", OTHER, "6")
                .unwrap_err()
                .contains("Pick the main part")
        );
        assert!(
            refused("Weld", MAIN, "", "6")
                .unwrap_err()
                .contains("Pick the main part")
        );
        assert!(
            refused("Weld", MAIN, MAIN, "6")
                .unwrap_err()
                .contains("two distinct parts")
        );
    }

    #[test]
    fn a_size_that_would_not_parse_refuses_in_the_form_and_a_zero_reaches_the_engine() {
        assert!(args("").unwrap_err().contains("Size"));
        assert!(args("wide").unwrap_err().contains("not a number"));
        assert!(args("6,5").unwrap_err().contains("not a number"));
        // A finite but non-positive size is the engine's refusal, not the form's: it must
        // reach the real command and come back as the engine's verdict.
        assert!(args("0").is_ok());
        assert!(args("-1").is_ok());
    }
}
