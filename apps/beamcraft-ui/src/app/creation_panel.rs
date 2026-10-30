// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Modelling properties and point-driven creation on the existing command adapters.

use super::{App, View, choice_combo, field, ground_point, pick_point_in, workbench::Bench};
use crate::tools::{ToolError, ToolKind};
use beamcraft_engine::query::pick::{SnapQuery, snap_with_grids};

impl App {
    pub(super) fn finish_creation_gesture(
        &mut self,
        response: &egui::Response,
        view: &View,
    ) -> bool {
        if self.other_command_running() {
            return false;
        }
        let finish_click = response.clicked_by(egui::PointerButton::Secondary)
            || response.clicked_by(egui::PointerButton::Middle);
        if self.beam_mode && finish_click {
            self.set_bench(Bench::Select);
            return true;
        }
        if !self.plate_mode {
            return false;
        }
        let close_marker = response.clicked_by(egui::PointerButton::Primary)
            && response.interact_pointer_pos().is_some_and(|point| {
                self.plate_draft.closes_near(
                    response.rect,
                    view,
                    self.loaded
                        .as_ref()
                        .map_or([0.0; 3], |l| l.summary.origin_mm),
                    point,
                )
            });
        if close_marker
            || response.clicked_by(egui::PointerButton::Secondary)
            || response.clicked_by(egui::PointerButton::Middle)
        {
            self.finish_plate();
            return true;
        }
        false
    }
    pub(super) fn creation_point(
        &self,
        ray: crate::edit::Ray,
        planar: bool,
    ) -> Result<[f64; 3], ToolError> {
        let point = if let Some(plane) = self.tools.plane {
            plane.ray_point(ray.origin, ray.direction)
        } else if planar {
            ground_point(&ray)
        } else if let Some(loaded) = &self.loaded {
            pick_point_in(&loaded.snapshot, &ray)?
                .map(|(point, _)| point)
                .or_else(|| ground_point(&ray))
        } else {
            ground_point(&ray)
        };
        let mut point = point.ok_or_else(|| {
            ToolError::local(
                "INVALID_ARGUMENT",
                "Aim at the model or the active work plane.",
            )
        })?;
        let kinds = self.selection.snap_kinds();
        if !kinds.is_empty()
            && let Some(loaded) = &self.loaded
        {
            let grids = if self.grids_visible {
                self.grid_scene.query_grids.as_slice()
            } else {
                &[]
            };
            let result = snap_with_grids(
                &loaded.snapshot,
                &SnapQuery {
                    point_mm: point,
                    radius_mm: 100.0,
                    kinds,
                    max_hits: 32,
                },
                grids,
                None,
            )
            .map_err(|error| ToolError::local("INVALID_ARGUMENT", error.to_string()))?;
            if let Some(hit) = result.candidates.iter().find(|hit| {
                self.tools.plane.map_or_else(
                    || !planar || hit.point_world_mm[2].abs() <= 1e-6,
                    |plane| {
                        plane
                            .to_local(hit.point_world_mm)
                            .is_ok_and(|[_, _, z]| z.abs() <= 1e-6)
                    },
                )
            }) {
                point = hit.point_world_mm;
            }
        }
        Ok(point)
    }

    pub(super) fn create_from_form(&mut self) {
        if self.other_command_running() {
            return;
        }
        match self.tools.preview(&self.source.store) {
            Ok(_) => self.start_commit(),
            Err(error) => {
                self.tools.cancel();
                self.tool_note = Some(Err(error.to_string()));
            }
        }
    }

    pub(super) fn place_plate(&mut self, ray: crate::edit::Ray) {
        if self.other_command_running() {
            return;
        }
        match self.creation_point(ray, true) {
            Ok(point) => {
                if self.plate_draft.points().len() >= 3
                    && self.plate_draft.points().first().is_some_and(|first| {
                        first.iter().zip(point).all(|(a, b)| (*a - b).abs() <= 1e-6)
                    })
                {
                    self.finish_plate();
                    return;
                }
                if let Err(error) = self.plate_draft.push(point) {
                    self.tool_note = Some(Err(error.to_string()));
                } else {
                    self.tool_note = None;
                }
            }
            Err(error) => self.tool_note = Some(Err(error.to_string())),
        }
    }

    pub(super) fn finish_plate(&mut self) {
        if self.other_command_running() {
            return;
        }
        match self.plate_draft.contour(self.tools.plane) {
            Ok(contour) => {
                self.tools.plate.contour = contour;
                self.create_from_form();
            }
            Err(error) => self.tool_note = Some(Err(error.to_string())),
        }
    }

    pub(super) fn paint_plate_draft(&self, ui: &egui::Ui, rect: egui::Rect, view: &View) {
        let origin = self
            .loaded
            .as_ref()
            .map_or([0.0; 3], |l| l.summary.origin_mm);
        let project = |point: [f64; 3]| {
            let local = std::array::from_fn(|i| {
                (point.get(i).copied().unwrap_or(0.0) - origin.get(i).copied().unwrap_or(0.0))
                    as f32
            });
            let [x, y, z] = view.project(local);
            (x.is_finite()
                && y.is_finite()
                && x.abs() < 1e5
                && y.abs() < 1e5
                && (0.0..=1.0).contains(&z))
            .then(|| {
                egui::pos2(
                    rect.left() + (x + 1.0) * rect.width() / 2.0,
                    rect.top() + (1.0 - y) * rect.height() / 2.0,
                )
            })
        };
        let painter = ui.painter_at(rect);
        let stroke = egui::Stroke::new(2.0, crate::theme::Palette::current(ui).link);
        for pair in self.plate_draft.points().windows(2) {
            if let [a, b] = pair
                && let (Some(a), Some(b)) = (project(*a), project(*b))
            {
                painter.line_segment([a, b], stroke);
            }
        }
        for point in self.plate_draft.points() {
            if let Some(point) = project(*point) {
                painter.circle_filled(point, 4.0, stroke.color);
            }
        }
        if let Some(hover) = self.plate_hover
            && let Some(last) = self.plate_draft.points().last()
            && let (Some(a), Some(b)) = (project(*last), project(hover))
        {
            painter.line_segment([a, b], stroke);
            if self.plate_draft.points().len() >= 2
                && let Some(first) = self.plate_draft.points().first().and_then(|p| project(*p))
            {
                painter.line_segment(
                    [b, first],
                    egui::Stroke::new(1.0, stroke.color.gamma_multiply(0.5)),
                );
            }
        }
    }

    pub(super) fn paint_member_draft(&self, ui: &egui::Ui, rect: egui::Rect, view: &View) {
        let (Some(start), Some(end)) = (self.beam.start(), self.beam_hover) else {
            return;
        };
        let origin = self
            .loaded
            .as_ref()
            .map_or([0.0; 3], |l| l.summary.origin_mm);
        let project = |point: [f64; 3]| {
            let local = std::array::from_fn(|i| {
                (point.get(i).copied().unwrap_or(0.0) - origin.get(i).copied().unwrap_or(0.0))
                    as f32
            });
            let [x, y, z] = view.project(local);
            (x.is_finite()
                && y.is_finite()
                && x.abs() < 1e5
                && y.abs() < 1e5
                && (0.0..=1.0).contains(&z))
            .then(|| {
                egui::pos2(
                    rect.left() + (x + 1.0) * rect.width() / 2.0,
                    rect.top() + (1.0 - y) * rect.height() / 2.0,
                )
            })
        };
        if let (Some(a), Some(b)) = (project(start), project(end)) {
            let painter = ui.painter_at(rect);
            let color = crate::theme::Palette::current(ui).link;
            painter.line_segment([a, b], egui::Stroke::new(2.0, color));
            painter.circle_filled(a, 4.0, color);
            painter.circle_filled(b, 4.0, color);
        }
    }

    pub(super) fn tools_panel(&mut self, ui: &mut egui::Ui) {
        let busy = self.tools.busy() || self.other_command_running();
        ui.add_enabled_ui(!busy, |ui| {
            match self.tools.active {
                ToolKind::Plate => {
                    ui.label(crate::theme::bold("Plate properties", 16.0));
                    field(ui, "Name", &mut self.tools.plate.name, 160.0);
                    field(ui, "Class", &mut self.tools.plate.class, 80.0);
                    field(ui, "Thickness (mm)", &mut self.tools.plate.thickness, 80.0);
                    choice_combo(ui, "plate-material", "Material", &mut self.tools.plate.material, &self.tools.definitions.materials, false);
                    ui.separator();
                    ui.label("Pick the contour vertices in the model view. Enter, right-click, wheel-click or Finish creates the plate. Backspace removes the last point.");
                    ui.label(format!("{} vertices", self.plate_draft.points().len()));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.plate_draft.points().len() >= 3, egui::Button::new("Finish plate")).clicked() { self.finish_plate(); }
                        if ui.add_enabled(!self.plate_draft.points().is_empty(), egui::Button::new("Remove last point")).clicked() { self.plate_draft.back(); }
                        if ui.add_enabled(self.plate_draft.points().len() >= 3, egui::Button::new("Reverse direction")).clicked() { self.plate_draft.reverse(); }
                    });
                    egui::CollapsingHeader::new("Precise point · work-plane mm").show(ui, |ui| {
                        for (axis, text) in ["X", "Y"].into_iter().zip(&mut self.plate_input) { field(ui, axis, text, 100.0); }
                        if ui.button("Add point").clicked() {
                            let numbers = self.plate_input.clone().map(|s| s.parse::<f64>().ok());
                            if let [Some(x),Some(y)] = numbers {
                                let point = self.tools.plane.map_or(Ok([x,y,0.0]), |plane| plane.to_world([x,y,0.0]));
                                match point {
                                    Ok(point) => if let Err(e) = self.plate_draft.push(point) { self.tool_note = Some(Err(e.to_string())); },
                                    Err(e) => self.tool_note = Some(Err(e.message.into())),
                                }
                            } else { self.tool_note = Some(Err("Enter finite X and Y coordinates.".into())); }
                        }
                    });
                }
                ToolKind::Member => {
                    ui.label(crate::theme::bold(if self.bench == Bench::Column { "Column properties" } else { "Beam properties" }, 16.0));
                    field(ui, "Name", &mut self.tools.member.name, 160.0);
                    field(ui, "Class", &mut self.tools.member.class, 80.0);
                    choice_combo(ui, "member-profile", "Profile", &mut self.tools.member.profile, &self.tools.definitions.profiles, true);
                    choice_combo(ui, "member-material", "Material", &mut self.tools.member.material, &self.tools.definitions.materials, false);
                    ui.separator();
                    ui.label(if self.beam.start().is_some() { "Pick the end point to create the member." } else { "Pick the start point, then the end point in the model view." });
                    if ui.button("Pick new points").clicked() {
                        self.beam.reset(); self.beam_mode = true; self.beam_hover = None;
                        self.renderer.set_ghost(&self.state.device, &self.state.queue, None);
                    }
                    ui.label("Right-click or wheel-click ends this tool.");
                    if self.tools.definitions.profiles.is_empty() { ui.label("Add a profile to the model catalogue before creating a member."); }
                    egui::CollapsingHeader::new("Precise placement").show(ui, |ui| {
                        for (label, point) in [("Start (mm)", &mut self.tools.member.start), ("End (mm)", &mut self.tools.member.end)] {
                            ui.label(label); super::point_fields(ui, point);
                        }
                        field(ui, "Start expression", &mut self.tools.member.start_input, 160.0);
                        field(ui, "End expression", &mut self.tools.member.end_input, 160.0);
                        field(ui, "Roll (radians)", &mut self.tools.member.roll, 80.0);
                        if ui.button("Create member").clicked() { self.create_from_form(); }
                    });
                }
            }
            egui::CollapsingHeader::new("Work plane").show(ui, |ui| {
                match self.plane_form.show(ui) {
                    Ok(Some(crate::work_plane::Action::Apply(plane))) => self.install_plane(Some(plane)),
                    Ok(Some(crate::work_plane::Action::Clear)) => self.install_plane(None),
                    Ok(Some(crate::work_plane::Action::PickFace)) => { self.install_plane(self.tools.plane); self.plane_pick = true; },
                    Err(error) => self.tool_note = Some(Err(error.to_string())),
                    _ => {}
                }
            });
            if ui.button("Cancel").clicked() { self.tools.cancel(); self.set_bench(Bench::Select); self.tool_note = None; }
        });
        if busy {
            ui.spinner();
            ui.label("Saving element…");
        }
        if let Some(note) = &self.tool_note {
            match note {
                Ok(message) => {
                    ui.label(message);
                }
                Err(message) => {
                    ui.colored_label(crate::theme::Palette::current(ui).error, message);
                }
            }
        }
    }
}
