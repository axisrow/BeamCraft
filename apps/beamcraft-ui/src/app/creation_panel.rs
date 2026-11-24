// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Modelling properties and point-driven creation on the existing command adapters.

use super::{App, View, choice_combo, field, ground_point, workbench::Bench};
use crate::tools::{ToolError, ToolKind};
use beamcraft_engine::query::pick::{
    SnapContext, SnapCursor, SnapKind, SnapQuery, snap_with_context,
};

/// A labelled drop-down that keeps one of `options`' tokens in `value`.
/// The sides a plate's thickness is laid off from, as `--thickness-side` tokens and their names.
pub(super) const THICKNESS_SIDES: &[(&str, &str)] =
    &[("top", "Top"), ("middle", "Middle"), ("bottom", "Bottom")];

/// Whether the vertical guide plane through the world XY segment `a`..`b` is seen edge-on
/// along `direction`: the view direction lies in the plane, within about one degree.
fn guide_edge_on(direction: [f64; 3], [ax, ay]: [f64; 2], [bx, by]: [f64; 2]) -> bool {
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx.hypot(dy);
    let [rx, ry, rz] = direction;
    let norm = (rx * rx + ry * ry + rz * rz).sqrt();
    if !(length > 0.0 && length.is_finite() && norm > 0.0 && norm.is_finite()) {
        return false;
    }
    // The guide's horizontal normal is (-dy, dx, 0) / length.
    ((-dy * rx + dx * ry) / (length * norm)).abs() <= 0.02
}

pub(super) fn token_combo(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: &mut String,
    options: &[(&str, &str)],
) {
    let shown = options
        .iter()
        .find(|(token, _)| *token == value.as_str())
        .map_or("", |(_, name)| *name);
    crate::kit::prop_row(ui, label, |ui| {
        egui::ComboBox::from_id_salt(id)
            .width((ui.available_width() - crate::kit::FIT).max(60.0))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (token, name) in options {
                    ui.selectable_value(value, (*token).to_owned(), *name);
                }
            });
    });
}

/// The drop-down of the environment's presets for the active part kind; picking one refills the
/// form (every field stays editable). Nothing is drawn when the environment has no preset.
fn preset_combo(ui: &mut egui::Ui, tools: &mut crate::tools::Tools) {
    let kind = tools.active_preset_kind();
    let options: Vec<(String, String)> = tools
        .presets_of(kind)
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    if options.is_empty() {
        return;
    }
    let chosen = tools.chosen_preset.clone();
    // In the panel's type row, as the view properties show theirs: the chosen preset's name,
    // or "Presets…" once a field was changed by hand.
    let selected = options
        .iter()
        .find(|(id, _)| Some(id) == chosen.as_ref())
        .map_or("Presets…", |(_, name)| name.as_str())
        .to_owned();
    let mut picked = None;
    egui::ComboBox::from_id_salt("part-preset")
        .width((ui.available_width() - crate::kit::FIT).max(80.0))
        .selected_text(selected)
        .truncate()
        .show_ui(ui, |ui| {
            for (id, name) in &options {
                if ui
                    .selectable_label(Some(id) == chosen.as_ref(), name)
                    .clicked()
                {
                    picked = Some(id.clone());
                }
            }
        });
    if let Some(id) = picked {
        tools.use_preset(kind, Some(&id));
    }
}

/// The marker and caption of the snap the pointer resolved to, shared by every point pick.
pub(super) fn paint_snap_marker(
    painter: &egui::Painter,
    pos: egui::Pos2,
    kind: SnapKind,
    color: egui::Color32,
) {
    let rect = egui::Rect::from_center_size(pos, egui::vec2(16.0, 16.0));
    let label = match kind {
        SnapKind::Midpoint => "Midpoint",
        SnapKind::Center => "Axis center",
        SnapKind::GridIntersection => "Grid intersection",
        SnapKind::Edge | SnapKind::Axis | SnapKind::GridLine => "On line",
        SnapKind::Perpendicular => "Perpendicular",
        SnapKind::ConstructionPoint => "Construction point",
        SnapKind::ConstructionCenter => "Construction center",
        SnapKind::ConstructionLine | SnapKind::ConstructionCircle => "On construction line",
        SnapKind::ConstructionPlane => "On construction plane",
        SnapKind::GridEnd => "Grid end",
        SnapKind::AxisEnd => "Axis end",
        SnapKind::Vertex | SnapKind::ConstructionEnd => "Endpoint",
    };
    super::switches::snap_marker(painter, rect, kind, color);
    painter.text(
        pos + egui::vec2(12.0, -14.0),
        egui::Align2::LEFT_BOTTOM,
        label,
        egui::FontId::proportional(12.0),
        color,
    );
}

impl App {
    pub(super) fn finish_creation_gesture(
        &mut self,
        response: &egui::Response,
        view: &View,
    ) -> bool {
        if self.other_command_running() {
            return false;
        }
        // Ctrl+Shift+middle click sets the rotation point; it never finishes a tool.
        let pivot_click = response
            .ctx
            .input(|i| i.modifiers.ctrl && i.modifiers.shift)
            && response.clicked_by(egui::PointerButton::Middle);
        if pivot_click {
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
    /// The last point the running gesture picked, the start of a perpendicular snap: a
    /// plate's last corner, a beam's start, the origin of a copy or move, or the first point
    /// of a bolt group.
    pub(super) fn last_picked_point(&self) -> Option<[f64; 3]> {
        if self.plate_mode {
            return self.plate_draft.points().last().copied();
        }
        if self.beam_mode {
            return self.beam.start();
        }
        if let Some(two) = &self.two_point {
            return two.from;
        }
        if self.bench == super::workbench::Bench::Bolt && self.bolt.point2.is_none() {
            return self.bolt.point1;
        }
        None
    }

    /// Whether the pointer is picking a point outside plate and member placement: a copy or
    /// move by two points, the two points of a bolt group, a hole location, a cut/split plane
    /// point, or a leader endpoint.
    pub(super) fn picking_point(&self) -> bool {
        self.two_point.is_some()
            || (self.bench == super::workbench::Bench::Bolt
                && self.bolt.parts.len() >= 2
                && self.bolt.point2.is_none())
            || self.bench == super::workbench::Bench::Hole
            || (self.bench == super::workbench::Bench::Cut
                && matches!(
                    self.fit.cut_mode,
                    super::fit_panel::CutMode::ByPlane | super::fit_panel::CutMode::Split
                ))
            || (self.bench == super::workbench::Bench::Annotations
                && self.annotation.tool == super::annotation_panel::Tool::Leader
                && self.annotation.leader_end.is_none())
    }

    pub(super) fn creation_point(
        &self,
        ray: crate::edit::Ray,
        planar: bool,
    ) -> Result<[f64; 3], ToolError> {
        self.creation_hit(ray, planar).map(|(point, _)| point)
    }

    /// Whether a grid-line snap lies where the user sees the line. A grid line stands for a
    /// vertical guide plane: seen edge-on (a plan, or an elevation across the line) the whole
    /// guide shows as the line, so any height on it counts. Seen at an angle or face-on (an
    /// elevation along the line) the guide covers the view, and only the line's drawn trace at
    /// Z = 0 counts.
    fn grid_line_seen(
        &self,
        hit: &beamcraft_engine::query::pick::SnapCandidate,
        direction: [f64; 3],
        near: impl Fn([f64; 3]) -> bool,
    ) -> bool {
        use beamcraft_engine::grid_query::{local_to_world, segments};
        let segment = hit.grid.as_ref().and_then(|identity| {
            let grid = self
                .grid_scene
                .query_grids
                .iter()
                .find(|g| g.id == hit.element_id)?;
            let segment = segments(grid)
                .ok()?
                .into_iter()
                .find(|s| s.axis_id == identity.axis_id)?;
            Some((
                local_to_world(grid, segment.start),
                local_to_world(grid, segment.end),
            ))
        });
        if segment.is_some_and(|(a, b)| guide_edge_on(direction, a, b)) {
            return true;
        }
        let [x, y, _] = hit.point_world_mm;
        near([x, y, 0.0])
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
            // An active model view hits only what it shows (V12); the display cut hides in the
            // picture, so picks and snaps skip it too (issue 569 item 17).
            match self.views.active_view() {
                Some(view) => beamcraft_engine::views::pick_visible_except(
                    &loaded.snapshot,
                    view,
                    ray.origin,
                    ray.direction,
                    beamcraft_engine::query::pick::SelectionFilter::ALL,
                    &std::collections::BTreeSet::new(),
                    self.cut.as_ref(),
                    false,
                )
                .map_err(|e| ToolError::local("INVALID_ARGUMENT", e.to_string()))?
                .map(|hit| hit.point_world_mm),
                None => beamcraft_engine::views::pick_kept(
                    &loaded.snapshot,
                    ray.origin,
                    ray.direction,
                    beamcraft_engine::query::pick::SelectionFilter::ALL,
                    self.cut.as_ref(),
                    false,
                )
                .map_err(|e| ToolError::local("INVALID_ARGUMENT", e.to_string()))?
                .map(|hit| hit.point_world_mm),
            }
            // Empty space lands on the active view's own plane, as in Tekla: a beam drawn in a
            // level plan sits at that level, never silently at the ground.
            .or_else(|| {
                self.views
                    .active_view()
                    .filter(|view| !view.is_default)
                    .and_then(|view| view.plane.ray_point(ray.origin, ray.direction))
            })
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
        // The active pane is where the pointer picks: its camera and its image height.
        let pane = self
            .views
            .active_pane()
            .ok_or_else(|| ToolError::local("INVALID_ARGUMENT", "Open a view to pick in."))?;
        let view = pane
            .camera
            .view(pane.target.aspect())
            .map_err(|e| ToolError::local("INVALID_ARGUMENT", e.to_string()))?;
        let viewport_height = pane.body.height().max(1.0);
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
        let mm_per_pixel = 2.0 * scale / f64::from(viewport_height);
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
                    * f64::from(viewport_height)
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
            // The cut's kept half-spaces: a snap target they hide never offers itself (issue
            // 569 item 17), like the view's own slab and work area below.
            let cut_planes = self.cut.as_ref().and_then(|cut| cut.effective().ok());
            // A view that hides the grid kind hides its lines, so it hides its snaps too
            // (issue 569 item 13, the review on the 1111 batch): `grids_shown` is only the
            // pane's guides switch.
            let shown = pane.view.and_then(|id| self.views.book.get(&id));
            let grids = if self.grids_shown()
                && shown.is_none_or(|v| v.shows_kind(beamcraft_engine::ElementKind::Grid))
            {
                self.grid_scene.query_grids.as_slice()
            } else {
                &[]
            };
            let aperture = (20.0 * f64::from(tan) / f64::from(viewport_height)).min(0.1);
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
                // T02: construction objects reach the UI snap context with the UI slice that
                // displays them; the engine and the CLI resolve them already.
                construction: &[],
                plane,
                cursor: Some(cursor),
                sources: self.selection.snap_sources(),
                nearest: self.selection.snap_nearest,
                from: self.last_picked_point(),
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
                    // The active view snaps only to what it shows (V12), and what the window
                    // cut hides never offers itself either (issue 569 item 17).
                    .filter(|hit| {
                        cut_planes.as_ref().is_none_or(|planes| {
                            planes
                                .iter()
                                .all(|plane| plane.distance(hit.point_world_mm) >= 0.0)
                        })
                    })
                    .filter(|hit| {
                        self.views.active_view().is_none_or(|view| {
                            view.contains(hit.point_world_mm)
                                && loaded
                                    .snapshot
                                    .element(&hit.element_id)
                                    .is_none_or(|e| view.shows_kind(e.kind))
                        })
                    })
                    // A grid line seen face-on covers the whole view; it counts only on its trace.
                    .filter(|hit| {
                        hit.kind != SnapKind::GridLine
                            || self.grid_line_seen(hit, ray.direction, |p| {
                                screen_distance(p).is_some_and(|d| d <= 10.0)
                            })
                    })
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

    /// Sets a temporary Ctrl+Click reference origin without placing a part, so the user can
    /// aim and type an offset distance in millimetres before committing the placement point.
    pub(super) fn set_ctrl_reference(&mut self, ray: crate::edit::Ray, planar: bool) {
        if self.other_command_running() {
            return;
        }
        let prev_ref = self.ctrl_reference.take();
        let prev_dist = std::mem::take(&mut self.distance_input);
        let prev_aim = self.distance_aim.take();
        match self.creation_point(ray, planar) {
            Ok(point) => {
                self.ctrl_reference = Some(point);
                self.tool_note = Some(Ok(format!(
                    "Reference {}. Aim and type a distance, or type X-800, Y+500 or Z-200. Enter accepts the point.",
                    super::PointText(point)
                )));
            }
            Err(error) => {
                self.ctrl_reference = prev_ref;
                self.distance_input = prev_dist;
                self.distance_aim = prev_aim;
                self.tool_note = Some(Err(error.to_string()));
            }
        }
    }

    pub(super) fn constrain_creation(
        &self,
        point: [f64; 3],
        kind: Option<SnapKind>,
        ray: crate::edit::Ray,
        planar: bool,
    ) -> Result<([f64; 3], Option<SnapKind>), ToolError> {
        let reference = self.ctrl_reference.or_else(|| {
            if planar {
                self.plate_draft.points().last().copied()
            } else {
                self.beam
                    .start()
                    .or_else(|| self.two_point.as_ref().and_then(|t| t.from))
                    .or_else(|| self.grid_line_start_point())
            }
        });
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
        let resolved = if self.distance_input.is_empty() {
            beamcraft_engine::coordinate::construction_step(
                reference,
                aim,
                None,
                self.selection.orthogonal,
                self.tools.plane,
            )
        } else {
            beamcraft_engine::coordinate::typed_step(
                reference,
                Some(aim),
                &self.distance_input,
                self.selection.orthogonal,
                self.tools.plane,
            )
        }
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
            || !(self.plate_mode
                || self.beam_mode
                || self.two_point.is_some()
                || self.grid_line.click.is_some()
                || self.picking_point())
        {
            return;
        }
        let typing_mode = self.plate_mode
            || self.beam_mode
            || self.two_point.is_some()
            || self.grid_line.click.is_some();
        let has_ref = (self.plate_mode && !self.plate_draft.points().is_empty())
            || (self.beam_mode && self.beam.start().is_some())
            || self.two_point.as_ref().and_then(|t| t.from).is_some()
            || self.grid_line_start_point().is_some()
            || self.ctrl_reference.is_some();
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
                egui::Event::Text(text) if has_ref && typing_mode => {
                    if !text.is_empty()
                        && text.chars().all(|c| {
                            c.is_ascii_digit()
                                || matches!(
                                    c.to_ascii_uppercase(),
                                    'X' | 'Y' | 'Z' | '.' | ',' | '-' | '+'
                                )
                        })
                        && self.distance_input.len().saturating_add(text.len()) <= 64
                    {
                        if self.distance_input.is_empty() {
                            self.distance_aim = if self.plate_mode {
                                self.plate_hover
                            } else {
                                self.beam_hover.or(self.member_cursor)
                            };
                        }
                        self.distance_input.push_str(&text.replace(',', "."));
                    }
                }
                egui::Event::Key {
                    key: egui::Key::Backspace,
                    pressed: true,
                    ..
                } if typing_mode => {
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
                    modifiers,
                    ..
                } if typing_mode => {
                    let chain = modifiers.ctrl || modifiers.command;
                    if self.distance_input.is_empty() {
                        if self.plate_mode && !chain {
                            self.finish_plate();
                        }
                    } else {
                        self.accept_distance(chain);
                    }
                }
                _ => {}
            }
        }
    }

    fn accept_distance(&mut self, chain: bool) {
        let result = (|| {
            let reference = self
                .ctrl_reference
                .or_else(|| {
                    if self.plate_mode {
                        self.plate_draft.points().last().copied()
                    } else if self.beam_mode {
                        self.beam.start()
                    } else if self.grid_line.click == Some(true) {
                        self.grid_line_start_point()
                    } else {
                        self.two_point.as_ref().and_then(|t| t.from)
                    }
                })
                .ok_or_else(|| {
                    "Pick the first point (or Ctrl+click a reference point).".to_owned()
                })?;
            let aim = self.distance_aim.or(if self.plate_mode {
                self.plate_hover
            } else {
                self.beam_hover.or(self.member_cursor)
            });
            let point = beamcraft_engine::coordinate::typed_step(
                reference,
                aim,
                &self.distance_input,
                self.selection.orthogonal,
                self.tools.plane,
            )
            .map_err(|_| {
                "Enter a length in mm along the cursor direction, or an axis offset such as X-800."
                    .to_owned()
            })?;
            if chain {
                self.ctrl_reference = Some(point);
                return Ok::<bool, String>(true);
            }
            if self.plate_mode {
                if self.tools.plate.by_size {
                    self.place_plate_by_size_at(point);
                } else {
                    self.plate_draft.push(point).map_err(|e| e.to_string())?;
                }
            } else if self.beam_mode {
                self.accept_member_point(point);
            } else if self.grid_line.click.is_some() {
                self.accept_grid_point(point);
            } else if self.two_point.is_some() {
                self.accept_two_point(point);
            }
            Ok::<bool, String>(false)
        })();
        match result {
            Ok(chained) => {
                self.distance_input.clear();
                self.distance_aim = None;
                self.plate_hover = None;
                if chained {
                    if let Some(point) = self.ctrl_reference {
                        self.tool_note = Some(Ok(format!(
                            "Reference {}. Aim next offset + Ctrl+Enter, or Enter / click to place (Esc clears).",
                            super::PointText(point)
                        )));
                    }
                } else {
                    self.ctrl_reference = None;
                    if self.plate_mode {
                        self.tool_note = None;
                    }
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
        if self.tools.plate.by_size {
            self.place_plate_by_size(ray);
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

    /// One click by size: the clicked point (snapped) is the centre of the typed rectangle, on
    /// the work plane, else the active view's plane, else level with the point.
    fn place_plate_by_size(&mut self, ray: crate::edit::Ray) {
        match self.creation_point(ray, false) {
            Ok(centre) => self.place_plate_by_size_at(centre),
            Err(error) => self.tool_note = Some(Err(error.to_string())),
        }
    }

    fn place_plate_by_size_at(&mut self, centre: [f64; 3]) {
        let plane = self
            .tools
            .plane
            .or_else(|| {
                self.views
                    .active_view()
                    .filter(|view| !view.is_default)
                    .map(|view| view.plane)
            })
            .unwrap_or_default();
        let result = crate::creation::rectangle_corners(
            centre,
            &plane,
            &self.tools.plate.width,
            &self.tools.plate.length,
        )
        .map_err(|e| ToolError::local("INVALID_ARGUMENT", e.to_string()));
        match result {
            Ok(corners) => {
                self.plate_draft.clear();
                for corner in corners {
                    if let Err(error) = self.plate_draft.push(corner) {
                        self.tool_note = Some(Err(error.to_string()));
                        self.plate_draft.clear();
                        return;
                    }
                }
                self.finish_plate();
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
        self.paint_ctrl_reference(ui, rect, view, self.plate_hover);
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
            paint_snap_marker(&painter, pos, kind, crate::theme::Palette::current(ui).link);
        }
        if let Some(hover) = self.plate_hover
            && let Some(last) = self.plate_draft.points().last()
            && let (Some(a), Some(b)) = (project(*last), project(hover))
        {
            if self.ctrl_reference.is_some()
                || !self.paint_axis_dimension(&painter, [a, b], *last, hover)
            {
                painter.line_segment([a, b], stroke);
            }
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
        self.paint_ctrl_reference(ui, rect, view, self.beam_hover.or(self.member_cursor));
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
        let color = crate::theme::Palette::current(ui).link;
        if let Some(cursor) = self.member_cursor
            && let Some(pos) = project(cursor)
            && let Some(kind) = self.snap_hover
        {
            paint_snap_marker(&ui.painter_at(rect), pos, kind, color);
        }
        if let (Some(start), Some(end)) = (self.beam.start(), self.beam_hover)
            && let (Some(a), Some(b)) = (project(start), project(end))
        {
            let painter = ui.painter_at(rect);
            if self.ctrl_reference.is_none()
                && self.paint_axis_dimension(&painter, [a, b], start, end)
            {
                return;
            }
            painter.line_segment([a, b], egui::Stroke::new(2.0, color));
            painter.circle_filled(a, 4.0, color);
            painter.circle_filled(b, 4.0, color);
        }
    }

    /// Draw an axial distance in the same work frame used to resolve the placement.
    pub(super) fn paint_axis_dimension(
        &self,
        painter: &egui::Painter,
        [a, b]: [egui::Pos2; 2],
        from: [f64; 3],
        to: [f64; 3],
    ) -> bool {
        let typed_axis = self
            .distance_input
            .starts_with(['x', 'X', 'y', 'Y', 'z', 'Z']);
        if !self.selection.orthogonal && !typed_axis {
            return false;
        }
        let frame = self.tools.plane.unwrap_or_default();
        let (Ok([x, y, z]), Ok([tx, ty, tz])) = (frame.to_local(from), frame.to_local(to)) else {
            return false;
        };
        let axes = [
            ("X", tx - x, egui::Color32::from_rgb(245, 88, 86)),
            ("Y", ty - y, egui::Color32::from_rgb(66, 196, 99)),
            ("Z", tz - z, egui::Color32::from_rgb(69, 148, 255)),
        ];
        let mut active = axes.into_iter().filter(|(_, v, _)| v.abs() > 1e-6);
        let Some((axis, value, color)) = active.next() else {
            return false;
        };
        if active.next().is_some() {
            return false;
        }
        painter.add(egui::Shape::dashed_line(
            &[a, b],
            egui::Stroke::new(2.0, color),
            6.0,
            3.0,
        ));
        painter.circle_filled(a, 3.0, color);
        painter.circle_stroke(b, 5.0, egui::Stroke::new(1.5, color));
        painter.text(
            a.lerp(b, 0.5) - egui::vec2(0.0, 8.0),
            egui::Align2::CENTER_BOTTOM,
            format!("{axis} {value:+.1} mm"),
            egui::FontId::proportional(13.0),
            color,
        );
        true
    }

    pub(super) fn paint_ctrl_reference(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        view: &View,
        hover: Option<[f64; 3]>,
    ) {
        let Some(ref_pt) = self.ctrl_reference else {
            return;
        };
        let origin = self
            .loaded
            .as_ref()
            .map_or([0.0; 3], |l| l.summary.origin_mm);
        let project = |point: [f64; 3]| crate::grid::project(point, rect, view, origin);
        let Some(at) = project(ref_pt) else {
            return;
        };
        let painter = ui.painter_at(rect);
        let accent = egui::Color32::from_rgb(0xFF, 0xB3, 0x00);
        let stroke = egui::Stroke::new(1.8, accent);
        painter.rect_stroke(
            egui::Rect::from_center_size(at, egui::vec2(10.0, 10.0)),
            1,
            stroke,
            egui::StrokeKind::Inside,
        );
        painter.line_segment(
            [at - egui::vec2(8.0, 0.0), at + egui::vec2(8.0, 0.0)],
            stroke,
        );
        painter.line_segment(
            [at - egui::vec2(0.0, 8.0), at + egui::vec2(0.0, 8.0)],
            stroke,
        );
        painter.text(
            at + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            "Ref (Ctrl)",
            egui::FontId::proportional(11.0),
            accent,
        );
        if let Some(target) = hover
            && let Some(target_px) = project(target)
        {
            if self.paint_axis_dimension(&painter, [at, target_px], ref_pt, target) {
                return;
            }
            painter.add(egui::Shape::dashed_line(
                &[at, target_px],
                egui::Stroke::new(1.5, accent),
                5.0,
                4.0,
            ));
            let dx = target[0] - ref_pt[0];
            let dy = target[1] - ref_pt[1];
            let dz = target[2] - ref_pt[2];
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            if dist > 1e-3 {
                let mid = egui::pos2(
                    (at.x + target_px.x) * 0.5,
                    (at.y + target_px.y) * 0.5 - 10.0,
                );
                painter.text(
                    mid,
                    egui::Align2::CENTER_BOTTOM,
                    format!("{dist:.0} mm"),
                    egui::FontId::proportional(11.0),
                    accent,
                );
            }
        }
    }

    pub(super) fn paint_distance_entry(&self, ui: &egui::Ui, rect: egui::Rect) {
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

    /// Bottom action bar for `Bench::Plate | Bench::Beam | Bench::Column | Bench::Brace`.
    pub(super) fn creation_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.tools.busy() || self.other_command_running();
        let (primary, enabled) = match self.tools.active {
            ToolKind::Plate if !self.tools.plate.by_size => {
                ("Finish plate", self.plate_draft.points().len() >= 3)
            }
            ToolKind::Plate => ("Create plate", true),
            ToolKind::Member => ("Create member", true),
        };
        let done = crate::kit::actions(ui, primary, !busy && enabled, "Cancel", busy);
        if done.primary {
            match self.tools.active {
                ToolKind::Plate if !self.tools.plate.by_size => self.finish_plate(),
                _ => self.create_from_form(),
            }
        }
        if done.cancel {
            self.tools.cancel();
            self.set_bench(Bench::Select);
            self.tool_note = None;
        }
    }

    pub(super) fn tools_panel(&mut self, ui: &mut egui::Ui) {
        let busy = self.tools.busy() || self.other_command_running();
        ui.add_enabled_ui(!busy, |ui| {
            match self.tools.active {
                ToolKind::Plate => {
                    crate::kit::type_header(ui, crate::theme::Icon::Plate, "Plate", |ui| preset_combo(ui, &mut self.tools));
                    crate::kit::section(ui, "plate", "Part", true, |ui| {
                        field(ui, "Name", &mut self.tools.plate.name, 160.0);
                        super::class_field(ui, &mut self.tools.plate.class);
                        field(ui, "Thickness, mm", &mut self.tools.plate.thickness, 80.0);
                        token_combo(ui, "plate-thickness-side", "Thickness from", &mut self.tools.plate.thickness_side, THICKNESS_SIDES);
                        choice_combo(ui, "plate-material", "Material", &mut self.tools.plate.material, &self.tools.definitions.materials, false);
                    });
                    crate::kit::section(ui, "plate", "Numbering series", false, |ui| {
                        field(ui, "Part prefix", &mut self.numbering.plate_prefix, 80.0);
                        field(ui, "Start number", &mut self.numbering.plate_start, 80.0);
                    });
                    crate::kit::section(ui, "plate", "Placement", true, |ui| {
                    crate::kit::prop_choice(ui, "plate-mode", "Shape", &mut self.tools.plate.by_size, &[(false, "By contour"), (true, "By size")]);
                    if self.tools.plate.by_size {
                        field(ui, "Width, mm", &mut self.tools.plate.width, 80.0);
                        field(ui, "Length, mm", &mut self.tools.plate.length, 80.0);
                        ui.label("Click the centre of the plate (or Ctrl+click a reference point, aim, type offset + Enter). Width runs along the work plane's X, length along its Y.");
                    } else {
                    ui.label("Pick contour vertices in 3D (Ctrl+click sets a reference point for offset). Aim, type a length and press Enter to add a point. O toggles orthogonal X/Y/Z. Enter without a length, right-click, wheel-click or Finish creates the plate.");
                    ui.checkbox(&mut self.selection.orthogonal,"Orthogonal X/Y/Z (O)");
                    ui.label(format!("{} vertices", self.plate_draft.points().len()));
                    // Wrapped: three actions exceed the panel's width, and an unwrapped row
                    // would squeeze the last button into a letter-per-line column.
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(self.plate_draft.points().len() >= 3, egui::Button::new("Finish plate")).clicked() { self.finish_plate(); }
                        if ui.add_enabled(!self.plate_draft.points().is_empty(), egui::Button::new("Remove last point")).clicked() { self.plate_draft.back(); }
                        if ui.add_enabled(self.plate_draft.points().len() >= 3, egui::Button::new("Reverse direction")).clicked() { self.plate_draft.reverse(); }
                    });
                    crate::kit::fold(ui, "plate", "Precise point · work-plane mm", false, |ui| {
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
                    });
                }
                ToolKind::Member => {
                    let column = self.bench == Bench::Column;
                    let brace = self.bench == Bench::Brace;
                    let (icon, kind) = if column {
                        (crate::theme::Icon::Column, "Column")
                    } else if brace {
                        (crate::theme::Icon::Brace, "Brace")
                    } else {
                        (crate::theme::Icon::Beam, "Beam")
                    };
                    crate::kit::type_header(ui, icon, kind, |ui| preset_combo(ui, &mut self.tools));
                    crate::kit::section(ui, "member", "Part", true, |ui| {
                    field(ui, "Name", &mut self.tools.member.name, 160.0);
                    super::class_field(ui, &mut self.tools.member.class);
                    if super::profile_row(
                        ui,
                        "member-profile",
                        &mut self.tools.member.profile,
                        &self.tools.definitions.profiles,
                        true,
                    ) {
                        self.open_profile_catalog(super::profile_catalog::ProfileTarget::MemberTool);
                    }
                    choice_combo(ui, "member-material", "Material", &mut self.tools.member.material, &self.tools.definitions.materials, false);
                    if column {
                        let levels: Vec<crate::tools::Choice> = self.tools.definitions.levels.iter().map(|l| crate::tools::Choice {
                            label: format!("{} · {} mm", l.name, l.elevation_mm),
                            digest: l.id.clone(),
                            rect_mm: None,
                        }).collect();
                        field(ui, "Height, mm", &mut self.tools.member.height, 80.0);
                        choice_combo(ui, "column-top-level", "Top level", &mut self.tools.member.top_level, &levels, false);
                    }
                    });
                    crate::kit::section(ui, "member", "Numbering series", false, |ui| {
                        if column {
                            field(ui, "Part prefix", &mut self.numbering.column_prefix, 80.0);
                            field(ui, "Start number", &mut self.numbering.column_start, 80.0);
                        } else {
                            field(ui, "Part prefix", &mut self.numbering.beam_prefix, 80.0);
                            field(ui, "Start number", &mut self.numbering.beam_start, 80.0);
                        }
                    });
                    crate::kit::section(ui, "member", "Position", true, |ui| {
                        let member = &mut self.tools.member;
                        field(ui, "Rotation, °", &mut member.roll, 80.0);
                        token_combo(ui, "member-plane", "On plane", &mut member.plane, &[("left", "Left"), ("centre", "Middle"), ("right", "Right")]);
                        field(ui, "On plane offset, mm", &mut member.plane_offset, 80.0);
                        token_combo(ui, "member-depth", "At depth", &mut member.depth, &[("front", "Front"), ("centre", "Middle"), ("back", "Behind")]);
                        field(ui, "At depth offset, mm", &mut member.depth_offset, 80.0);
                        field(ui, "Start offset, mm", &mut member.start_offset, 80.0);
                        field(ui, "End offset, mm", &mut member.end_offset, 80.0);
                    });
                    crate::kit::section(ui, "member", "Placement", true, |ui| {
                    ui.label(if column && self.beam.start().is_none() { "Click the base point (or Ctrl+click a reference point, aim, type offset + Enter). Top follows level or height." } else if self.beam.start().is_some() { "Aim, type a length in mm and press Enter, or pick the end point. Ctrl+click sets a reference point. O toggles orthogonal X/Y/Z." } else { "Pick the start point (or Ctrl+click a reference point for offset), then the end point in the model view." });
                    if ui.button("Pick new points").clicked() {
                        self.beam.reset(); self.beam_mode = true; self.beam_hover = None;
                        self.distance_input.clear(); self.distance_aim = None;
                        self.clear_ghost();
                    }
                    ui.label("Right-click or wheel-click ends this tool.");
                    if self.tools.definitions.profiles.is_empty()
                        && ui.button("Open Profile Catalogue…").clicked()
                    {
                        self.open_profile_catalog(super::profile_catalog::ProfileTarget::MemberTool);
                    }
                    crate::kit::fold(ui, "member", "Precise placement", false, |ui| {
                        for (label, point) in [("Start (mm)", &mut self.tools.member.start), ("End (mm)", &mut self.tools.member.end)] {
                            ui.label(label); super::point_fields(ui, point);
                        }
                        field(ui, "Start expression", &mut self.tools.member.start_input, 160.0);
                        field(ui, "End expression", &mut self.tools.member.end_input, 160.0);
                    });
                    });
                }
            }
            // The form draws its own "Work plane" section; wrapping it again nested one
            // collapsible inside another of the same name.
            match self.plane_form.show(ui) {
                Ok(Some(crate::work_plane::Action::Apply(plane))) => self.install_plane(Some(plane)),
                Ok(Some(crate::work_plane::Action::Clear)) => self.install_plane(None),
                Ok(Some(crate::work_plane::Action::PickFace)) => { self.install_plane(self.tools.plane); self.plane_pick = true; },
                Err(error) => self.tool_note = Some(Err(error.to_string())),
                _ => {}
            }
        });
        // Its outcome is told in the status bar (`App::status_note`).
    }
}

#[cfg(test)]
mod tests {
    use super::guide_edge_on;

    #[test]
    fn a_grid_guide_is_a_line_only_when_seen_edge_on() {
        // Grid line "1" runs along Y; grid line "A" runs along X.
        let along_y = ([0.0, 0.0], [0.0, 20000.0]);
        let along_x = ([0.0, 0.0], [18000.0, 0.0]);
        let plan = [0.0, 0.0, -1.0];
        let elevation_along_x = [-1.0, 0.0, 0.0];
        assert!(guide_edge_on(plan, along_y.0, along_y.1), "a plan");
        assert!(guide_edge_on(plan, along_x.0, along_x.1), "a plan");
        // Looking along X: line "A" is edge-on, line "1" fills the view face-on.
        assert!(guide_edge_on(elevation_along_x, along_x.0, along_x.1));
        assert!(!guide_edge_on(elevation_along_x, along_y.0, along_y.1));
        // An oblique 3D view sees no guide edge-on.
        assert!(!guide_edge_on([-0.5, -0.5, -0.7], along_y.0, along_y.1));
        assert!(
            !guide_edge_on(plan, [1.0, 1.0], [1.0, 1.0]),
            "a point is no guide"
        );
    }
}
