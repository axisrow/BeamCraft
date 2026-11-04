// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The work area of the model window: up to eight equal view panes drawn inside the central
//! panel, each a child window with a title bar, a frame, move, resize, maximize and close. The
//! topmost pane is the active one: selection, picking, snapping and every tool work in it, and
//! the accessors here read and write its display state.

use super::{
    App, View, annotation_panel, draw_selection, navigation, target_size, viewport_sense, workbench,
};
use crate::edit::{ndc_of, ray_through};
use crate::panes::{self, Edge};
use crate::theme;
use beamcraft_engine::RenderMode;
use egui::{CornerRadius, Stroke, StrokeKind, pos2, vec2};
use std::time::Instant;

/// What a drawn pane hands to the input step.
pub(super) struct PaneDraw {
    /// The pane drawn.
    pub key: u64,
    /// The response of its image.
    pub response: egui::Response,
    /// The camera view it was drawn with.
    pub view: View,
}

/// How the Arrange commands lay the panes out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Arrange {
    /// Cascade from the top-left corner.
    Cascade,
    /// A square-ish grid: four panes make 2 x 2.
    Tile,
    /// Strips stacked top to bottom.
    TileHorizontal,
    /// Columns side by side.
    TileVertical,
    /// Maximize the active pane, or restore it.
    ToggleMaximize,
}

/// Maximizes `pane` over `area`, or restores the rect it had before.
fn toggle_maximized(pane: &mut panes::ViewPane, area: egui::Rect) {
    if pane.maximized {
        pane.maximized = false;
        let back = pane
            .restore
            .take()
            .unwrap_or_else(|| panes::opening_rect(area, 0));
        pane.rect = panes::clamp_into(area, back);
    } else {
        pane.restore = Some(pane.rect);
        pane.maximized = true;
        pane.rect = panes::maximized(area);
    }
}

/// Gives `pane` its arranged rect, out of the maximized state.
fn place(pane: &mut panes::ViewPane, rect: egui::Rect) {
    pane.rect = rect;
    pane.placed = true;
    pane.maximized = false;
    pane.restore = None;
}

/// What one pane's window asked for this frame, applied after every pane is drawn.
pub(super) struct PaneOutcome {
    /// The pane.
    key: u64,
    /// Its drawn image, when the camera made a view.
    draw: Option<PaneDraw>,
    /// The close button was clicked.
    close: bool,
    /// The maximize button was clicked or the title double-clicked.
    toggle_max: bool,
    /// How far the title bar was dragged.
    moved: egui::Vec2,
    /// The resize handle dragged and how far.
    resized: Option<(Edge, egui::Vec2)>,
    /// The title bar was clicked.
    title_clicked: bool,
}

impl App {
    /// Width over height of the active pane's image; 1.6 without a pane.
    pub(super) fn aspect(&self) -> f32 {
        self.views.active_pane().map_or(1.6, |p| p.target.aspect())
    }

    /// The render mode of the active pane.
    pub(super) fn active_mode(&self) -> RenderMode {
        self.views
            .active_pane()
            .map_or(RenderMode::Shaded, |p| p.mode)
    }

    /// Sets the render mode of the active pane; switching never reloads the scene.
    pub(super) fn set_active_mode(&mut self, mode: RenderMode) {
        if let Some(pane) = self.views.active_pane_mut() {
            pane.mode = mode;
        }
    }

    /// Whether the active pane shows its grid guides.
    pub(super) fn grids_shown(&self) -> bool {
        self.views.active_pane().is_some_and(|p| p.grids)
    }

    /// Shows or hides the grid guides of the active pane.
    pub(super) fn set_grids_shown(&mut self, on: bool) {
        if let Some(pane) = self.views.active_pane_mut() {
            pane.grids = on;
        }
    }

    /// Drops the ghost preview of every pane.
    pub(super) fn clear_ghost(&mut self) {
        for pane in &mut self.views.panes {
            if let Some(renderer) = pane.target.renderer_mut() {
                renderer.set_ghost(&self.state.device, &self.state.queue, None);
            }
        }
    }

    /// Hands the ghost preview to the active pane's renderer, the pane the pointer places in.
    pub(super) fn set_active_ghost(&mut self, ghost: Option<&crate::scene::GhostMesh>) {
        if let Some(renderer) = self
            .views
            .panes
            .last_mut()
            .and_then(|p| p.target.renderer_mut())
        {
            renderer.set_ghost(&self.state.device, &self.state.queue, ghost);
        }
    }

    /// Whether the click that focused a pane may also select: Select or View bench, nothing
    /// armed. Otherwise the press only focuses, and the next click places or picks.
    pub(super) fn focus_click_selects(&self) -> bool {
        matches!(
            self.bench,
            workbench::Bench::Select | workbench::Bench::View
        ) && !self.beam_mode
            && !self.plate_mode
            && !self.plane_pick
            && self.views.picking.is_none()
            && self.grid_line.click.is_none()
    }

    /// Lays the panes out over `views.area`; a no-op while it is `None` (no work-area frame yet)
    /// or no pane is open. ToggleMaximize acts on the active pane. Cascade keeps the z-order
    /// bottom first, so the active pane ends up front-most; the tiles go to the active pane
    /// first, then down the z-order. The z-order itself never changes, and every layout but
    /// ToggleMaximize leaves no pane maximized.
    pub(super) fn arrange(&mut self, how: Arrange) {
        let Some(area) = self.views.area else {
            return;
        };
        let count = self.views.panes.len();
        let rects = match how {
            Arrange::ToggleMaximize => {
                if let Some(pane) = self.views.active_pane_mut() {
                    toggle_maximized(pane, area);
                    pane.placed = true;
                }
                return;
            }
            Arrange::Cascade => {
                let rects = panes::cascade(area, count);
                for (pane, rect) in self.views.panes.iter_mut().zip(rects) {
                    place(pane, rect);
                }
                return;
            }
            Arrange::Tile => panes::tile_grid(area, count),
            Arrange::TileHorizontal => panes::tile_horizontal(area, count),
            Arrange::TileVertical => panes::tile_vertical(area, count),
        };
        for (pane, rect) in self.views.panes.iter_mut().rev().zip(rects) {
            place(pane, rect);
        }
    }

    /// The MDI work area: the panes bottom to top, each in its own child Ui (so egui hit-tests
    /// them in z-order), then focus, window changes and the active pane's input. With no pane
    /// open it offers to open the free 3D view.
    pub(super) fn work_area(&mut self, ui: &mut egui::Ui, zoom_in: bool, zoom_out: bool) {
        let area = ui.available_rect_before_wrap();
        self.views.area = Some(area);
        // Clicks between panes land here and do nothing.
        let _background = ui.interact(area, egui::Id::unique("work-area"), egui::Sense::click());
        if self.views.panes.is_empty() {
            self.empty_work_area(ui, area);
            return;
        }
        // Panes opened before any work-area frame (the window's first pane, tests) are placed
        // now: the first one maximized, the others cascaded. Then every pane is kept inside.
        for (index, pane) in self.views.panes.iter_mut().enumerate() {
            if !pane.placed {
                pane.rect = if index == 0 {
                    area
                } else {
                    panes::opening_rect(area, index)
                };
                pane.maximized = index == 0;
                pane.placed = true;
            }
            pane.rect = if pane.maximized {
                area
            } else {
                panes::clamp_into(area, pane.rect)
            };
        }
        let active = self.views.active_pane().map(|p| p.key);
        // A press counts only on the central panel's own layer: a press on a ribbon popup, an
        // inspector combo, Details, the publish dialog, the sheet editor or the command search
        // lying over the work area never focuses a pane.
        let layer = ui.layer_id();
        let pressed = ui
            .input(|i| {
                i.pointer
                    .primary_pressed()
                    .then(|| i.pointer.interact_pos())
                    .flatten()
            })
            .filter(|pos| ui.ctx().layer_id_at(*pos) == Some(layer))
            .and_then(|pos| panes::topmost_at(&self.views.panes, pos));
        // Panes under the topmost maximized one are covered: they are not drawn this frame.
        let covered = self
            .views
            .panes
            .iter()
            .rposition(|p| p.maximized)
            .unwrap_or(0);
        let order: Vec<u64> = self
            .views
            .panes
            .iter()
            .skip(covered)
            .map(|p| p.key)
            .collect();
        let mut outcomes = Vec::new();
        for key in order {
            let is_active = Some(key) == active;
            let keys = is_active.then_some((zoom_in, zoom_out));
            if let Some(outcome) = self.pane_window(ui, area, key, is_active, keys) {
                outcomes.push(outcome);
            }
        }
        // 1. Window changes, by key.
        let mut draws = Vec::new();
        let mut title_clicked = None;
        for outcome in outcomes {
            if outcome.close {
                self.close_pane(outcome.key);
                continue;
            }
            self.change_window(area, &outcome);
            if outcome.title_clicked {
                title_clicked = Some(outcome.key);
            }
            if let Some(draw) = outcome.draw {
                draws.push(draw);
            }
        }
        // 2. Focus on the press; the click its release makes is remembered until then.
        if let Some(key) = pressed
            && Some(key) != active
            && self.views.panes.iter().any(|p| p.key == key)
        {
            self.focus_pane(key);
            self.views.focus_press = Some(key);
        }
        let now = self.views.active_pane().map(|p| p.key);
        // A click on the active pane's title shows its view in the panel, unless a new view's
        // form is open there.
        if title_clicked.is_some()
            && title_clicked == now
            && matches!(
                self.bench,
                workbench::Bench::Select | workbench::Bench::View
            )
            && !(self.bench == workbench::Bench::View
                && self.views.subject == crate::views::Subject::NewView)
        {
            self.show_view_properties();
        }
        // 3. The active pane's input.
        if let Some(draw) = draws.iter().find(|d| Some(d.key) == now) {
            self.pane_input(ui, draw);
        }
        // 4. The focusing press ends with its release.
        if ui.input(|i| i.pointer.primary_released()) {
            self.views.focus_press = None;
        }
    }

    /// The empty work area: a hint and the button that opens the default view.
    fn empty_work_area(&mut self, ui: &mut egui::Ui, area: egui::Rect) {
        let palette = theme::Palette::current(ui);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(area)
                .id_salt("work-area-empty")
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        child.add_space((area.height() / 2.0 - 32.0).max(0.0));
        child.label(egui::RichText::new("No view is open.").color(palette.muted));
        child.add_space(8.0);
        if theme::button(&mut child, theme::Kind::Primary, "Open default view", true).clicked() {
            self.open_pane(None);
        }
    }

    /// Applies a pane's maximize toggle, move or resize; a maximized pane neither moves nor
    /// resizes.
    fn change_window(&mut self, area: egui::Rect, outcome: &PaneOutcome) {
        let Some(pane) = self.views.panes.iter_mut().find(|p| p.key == outcome.key) else {
            return;
        };
        if outcome.toggle_max {
            toggle_maximized(pane, area);
            return;
        }
        if pane.maximized {
            return;
        }
        if outcome.moved != egui::Vec2::ZERO {
            pane.rect = panes::moved(area, pane.rect, outcome.moved);
        }
        if let Some((edge, delta)) = outcome.resized {
            pane.rect = panes::resized(area, pane.rect, edge, delta);
        }
    }

    /// One pane's window: the title bar, the image, the buttons and the resize handles, all in
    /// a child Ui clipped to the pane and the work area. The widgets register body first and
    /// handles last, so the handles win at the borders. `None` when the pane is gone.
    fn pane_window(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
        key: u64,
        active: bool,
        keys: Option<(bool, bool)>,
    ) -> Option<PaneOutcome> {
        let (rect, title, maximized) = self
            .views
            .panes
            .iter()
            .find(|p| p.key == key)
            .map(|p| (p.rect, p.title.clone(), p.maximized))?;
        let chrome = panes::chrome(rect);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("pane", key)));
        child.set_clip_rect(rect.intersect(area).intersect(ui.clip_rect()));
        let palette = theme::Palette::current(ui);
        let (stroke, title_fill) = if active {
            (Stroke::new(2.0, palette.link), palette.select)
        } else {
            (Stroke::new(1.0, palette.rule), palette.head)
        };
        let painter = child.painter().clone();
        painter.rect_filled(
            chrome.title,
            CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            },
            title_fill,
        );
        let body = child.interact(
            chrome.body,
            egui::Id::unique(("pane-body", key)),
            viewport_sense(),
        );
        let draw = self.draw_pane(&mut child, key, chrome.body, body, keys);
        let bar = child.interact(
            chrome.title,
            egui::Id::unique(("pane-title", key)),
            egui::Sense::click_and_drag(),
        );
        let close = child.interact(
            chrome.close,
            egui::Id::unique(("pane-close", key)),
            egui::Sense::click(),
        );
        let maximize = child.interact(
            chrome.maximize,
            egui::Id::unique(("pane-max", key)),
            egui::Sense::click(),
        );
        let mut resized = None;
        for (edge, handle, cursor) in [
            (Edge::Left, chrome.left, egui::CursorIcon::ResizeHorizontal),
            (
                Edge::Right,
                chrome.right,
                egui::CursorIcon::ResizeHorizontal,
            ),
            (Edge::Top, chrome.top, egui::CursorIcon::ResizeVertical),
            (
                Edge::Bottom,
                chrome.bottom,
                egui::CursorIcon::ResizeVertical,
            ),
            (Edge::Corner, chrome.corner, egui::CursorIcon::ResizeNwSe),
        ] {
            let response = child.interact(
                handle,
                egui::Id::unique(("pane-edge", key, edge)),
                egui::Sense::drag(),
            );
            if maximized {
                continue;
            }
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(cursor);
            }
            if response.dragged_by(egui::PointerButton::Primary) {
                resized = Some((edge, response.drag_delta()));
            }
        }
        // The chrome is painted over the image.
        for button in [&close, &maximize] {
            if button.hovered() {
                painter.rect_filled(button.rect, 0, palette.rule);
            }
        }
        painter.rect_stroke(rect, 4, stroke, StrokeKind::Inside);
        let text = painter.with_clip_rect(
            egui::Rect::from_min_max(
                chrome.title.min,
                pos2(chrome.maximize.min.x, chrome.title.max.y),
            )
            .intersect(painter.clip_rect()),
        );
        text.text(
            chrome.title.left_center() + vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(12.0),
            palette.text,
        );
        let line = Stroke::new(1.0, palette.text);
        let c = chrome.close.center();
        painter.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], line);
        painter.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], line);
        let m = chrome.maximize.center();
        if maximized {
            // Restore: two overlapping frames.
            painter.rect_stroke(
                egui::Rect::from_center_size(m + vec2(-1.5, 1.5), vec2(7.0, 7.0)),
                0,
                line,
                StrokeKind::Middle,
            );
            painter.line_segment([m + vec2(-2.0, -5.0), m + vec2(5.0, -5.0)], line);
            painter.line_segment([m + vec2(5.0, -5.0), m + vec2(5.0, 2.0)], line);
        } else {
            painter.rect_stroke(
                egui::Rect::from_center_size(m, vec2(9.0, 9.0)),
                0,
                line,
                StrokeKind::Middle,
            );
        }
        if !maximized {
            // The corner grip: three short diagonals (egui's own grip is private).
            let grip = Stroke::new(1.0, palette.muted);
            let corner = rect.max - vec2(3.0, 3.0);
            for step in [4.0, 7.0, 10.0] {
                painter.line_segment([corner - vec2(step, 0.0), corner - vec2(0.0, step)], grip);
            }
        }
        Some(PaneOutcome {
            key,
            draw,
            close: close.clicked(),
            toggle_max: maximize.clicked() || bar.double_clicked(),
            moved: if !maximized && bar.dragged_by(egui::PointerButton::Primary) {
                bar.drag_delta()
            } else {
                egui::Vec2::ZERO
            },
            resized,
            title_clicked: bar.clicked(),
        })
    }

    /// Draws pane `key` over `body` (points): resizes its target, navigates its camera from
    /// `response` (the zoom keys only when `keys` is given), renders with its mode, the view's
    /// tint and clip and the scene's hidden kinds, then paints the grid guides, annotations and
    /// selection edges. `None` when the pane is gone or its camera makes no view (the error goes
    /// to `self.error`).
    pub(super) fn draw_pane(
        &mut self,
        ui: &mut egui::Ui,
        key: u64,
        body: egui::Rect,
        response: egui::Response,
        keys: Option<(bool, bool)>,
    ) -> Option<PaneDraw> {
        // Everything read from the views is taken before the pane is borrowed mutably.
        let shown = self
            .views
            .panes
            .iter()
            .find(|p| p.key == key)?
            .view
            .and_then(|id| self.views.book.get(&id));
        // The model view tints the model (V12); the free 3D view does not.
        let tint = crate::views::tint_of(shown);
        // The view adds its depth slab and work area to the display cut (V12).
        let cut = crate::views::display_clip(self.cut.as_ref(), shown);
        // The loaded scene follows the active pane's hidden kinds; another pane whose view hides
        // other kinds draws its own filtered upload.
        let hidden = shown.map_or(beamcraft_engine::KindFilter::SHOW_ALL, |v| v.hidden_kinds);
        let own_scene = hidden != self.views.shown;
        let scale = ui.ctx().pixels_per_point();
        let [width, height] = target_size([body.width(), body.height()], scale);
        let dark = ui.ctx().theme() == egui::Theme::Dark;
        let pane = self.views.panes.iter_mut().find(|p| p.key == key)?;
        pane.body = body;
        if pane.target.size() != Some([width, height]) {
            let started = Instant::now();
            // Only the targets follow the size; the shader and the pipeline stay.
            match pane
                .target
                .resize(&self.state, width, height, self.source.samples, &self.stats)
            {
                Ok(_) => self.stats.resize_ns.add_duration(started.elapsed()),
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if let Some(texture) = pane.target.texture() {
            ui.painter().image(
                texture,
                body,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        let hovered = response.hovered();
        let (delta, shift, ctrl, scroll_y, pinch) = ui.input(|i| {
            (
                i.pointer.delta(),
                i.modifiers.shift,
                i.modifiers.ctrl,
                if hovered {
                    i.smooth_scroll_delta.y
                } else {
                    0.0
                },
                if hovered { i.zoom_delta() } else { 1.0 },
            )
        });
        let (zoom_in, zoom_out) = keys.unwrap_or((false, false));
        let (orbit, pan, zoom) = navigation(
            self.preset,
            delta,
            response.dragged_by(egui::PointerButton::Primary),
            response.dragged_by(egui::PointerButton::Secondary),
            response.dragged_by(egui::PointerButton::Middle),
            ctrl,
            shift,
            pane.camera.distance(),
            hovered,
            scroll_y,
            pinch,
            zoom_in,
            zoom_out,
            self.invert_zoom,
        );
        let aspect = pane.target.aspect();
        // Zoom toward the cursor: keep the point under the pointer still while zooming. A
        // hostile cursor or an extreme distance falls back to the view centre.
        let pan = if zoom == 0.0 {
            pan
        } else {
            // hover_pos (not interact_pointer_pos): egui fills the latter only while a
            // button is down on this widget, so wheel and pinch zoom need the hover point.
            let ndc = response.hover_pos().and_then(|pos| {
                ndc_of(
                    [pos.x, pos.y],
                    [response.rect.min.x, response.rect.min.y],
                    [response.rect.width(), response.rect.height()],
                )
            });
            match ndc.map(|n| pane.camera.cursor_zoom_pan(n, aspect, zoom)) {
                Some(Ok(extra)) => [pan[0] + extra[0], pan[1] + extra[1]],
                _ => pan,
            }
        };
        let mut view = match pane
            .camera
            .navigate(orbit, pan, zoom)
            .and_then(|()| pane.camera.view(aspect))
        {
            Ok(view) => view,
            Err(e) => {
                self.error = Some(e.to_string());
                return None;
            }
        };
        view.tint = tint;
        let rgb = if dark {
            [0x17, 0x21, 0x2B]
        } else {
            [0xED, 0xF1, 0xF4]
        };
        let [r, g, b] = rgb.map(|c: u8| f64::from(c) / 255.0);
        if let Some(renderer) = pane.target.renderer_mut() {
            renderer.background = wgpu::Color { r, g, b, a: 1.0 };
        }
        if let Some(l) = &self.loaded {
            if own_scene {
                let token = l.snapshot.token().as_str().to_owned();
                let stale = pane
                    .filtered
                    .as_ref()
                    .is_none_or(|(t, h, _)| *t != token || *h != hidden);
                if stale {
                    let overlay = self.colour.as_ref().map(|(_, o)| std::sync::Arc::clone(o));
                    let built = crate::scene::Scene::from_snapshot_showing(
                        &l.snapshot,
                        beamcraft_engine::Representation::Full,
                        &l.uploaded.resident(),
                        overlay.as_deref(),
                        hidden,
                    )
                    .and_then(|scene| {
                        crate::render::Uploaded::upload(
                            &self.state.device,
                            &self.state.queue,
                            &scene,
                            Some(&l.uploaded),
                        )
                    });
                    match built {
                        Ok(uploaded) => pane.filtered = Some((token, hidden, uploaded)),
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
            }
            let uploaded = match &pane.filtered {
                Some((_, h, uploaded)) if own_scene && *h == hidden => uploaded,
                _ => &l.uploaded,
            };
            // The display cut is uniform state: the scene and its snapshot never change.
            // Planes are model millimetres; the uniform is scene-centred.
            let clip = match &cut {
                Some(cut) => crate::cut_plane::ClipGpu::of(cut, l.summary.origin_mm),
                None => crate::cut_plane::ClipGpu::none(),
            };
            // One loaded scene, five drawing styles: the mode picks the pipeline and never
            // rewrites the snapshot or its token.
            if pane.mode == RenderMode::SelectedOnly {
                // The one engine query runs on a selection or reload change, not per frame;
                // the fields are disjoint, so the cache updates while the scene is borrowed.
                let key = (
                    self.editor
                        .selected()
                        .map(|s| s.id.clone())
                        .unwrap_or_default(),
                    l.summary.origin_mm,
                );
                if pane.selected_only.0 != key {
                    let draw = self
                        .editor
                        .selected()
                        .and_then(|s| beamcraft_engine::ElementId::parse(&s.id).ok())
                        .and_then(|id| {
                            crate::scene::SelectedDraw::of(&l.snapshot, &id, l.summary.origin_mm)
                        });
                    pane.selected_only = (key, draw);
                }
            }
            if let Some(renderer) = pane.target.renderer() {
                match pane.mode {
                    RenderMode::SelectedOnly => match &pane.selected_only.1 {
                        Some(selected) => {
                            renderer.draw_one(
                                &self.state.device,
                                &self.state.queue,
                                &l.uploaded,
                                view,
                                &self.stats,
                                &selected.digest,
                                &selected.instance,
                                &clip,
                            );
                        }
                        // Nothing selected: an empty view, not the whole model.
                        None => {
                            renderer.draw(
                                &self.state.device,
                                &self.state.queue,
                                &l.uploaded,
                                view,
                                &self.stats,
                                RenderMode::SelectedOnly,
                                &clip,
                            );
                        }
                    },
                    mode => {
                        renderer.draw(
                            &self.state.device,
                            &self.state.queue,
                            uploaded,
                            view,
                            &self.stats,
                            mode,
                            &clip,
                        );
                    }
                }
            }
        }
        if let Some(l) = &self.loaded {
            if pane.grids {
                self.grid_scene.paint(
                    ui,
                    response.rect,
                    &view,
                    l.summary.origin_mm,
                    self.grid_edit.as_ref().map(|edit| edit.id.as_str()),
                );
            }
            if let Ok(rows) = self.annotation.rows.as_deref()
                && !rows.is_empty()
            {
                annotation_panel::paint(ui, response.rect, &view, l.summary.origin_mm, rows);
            }
            if let Some(selected) = self.editor.selected() {
                let key = (selected.id.clone(), l.snapshot.token().as_str().to_owned());
                if self.selection_edges.0 != key {
                    let edges = beamcraft_engine::ElementId::parse(&selected.id)
                        .ok()
                        .and_then(|id| {
                            beamcraft_engine::query::feature::element_edges(&l.snapshot, &id).ok()
                        })
                        .unwrap_or_default();
                    self.selection_edges = (key, edges);
                }
                let painter = ui.painter_at(response.rect);
                draw_selection(
                    &painter,
                    response.rect,
                    &view,
                    l.summary.origin_mm,
                    &self.selection_edges.1,
                );
            } else {
                self.selection_edges = Default::default();
            }
        }
        Some(PaneDraw {
            key,
            response,
            view,
        })
    }

    /// The active pane's click, hover previews and tool dispatch: a click selects, picks a grid
    /// line or a view point, or places a point of the armed tool. A pane that is not the active
    /// one takes none of them, and the click that focused the pane only focuses it unless it may
    /// select (`focus_click_selects`).
    pub(super) fn pane_input(&mut self, ui: &mut egui::Ui, draw: &PaneDraw) {
        if self.views.active_pane().map(|p| p.key) != Some(draw.key) {
            return;
        }
        let swallow = self.views.focus_press == Some(draw.key) && !self.focus_click_selects();
        let response = &draw.response;
        let view = &draw.view;
        let mut picked = None;
        let mut picked_grid = None;
        let finished_creation = !swallow && self.finish_creation_gesture(response, view);
        // A closure gesture consumes the click before it can append another point.
        if !swallow
            && !finished_creation
            && response.clicked_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
            && let Some(ndc) = ndc_of(
                [pos.x, pos.y],
                [response.rect.min.x, response.rect.min.y],
                [response.rect.width(), response.rect.height()],
            )
            && let Some(ray) = ray_through(
                view,
                ndc,
                self.loaded
                    .as_ref()
                    .map_or([0.0; 3], |l| l.summary.origin_mm),
            )
        {
            // The grids switch (V11) takes grids out of picking; the display switch stays the
            // pane's grid guides.
            if !self.beam_mode
                && !self.plate_mode
                && self.views.picking.is_none()
                && self.grid_line.click.is_none()
                && self.grids_shown()
                && self
                    .selection
                    .is_on(beamcraft_engine::query::pick::SelectionClass::Grids)
            {
                picked_grid = self
                    .grid_scene
                    .hit_test(
                        response.rect,
                        view,
                        self.loaded
                            .as_ref()
                            .map_or([0.0; 3], |l| l.summary.origin_mm),
                        pos,
                    )
                    .map(|grid| grid.id.clone());
            }
            if picked_grid.is_none() {
                picked = Some(ray);
            }
        }
        if self.plate_mode {
            let resolved = response
                .hover_pos()
                .and_then(|pos| {
                    ndc_of(
                        [pos.x, pos.y],
                        [response.rect.min.x, response.rect.min.y],
                        [response.rect.width(), response.rect.height()],
                    )
                })
                .and_then(|ndc| {
                    ray_through(
                        view,
                        ndc,
                        self.loaded
                            .as_ref()
                            .map_or([0.0; 3], |l| l.summary.origin_mm),
                    )
                })
                .and_then(|ray| self.creation_hit(ray, true).ok());
            self.plate_hover = resolved.map(|(point, _)| point);
            self.snap_hover = resolved.and_then(|(_, kind)| kind);
            self.paint_plate_draft(ui, response.rect, view);
        }
        // The ghost preview follows the pointer on the plane through the armed start, parallel
        // to the view; the committed endpoint comes from the click.
        if self.beam_mode {
            match response.hover_pos().and_then(|pos| {
                ndc_of(
                    [pos.x, pos.y],
                    [response.rect.min.x, response.rect.min.y],
                    [response.rect.width(), response.rect.height()],
                )
            }) {
                Some(ndc) => {
                    self.update_ghost(view, ndc);
                    self.paint_member_draft(ui, response.rect, view);
                }
                None => {
                    self.member_cursor = None;
                    self.snap_hover = None;
                }
            }
        }
        if self.views.picking.is_some() {
            let hover = response.hover_pos().and_then(|pos| {
                ndc_of(
                    [pos.x, pos.y],
                    [response.rect.min.x, response.rect.min.y],
                    [response.rect.width(), response.rect.height()],
                )
                .and_then(|ndc| ray_through(view, ndc, self.view_origin()))
            });
            self.paint_view_picking(ui, response.rect, view, hover);
        }
        if let Some(id) = picked_grid {
            if !self.other_command_running() {
                self.select_grid(&id);
            }
        } else if let Some(ray) = picked {
            if self.views.picking.is_some() {
                self.view_click(ray);
            } else if self.plane_pick {
                if let Some(loaded) = &self.loaded {
                    match beamcraft_engine::work_plane::pick_work_plane(
                        &loaded.snapshot,
                        ray.origin,
                        ray.direction,
                    ) {
                        Ok(Some(plane)) => {
                            self.plane_form.set(plane);
                            self.install_plane(Some(plane));
                        }
                        Ok(None) => {
                            self.tool_note = Some(Err("No face was hit; pick a model face.".into()))
                        }
                        Err(e) => self.tool_note = Some(Err(e.to_string())),
                    }
                }
            } else if self.grid_line.click.is_some() {
                self.place_grid_line(ray);
            } else if self.plate_mode {
                self.place_plate(ray);
            } else if self.beam_mode {
                self.place_beam(ray);
            } else if self.bench == workbench::Bench::Annotations {
                self.annotation_click(ray);
            } else if self.bench == workbench::Bench::Weld {
                self.weld_click(ray);
            } else if !self.other_command_running() {
                self.start_select(ray);
            }
        }
    }
}
