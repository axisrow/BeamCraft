// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Modelling properties and point-driven creation on the existing command adapters.

use super::{App, View, choice_combo, field, ground_point, pick_point_in, workbench::Bench};
use crate::tools::{ToolError, ToolKind};
use beamcraft_engine::query::pick::{
    SnapContext, SnapCursor, SnapKind, SnapQuery, snap_with_context,
};

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
        self.creation_hit(ray, planar).map(|(point, _)| point)
    }

    /// Resolve the preview and click through exactly the same coordinate path.
    pub(super) fn creation_hit(
        &self,
        ray: crate::edit::Ray,
        planar: bool,
    ) -> Result<([f64; 3], Option<SnapKind>), ToolError> {
        let draft_plane = planar
            .then(|| {
                beamcraft_engine::coordinate::contour_frame(
                    self.plate_draft.points(),
                    self.tools.plane,
                )
                .ok()
            })
            .flatten();
        let fallback_plane = draft_plane.or(self.tools.plane).unwrap_or_else(|| {
            let origin = self.plate_draft.points().last().copied().unwrap_or([0.; 3]);
            beamcraft_engine::work_plane::WorkPlane::new(origin, [1., 0., 0.], [0., 1., 0.])
                .unwrap_or_default()
        });
        let point = if planar {
            fallback_plane.ray_point(ray.origin, ray.direction)
        } else if let Some(plane) = self.tools.plane {
            plane.ray_point(ray.origin, ray.direction)
        } else if let Some(loaded) = &self.loaded {
            pick_point_in(&loaded.snapshot, &ray)?
                .map(|(point, _)| point)
                .or_else(|| ground_point(&ray))
        } else {
            ground_point(&ray)
        };
        let point = point.ok_or_else(|| {
            ToolError::local(
                "INVALID_ARGUMENT",
                "Aim at the model or the active work plane.",
            )
        })?;
        let plane = if planar {
            draft_plane
        } else {
            self.tools.plane
        };
        let view = self
            .camera
            .view(self.aspect())
            .map_err(|e| ToolError::local("INVALID_ARGUMENT", e.to_string()))?;
        let scene_origin = self
            .loaded
            .as_ref()
            .map_or([0.0; 3], |l| l.summary.origin_mm);
        // Ten logical screen pixels, independent of zoom and monitor scaling. The broad world
        // band includes the foreshortened plane; the final screen test keeps the aperture round.
        let [tan, aspect, _, _] = view.params;
        let orthographic = view.mode.first().is_some_and(|v| *v > 0.5);
        let depth = point
            .iter()
            .zip(ray.origin)
            .zip(view.back)
            .map(|((p, o), b)| (o - p) * f64::from(b))
            .sum::<f64>()
            .abs();
        let scale = if orthographic {
            f64::from(tan)
        } else {
            depth * f64::from(tan)
        };
        let mm_per_pixel = 2.0 * scale / f64::from(self.viewport_height.max(1.0));
        let slope = plane
            .map_or(1.0, |plane| {
                plane
                    .normal()
                    .iter()
                    .zip(ray.direction)
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
                    .abs()
            })
            .max(0.01);
        let radius = (10.0 * mm_per_pixel / slope).clamp(0.001, 1_000_000.0);
        let screen_distance = |p: [f64; 3]| -> Option<f64> {
            let direction_ndc = |basis: &[f32; 4]| {
                ray.direction
                    .iter()
                    .zip(basis)
                    .map(|(a, b)| a * f64::from(*b))
                    .sum::<f64>()
            };
            let forward = -direction_ndc(&view.back);
            if forward <= 0.0 {
                return None;
            }
            let [nx, ny] = if orthographic {
                let origin = std::array::from_fn(|i| {
                    (ray.origin.get(i).copied().unwrap_or(0.0)
                        - scene_origin.get(i).copied().unwrap_or(0.0)) as f32
                });
                let [x, y, _] = view.project(origin);
                [f64::from(x), f64::from(y)]
            } else {
                [
                    direction_ndc(&view.right) / forward / f64::from(tan * aspect),
                    direction_ndc(&view.up) / forward / f64::from(tan),
                ]
            };
            let local = std::array::from_fn(|i| {
                (p.get(i).copied().unwrap_or(0.0) - scene_origin.get(i).copied().unwrap_or(0.0))
                    as f32
            });
            let [x, y, z] = view.project(local);
            if !(0.0..=1.0).contains(&z) {
                return None;
            }
            Some(
                ((f64::from(x) - nx) * f64::from(aspect)).hypot(f64::from(y) - ny)
                    * f64::from(self.viewport_height)
                    * 0.5,
            )
        };
        if planar
            && self.distance_input.is_empty()
            && self.plate_draft.points().len() >= 3
            && let Some(first) = self.plate_draft.points().first()
            && screen_distance(*first).is_some_and(|d| d <= 10.0)
        {
            return Ok((*first, Some(SnapKind::Vertex)));
        }
        let kinds = self.selection.snap_kinds();
        if !kinds.is_empty()
            && let Some(loaded) = &self.loaded
        {
            let grids = if self.grids_visible {
                self.grid_scene.query_grids.as_slice()
            } else {
                &[]
            };
            let aperture = (20.0 * f64::from(tan) / f64::from(self.viewport_height)).min(0.1);
            let cursor = if orthographic {
                SnapCursor::orthographic(
                    ray.origin,
                    ray.direction,
                    (10.0 * mm_per_pixel).clamp(0.001, 1_000_000.0),
                )
            } else {
                SnapCursor::new(ray.origin, ray.direction, aperture)
            }
            .map_err(|e| ToolError::local("INVALID_ARGUMENT", e.to_string()))?;
            let context = SnapContext {
                grids,
                member_axes: &self.grid_scene.member_axes,
                plane,
                cursor: Some(cursor),
                sources: self.selection.snap_sources(),
                nearest: self.selection.snap_nearest,
            };
            // Exact targets get first refusal; a nearby line must not steal an intersection
            // or midpoint just because its perpendicular distance is smaller.
            for discrete in [true, false] {
                let kinds = kinds
                    .iter()
                    .copied()
                    .filter(|kind| {
                        let point_kind =
                            !matches!(kind, SnapKind::Axis | SnapKind::Edge | SnapKind::GridLine);
                        point_kind == discrete
                    })
                    .collect::<Vec<_>>();
                if kinds.is_empty() {
                    continue;
                }
                let result = snap_with_context(
                    &loaded.snapshot,
                    &SnapQuery {
                        point_mm: point,
                        radius_mm: radius,
                        kinds,
                        max_hits: 64,
                    },
                    &context,
                )
                .map_err(|error| ToolError::local("INVALID_ARGUMENT", error.to_string()))?;
                if let Some((hit, _)) = result
                    .candidates
                    .iter()
                    .filter_map(|hit| {
                        screen_distance(hit.point_world_mm)
                            .filter(|d| *d <= 10.0)
                            .map(|d| (hit, d))
                    })
                    .min_by(|(_, a), (_, b)| a.total_cmp(b))
                {
                    return self.constrain_creation(
                        hit.point_world_mm,
                        Some(hit.kind),
                        ray,
                        planar,
                    );
                }
            }
        }
        self.constrain_creation(point, None, ray, planar)
    }

    fn constrain_creation(
        &self,
        point: [f64; 3],
        kind: Option<SnapKind>,
        ray: crate::edit::Ray,
        planar: bool,
    ) -> Result<([f64; 3], Option<SnapKind>), ToolError> {
        let reference = if planar {
            self.plate_draft.points().last().copied()
        } else {
            self.beam.start()
        };
        let Some(reference) = reference else {
            return Ok((point, kind));
        };
        let mut aim = self.distance_aim.unwrap_or(point);
        if self.selection.orthogonal && kind.is_none() && self.distance_aim.is_none() {
            aim = beamcraft_engine::coordinate::orthogonal_cursor(
                reference,
                ray.origin,
                ray.direction,
                self.tools.plane,
            )
            .map_err(|e| ToolError::local("INVALID_ARGUMENT", e.message))?;
        }
        let distance = if self.distance_input.is_empty() {
            None
        } else {
            Some(self.distance_input.parse::<f64>().map_err(|_| {
                ToolError::local(
                    "INVALID_ARGUMENT",
                    "Enter a positive distance in millimetres.",
                )
            })?)
        };
        let resolved = beamcraft_engine::coordinate::construction_step(
            reference,
            aim,
            distance,
            self.selection.orthogonal,
            self.tools.plane,
        )
        .map_err(|e| ToolError::local("INVALID_ARGUMENT", e.message))?;
        let same = resolved
            .iter()
            .zip(point)
            .all(|(a, b)| (a - b).abs() <= 1e-6);
        Ok((resolved, if same { kind } else { None }))
    }

    pub(super) fn construction_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused()
            || self.menu_open
            || self.command_search
            || self.other_command_running()
            || !(self.plate_mode || self.beam_mode)
        {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            match event {
                egui::Event::Key {
                    key: egui::Key::O,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if !modifiers.ctrl && !modifiers.alt && !modifiers.command => {
                    self.selection.orthogonal = !self.selection.orthogonal;
                }
                egui::Event::Text(text)
                    if (self.plate_mode && !self.plate_draft.points().is_empty())
                        || (self.beam_mode && self.beam.start().is_some()) =>
                {
                    if !text.is_empty()
                        && text
                            .chars()
                            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+'))
                        && self.distance_input.len().saturating_add(text.len()) <= 64
                    {
                        if self.distance_input.is_empty() {
                            self.distance_aim = if self.plate_mode {
                                self.plate_hover
                            } else {
                                self.beam_hover
                            };
                        }
                        self.distance_input.push_str(&text.replace(',', "."));
                    }
                }
                egui::Event::Key {
                    key: egui::Key::Backspace,
                    pressed: true,
                    ..
                } if self.plate_mode || self.beam_mode => {
                    if self.distance_input.is_empty() {
                        if self.plate_mode {
                            self.plate_draft.back();
                            self.plate_hover = None;
                        }
                    } else {
                        self.distance_input.pop();
                        if self.distance_input.is_empty() {
                            self.distance_aim = None;
                        }
                    }
                }
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    repeat: false,
                    ..
                } if self.plate_mode || self.beam_mode => {
                    if self.distance_input.is_empty() {
                        if self.plate_mode {
                            self.finish_plate();
                        }
                    } else {
                        self.accept_distance();
                    }
                }
                _ => {}
            }
        }
    }

    fn accept_distance(&mut self) {
        let result = (|| {
            let reference = if self.plate_mode {
                self.plate_draft.points().last().copied()
            } else {
                self.beam.start()
            }
            .ok_or_else(|| "Pick the first point.".to_owned())?;
            let aim = self.distance_aim.ok_or_else(|| {
                "Aim in the next segment direction before entering a distance.".to_owned()
            })?;
            let distance = self
                .distance_input
                .parse::<f64>()
                .map_err(|_| "Enter a positive distance in millimetres.".to_owned())?;
            let point = beamcraft_engine::coordinate::construction_step(
                reference,
                aim,
                Some(distance),
                self.selection.orthogonal,
                self.tools.plane,
            )
            .map_err(|e| e.message.to_owned())?;
            if self.plate_mode {
                self.plate_draft.push(point).map_err(|e| e.to_string())?;
            } else {
                self.accept_member_point(point);
            }
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => {
                self.distance_input.clear();
                self.distance_aim = None;
                self.plate_hover = None;
                if self.plate_mode {
                    self.tool_note = None;
                }
            }
            Err(error) => self.tool_note = Some(Err(error)),
        }
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
                    self.distance_input.clear();
                    self.distance_aim = None;
                }
            }
            Err(error) => self.tool_note = Some(Err(error.to_string())),
        }
    }

    pub(super) fn finish_plate(&mut self) {
        if self.other_command_running() {
            return;
        }
        match self.plate_draft.spatial_contour(self.tools.plane) {
            Ok((contour, plane)) => {
                self.tools.plate_plane = Some(plane);
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
        self.paint_distance_entry(ui, rect);
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
            && let Some(pos) = project(hover)
            && let Some(kind) = self.snap_hover
        {
            let color = crate::theme::Palette::current(ui).link;
            let rect = egui::Rect::from_center_size(pos, egui::vec2(16.0, 16.0));
            let label = match kind {
                SnapKind::Midpoint => "Midpoint",
                SnapKind::Center => "Axis center",
                SnapKind::GridIntersection => "Grid intersection",
                SnapKind::Edge | SnapKind::Axis | SnapKind::GridLine => "On line",
                _ => "Endpoint",
            };
            super::switches::snap_marker(&painter, rect, kind, color);
            painter.text(
                pos + egui::vec2(12.0, -14.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::FontId::proportional(12.0),
                color,
            );
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
        self.paint_distance_entry(ui, rect);
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

    fn paint_distance_entry(&self, ui: &egui::Ui, rect: egui::Rect) {
        if !self.distance_input.is_empty()
            && let Some(pos) = ui.ctx().pointer_hover_pos()
        {
            let palette = crate::theme::Palette::current(ui);
            let painter = ui.painter_at(rect);
            let label = painter.layout_no_wrap(
                format!("{} mm  Enter", self.distance_input),
                egui::FontId::proportional(14.0),
                palette.link,
            );
            let size = label.size() + egui::vec2(12.0, 8.0);
            let anchor = pos + egui::vec2(14.0, 18.0);
            let anchor = egui::pos2(
                anchor.x.min(rect.right() - size.x).max(rect.left()),
                anchor.y.min(rect.bottom() - size.y).max(rect.top()),
            );
            painter.rect_filled(egui::Rect::from_min_size(anchor, size), 3, palette.head);
            painter.galley(anchor + egui::vec2(6.0, 4.0), label, palette.link);
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
                    ui.label("Pick contour vertices in 3D. Aim, type a length and press Enter to add a point. O toggles orthogonal X/Y/Z. Enter without a length, right-click, wheel-click or Finish creates the plate. Backspace removes a digit or point.");
                    ui.checkbox(&mut self.selection.orthogonal,"Orthogonal X/Y/Z (O)");
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
                    ui.label(if self.beam.start().is_some() { "Aim, type a length in mm and press Enter, or pick the end point. O toggles orthogonal X/Y/Z." } else { "Pick the start point, then the end point in the model view." });
                    if ui.button("Pick new points").clicked() {
                        self.beam.reset(); self.beam_mode = true; self.beam_hover = None;
                        self.distance_input.clear(); self.distance_aim = None;
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
