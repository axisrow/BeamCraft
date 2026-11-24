// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The insertion form invokes `create-grid`; display endpoints arrive with the pinned snapshot.

use super::{App, workbench::Bench};
use crate::grid::{FamilyForm, GridScene};
use crate::theme::{self, Palette};
use crate::tools::ToolError;
use egui::Ui;
use std::sync::Arc;
use std::sync::mpsc::{self, TryRecvError};

pub(super) struct GridEdit {
    pub(super) id: String,
    original: crate::grid::GridForm,
    editable: bool,
    stale: bool,
    source: Option<beamcraft_model::grid::Grid>,
}

/// One text property of the grid form as a table row, with a muted hint while it is empty.
fn field(ui: &mut Ui, label: &str, value: &mut String, hint: &str) {
    crate::kit::prop_row(ui, label, |ui| {
        ui.add(
            egui::TextEdit::singleline(value)
                .hint_text(hint)
                .desired_width((ui.available_width() - crate::kit::SPACE_XS).max(30.0))
                .char_limit(4096),
        );
    });
}

fn family(ui: &mut Ui, title: &str, form: &mut FamilyForm, hint: &str) {
    crate::kit::section(ui, "grid", title, true, |ui| {
        field(
            ui,
            "Coordinates (mm)",
            &mut form.expression,
            "Start coordinate, then spacings",
        );
        if let Ok((_, positions)) =
            beamcraft_model::grid::parse_grid_expression(form.expression.trim())
        {
            ui.label(egui::RichText::new(format!("{} axes", positions.len())).small());
        }
        ui.label(
            egui::RichText::new("First position, then positive spacings; N*D repeats a spacing.")
                .small(),
        );
        field(ui, "Labels", &mut form.labels, hint);
        ui.label(
                egui::RichText::new("Missing labels continue from the last one. Extra labels are unused. Leave blank for automatic labels.")
                    .small(),
            );
        crate::kit::fold(ui, title, "Line limits", false, |ui| {
            let [start, end] = &mut form.span;
            field(ui, "Line start (mm)", start, "Automatic");
            field(ui, "Line end (mm)", end, "Automatic");
        });
    });
}

impl App {
    /// Leaves the edit of a selected grid. The form held that grid's values, its line limits
    /// too, so it goes back to a new grid's defaults: a grid created next must not inherit
    /// another grid's limits (a small grid took the large one's 50 m lines).
    pub(super) fn drop_grid_edit(&mut self) {
        if self.grid_edit.take().is_some() {
            self.grid_form = crate::grid::GridForm::default();
            self.grid_line = crate::grid::DrawLineForm::default();
        }
    }

    pub(super) fn select_grid(&mut self, id: &str) {
        let Some(grid) = self.grid_scene.get(id).cloned() else {
            return;
        };
        self.invalidate_select();
        self.editor.deselect();
        self.props.clear();
        self.set_bench(Bench::Select);
        self.beam_mode = false;
        self.beam.reset();
        self.ghost_end = None;
        self.clear_ghost();
        self.grid_form = grid.form.clone();
        self.grid_line = crate::grid::DrawLineForm::default();
        self.grid_drawn = !grid.editable;
        self.grid_edit = Some(GridEdit {
            id: grid.id,
            original: grid.form,
            editable: grid.editable,
            stale: false,
            source: self
                .grid_scene
                .query_grids
                .iter()
                .find(|g| g.id.to_string() == id)
                .cloned(),
        });
        self.grid_note = None;
    }

    pub(super) fn grid_line_start_point(&self) -> Option<[f64; 3]> {
        (self.grid_line.click == Some(true))
            .then(|| self.grid_line.points.first())
            .flatten()
            .and_then(|[x, y]| {
                Some([
                    x.trim().parse().ok()?,
                    y.trim().parse().ok()?,
                    self.tools.plane.map_or(0.0, |p| p.origin()[2]),
                ])
            })
    }

    /// Where a grid-line pick lands: the ground point under `ray`, snapped like placement, and
    /// the snap kind it took. `Err` names why there is no point.
    pub(super) fn grid_line_hit(
        &self,
        ray: crate::edit::Ray,
    ) -> Result<([f64; 3], Option<beamcraft_engine::query::pick::SnapKind>), String> {
        use beamcraft_engine::query::pick::SnapKind;
        let point = super::ground_point(&ray).ok_or("Aim at the horizontal grid plane.")?;
        let kinds = self.selection.snap_kinds();
        let from = self.grid_line_start_point().map(|p| [p[0], p[1], point[2]]);
        let Some(loaded) = self.loaded.as_ref().filter(|_| !kinds.is_empty()) else {
            return self
                .constrain_creation(point, None, ray, false)
                .map_err(|e| e.to_string());
        };
        let grids = if self.grids_shown() {
            self.grid_scene.query_grids.as_slice()
        } else {
            &[]
        };
        let context = beamcraft_engine::query::pick::SnapContext {
            grids,
            member_axes: &self.grid_scene.member_axes,
            sources: self.selection.snap_sources(),
            nearest: self.selection.snap_nearest,
            from,
            ..Default::default()
        };
        for discrete in [true, false] {
            let kinds = kinds
                .iter()
                .copied()
                .filter(|kind| {
                    matches!(kind, SnapKind::Axis | SnapKind::Edge | SnapKind::GridLine) != discrete
                })
                .collect::<Vec<_>>();
            if kinds.is_empty() {
                continue;
            }
            let query = beamcraft_engine::query::pick::SnapQuery {
                point_mm: point,
                radius_mm: 100.0,
                kinds,
                max_hits: 1,
            };
            let result = beamcraft_engine::query::pick::snap_with_context(
                &loaded.snapshot,
                &query,
                &context,
            )
            .map_err(|error| error.to_string())?;
            if let Some(hit) = result.candidates.first() {
                return self
                    .constrain_creation(hit.point_world_mm, Some(hit.kind), ray, false)
                    .map_err(|e| e.to_string());
            }
        }
        self.constrain_creation(point, None, ray, false)
            .map_err(|e| e.to_string())
    }

    pub(super) fn accept_grid_point(&mut self, point: [f64; 3]) {
        // A grid line is plan-only. Snap supplies XY; height never becomes grid data.
        let index = usize::from(self.grid_line.click == Some(true));
        if let Some(endpoint) = self.grid_line.points.get_mut(index) {
            *endpoint = [point[0].to_string(), point[1].to_string()];
        }
        self.grid_line.click = if index == 0 { Some(true) } else { None };
    }

    pub(super) fn place_grid_line(&mut self, ray: crate::edit::Ray) {
        if self.other_command_running() {
            return;
        }
        let point = match self.grid_line_hit(ray) {
            Ok((point, _)) => point,
            Err(error) => {
                self.grid_note = Some(Err(error));
                return;
            }
        };
        self.accept_grid_point(point);
    }

    fn drawn_line_fields(&mut self, ui: &mut Ui, editing: bool) {
        if editing {
            egui::ComboBox::from_id_salt("grid-line-action")
                .selected_text(match self.grid_line.action.as_str() {
                    "update-grid" => "Grid properties",
                    "reshape-grid-line" => "Reshape line",
                    "relabel-grid-line" => "Relabel line",
                    "remove-grid-line" => "Remove line",
                    _ => "Add line",
                })
                .show_ui(ui, |ui| {
                    for (action, label) in [
                        ("update-grid", "Grid properties"),
                        ("draw-grid-line", "Add line"),
                        ("reshape-grid-line", "Reshape line"),
                        ("relabel-grid-line", "Relabel line"),
                        ("remove-grid-line", "Remove line"),
                    ] {
                        ui.selectable_value(&mut self.grid_line.action, action.into(), label);
                    }
                });
            if !matches!(
                self.grid_line.action.as_str(),
                "draw-grid-line" | "update-grid"
            ) {
                let selected = self.grid_scene.query_grids.iter().find(|g| {
                    self.grid_edit
                        .as_ref()
                        .is_some_and(|e| e.id == g.id.to_string())
                });
                let axis_label = selected
                    .and_then(|grid| beamcraft_engine::grid_query::segments(grid).ok())
                    .and_then(|lines| {
                        lines
                            .into_iter()
                            .find(|line| line.axis_id.to_string() == self.grid_line.axis)
                    })
                    .map_or_else(|| "Choose axis".into(), |line| line.label);
                egui::ComboBox::from_id_salt("grid-line-axis")
                    .selected_text(axis_label)
                    .show_ui(ui, |ui| {
                        if let Some(grid) = selected
                            && let Ok(lines) = beamcraft_engine::grid_query::segments(grid)
                        {
                            for line in lines {
                                if ui
                                    .selectable_value(
                                        &mut self.grid_line.axis,
                                        line.axis_id.to_string(),
                                        &line.label,
                                    )
                                    .clicked()
                                {
                                    self.grid_line.label = line.label;
                                    self.grid_line.points = [line.start, line.end].map(|p| {
                                        beamcraft_engine::grid_query::local_to_world(grid, p)
                                            .map(|n| n.to_string())
                                    });
                                }
                            }
                        }
                    });
            }
        }
        if self.grid_line.action == "draw-grid-line" {
            ui.horizontal(|ui| {
                ui.label("Family");
                ui.selectable_value(&mut self.grid_line.family, "u".into(), "U");
                ui.selectable_value(&mut self.grid_line.family, "v".into(), "V");
            });
        }
        if matches!(
            self.grid_line.action.as_str(),
            "draw-grid-line" | "relabel-grid-line"
        ) {
            field(ui, "Axis label", &mut self.grid_line.label, "D1");
        }
        if matches!(
            self.grid_line.action.as_str(),
            "draw-grid-line" | "reshape-grid-line"
        ) {
            for (name, point) in ["Start", "End"].into_iter().zip(&mut self.grid_line.points) {
                ui.label(format!("{name} · world XY (mm)"));
                for (label, value) in ["X", "Y"].into_iter().zip(point) {
                    field(ui, label, value, "0");
                }
            }
            if ui.button("Pick two points in view").clicked() {
                self.grid_line.click = Some(false);
                self.beam_mode = false;
            }
            if let Some(second) = self.grid_line.click {
                ui.label(if second {
                    "Pick end point; Apply saves the line."
                } else {
                    "Pick start point. Esc cancels picking."
                });
                if ui.button("Cancel picking").clicked() {
                    self.grid_line.click = None;
                }
            }
            ui.label("Snaps use the bottom snap panel (100 mm radius). Hidden grids are excluded.");
        }
    }

    pub(super) fn sync_grid_selection(&mut self) {
        let Some(edit) = &mut self.grid_edit else {
            self.grid_refresh_edit = false;
            return;
        };
        let Some(grid) = self.grid_scene.get(&edit.id) else {
            self.drop_grid_edit();
            self.grid_refresh_edit = false;
            self.grid_note = Some(Err("The selected grid is no longer available.".into()));
            return;
        };
        let source = self
            .grid_scene
            .query_grids
            .iter()
            .find(|g| g.id.to_string() == edit.id)
            .cloned();
        let own_refresh = std::mem::take(&mut self.grid_refresh_edit);
        if self.grid_drawn && source != edit.source && !own_refresh {
            edit.stale = true;
        } else if own_refresh || self.grid_form == edit.original && !edit.stale {
            self.grid_form = grid.form.clone();
            edit.original = grid.form.clone();
            edit.source = source;
            edit.stale = false;
        } else {
            edit.stale = grid.form != edit.original;
        }
        edit.editable = grid.editable;
    }

    pub(super) fn grid_framing(&self, summary: &crate::scene::SceneSummary) -> ([f32; 3], f32) {
        if self.grids_shown() {
            self.grid_scene.framing(summary)
        } else {
            summary.framing()
        }
    }

    pub(super) fn apply_grids(&mut self, grids: Result<GridScene, ToolError>) {
        match grids {
            Ok(scene) => self.grid_scene = scene,
            Err(error) => {
                // The guides the window already shows stay: a transient CLI failure must not
                // wipe them or reframe the view onto the shrunken framing (issue 562 item 50,
                // review). The failure is reported on its own.
                self.grid_note = Some(Err(error.to_string()));
            }
        }
    }

    pub(super) fn grid_panel(&mut self, ui: &mut Ui) {
        let busy = self.other_command_running();
        let editing = self.grid_edit.is_some();
        let editable = self
            .grid_edit
            .as_ref()
            .is_none_or(|edit| edit.editable && !edit.stale);
        let changed = self
            .grid_edit
            .as_ref()
            .is_none_or(|edit| edit.original != self.grid_form);
        let drawn = self.grid_drawn;
        let line_editable = self.grid_edit.as_ref().is_none_or(|e| !e.stale);
        egui::Panel::bottom("grid-actions")
            .min_size(
                crate::kit::ACTION_BAR_HEIGHT
                    + if editing {
                        crate::kit::ROW_HEIGHT + crate::kit::SPACE_S
                    } else {
                        0.0
                    },
            )
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 4)))
            .show(ui, |ui| {
                let done = crate::kit::actions(
                    ui,
                    if editing {
                        "Apply changes"
                    } else {
                        "Create grid"
                    },
                    if drawn {
                        line_editable
                    } else {
                        editable && changed
                    },
                    "Cancel",
                    busy || self.grid_job.is_some(),
                );
                if done.primary {
                    self.start_grid();
                }
                // An existing grid is deleted from the panel itself, not only with the Delete key.
                if editing {
                    ui.horizontal(|ui| {
                        let free = !busy && self.grid_job.is_none() && editable;
                        if theme::button(ui, theme::Kind::Secondary, "Delete grid", free)
                            .on_hover_text("Delete this grid (Ctrl+Z restores it)")
                            .clicked()
                        {
                            self.delete_selection();
                        }
                    });
                }
                if done.cancel {
                    self.grid_edit = None;
                    self.grid_line = crate::grid::DrawLineForm::default();
                    self.grid_form = crate::grid::GridForm::default();
                    self.grid_note = None;
                    self.set_bench(Bench::Select);
                }
            });
        egui::ScrollArea::vertical()
            .id_salt("grid-form")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                crate::kit::type_box(
                    ui,
                    theme::Icon::Grid,
                    if editing { "Grid" } else { "New grid" },
                );
                if self.grid_edit.as_ref().is_some_and(|edit| edit.stale) {
                    ui.colored_label(Palette::current(ui).error, "This grid changed while you were editing. Select it again before applying changes.");
                }
                // The kind is chosen once, when the grid is made: a regular grid (positions by
                // spacing) and a drawn grid (lines one by one) never convert into each other, so an
                // existing grid opens in its own kind's tools, never in the other's, which the engine
                // would refuse.
                if !editing {
                    crate::kit::prop_choice(
                        ui,
                        "grid-kind",
                        "Kind",
                        &mut self.grid_drawn,
                        &[(false, "Regular grid"), (true, "Drawn grid")],
                    );
                }
                ui.add_space(8.0);
                ui.add_enabled_ui(!busy && (editable || drawn && line_editable), |ui| {
                    if !drawn || !editing || self.grid_line.action == "update-grid" {
                        field(ui, "Name", &mut self.grid_form.name, "Grid name");
                    }
                    if drawn {
                        self.drawn_line_fields(ui, editing);
                    } else {
                        family(ui, "U family · X positions", &mut self.grid_form.u, "1 2 3");
                        family(ui, "V family · Y positions", &mut self.grid_form.v, "A B C");
                    }
                    if !drawn || !editing || self.grid_line.action == "update-grid" {
                        crate::kit::section(ui, "grid", "Position", false, |ui| {
                            field(ui, "Origin X (mm)", &mut self.grid_form.origin[0], "0");
                            field(ui, "Origin Y (mm)", &mut self.grid_form.origin[1], "0");
                            field(ui, "Rotation (degrees)", &mut self.grid_form.rotation, "0");
                        });
                    }
                });
            });
        // Its outcome is told in the status bar (`App::status_note`).
    }

    fn start_grid(&mut self) {
        if self.other_command_running() {
            return;
        }
        self.grid_line.click = None;
        let prepared = if self.grid_drawn {
            if self.grid_edit.as_ref().is_some_and(|e| e.stale) {
                return;
            }
            if self.grid_line.action == "update-grid"
                && let Some(edit) = &self.grid_edit
            {
                self.grid_form.prepare_properties(
                    &edit.original,
                    &edit.id,
                    &self.source.store,
                    self.source.actor.as_deref(),
                )
            } else {
                self.grid_line.prepare(
                    self.grid_edit.as_ref().map(|e| e.id.as_str()),
                    &self.grid_form,
                    &self.source.store,
                    self.source.actor.as_deref(),
                )
            }
        } else if let Some(edit) = &self.grid_edit {
            if !edit.editable || edit.stale {
                return;
            }
            self.grid_form.prepare_update(
                &edit.original,
                &edit.id,
                &self.source.store,
                self.source.actor.as_deref(),
            )
        } else {
            self.grid_form
                .prepare(&self.source.store, self.source.actor.as_deref())
        };
        let args = match prepared {
            Ok(args) => args,
            Err(error) => {
                self.grid_note = Some(Err(error.to_string()));
                return;
            }
        };
        // A first grid in an empty model is framed at once, so it is seen where it was made.
        self.grid_fit_after = self.grid_edit.is_none()
            && self.loaded.as_ref().is_none_or(|loaded| {
                loaded
                    .snapshot
                    .elements()
                    .iter()
                    .all(|element| element.kind == beamcraft_engine::ElementKind::Grid)
            });
        self.spawn_grid_command(args);
    }

    /// Runs one grid command on a worker; [`App::poll_grid`] takes its outcome.
    fn spawn_grid_command(&mut self, args: Vec<String>) {
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("beamcraft-create-grid".into())
            .spawn(move || {
                let _ = tx.send(crate::grid::commit(&cli, &args, &stats));
            }) {
            Ok(_) => {
                self.grid_job = Some(rx);
                self.grid_note = None;
            }
            Err(_) => self.grid_note = Some(Err("The grid command could not be started.".into())),
        }
    }

    pub(super) fn poll_grid(&mut self) {
        let Some(rx) = &self.grid_job else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The grid command ended without a result.",
            )),
        };
        self.grid_job = None;
        match result {
            Ok(_) => {
                self.set_grids_shown(true);
                self.pending_fit |= std::mem::take(&mut self.grid_fit_after);
                self.grid_refresh_edit = self.grid_edit.is_some();
                self.grid_note = Some(Ok(if self.grid_refresh_edit {
                    "Grid updated."
                } else {
                    "Grid created."
                }
                .into()));
                self.reload();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                }
                self.grid_note = Some(Err(error.to_string()));
            }
        }
    }
}
