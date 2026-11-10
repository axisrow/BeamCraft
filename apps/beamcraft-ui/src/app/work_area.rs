// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The work area of the model window: up to eight equal view panes drawn inside the central
//! panel, each a child window with a title bar, a frame, move, resize, maximize and close. The
//! topmost pane is the active one: selection, picking, snapping and every tool work in it, and
//! the accessors here read and write its display state.

use super::{
    App, View, annotation_panel, contour_grips, draw_selection, navigation, target_size,
    viewport_sense, workbench,
};
use crate::edit::{ndc_of, ray_through};
use crate::panes;
use crate::theme;
use beamcraft_engine::RenderMode;
use beamcraft_model::id::Uuid7;
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
    /// Every view as a tab of one group, the active one shown.
    Tabs,
    /// One group per view in a square-ish grid: four make 2 x 2.
    Tile,
    /// One group per view, stacked top to bottom.
    TileHorizontal,
    /// One group per view, side by side.
    TileVertical,
    /// Maximize the active group over the work area, or restore the layout.
    ToggleMaximize,
}

/// A view tab being dragged: the key of its pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TabDrag(pub u64);

/// What one tab group asked for this frame, applied after every group is drawn.
pub(super) struct GroupOutcome {
    /// The image of the tab it shows, when the camera made a view.
    draw: Option<PaneDraw>,
    /// A tab whose close glyph was clicked (or middle-clicked).
    close: Option<u64>,
    /// A tab that was clicked or whose body was pressed: it becomes the active pane.
    focus: Option<u64>,
    /// A tab was double-clicked: maximize or restore the group.
    toggle_max: bool,
    /// The tab of the active pane was clicked.
    title_clicked: Option<u64>,
}

/// Width of one tab for `title`, between 96 and 200 points.
fn tab_width(ui: &egui::Ui, title: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        title.to_owned(),
        egui::FontId::proportional(12.0),
        egui::Color32::WHITE,
    );
    (galley.size().x + 44.0).clamp(96.0, 200.0)
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

    /// The tab order the arrangements start from: the active pane first, then the groups' tabs
    /// in layout order, then any pane the layout has not seen yet.
    fn arrange_order(&self) -> Vec<u64> {
        let dock = &self.views.dock;
        let mut order: Vec<u64> = self
            .views
            .active_pane()
            .map(|p| p.key)
            .into_iter()
            .collect();
        for id in dock.order() {
            if let Some(group) = dock.group(id) {
                order.extend(
                    group
                        .tabs
                        .iter()
                        .filter(|k| !order.contains(k))
                        .copied()
                        .collect::<Vec<_>>(),
                );
            }
        }
        for pane in &self.views.panes {
            if !order.contains(&pane.key) {
                order.push(pane.key);
            }
        }
        order.retain(|k| self.views.panes.iter().any(|p| p.key == *k));
        order
    }

    /// Lays the panes out: every view as tabs of one group, one group per view in a grid, in
    /// strips or in columns (the active view first), or the active group maximized and back.
    /// The rects follow at once when the work area has been laid out before.
    pub(super) fn arrange(&mut self, how: Arrange) {
        let order = self.arrange_order();
        let active = self.views.active_pane().map(|p| p.key);
        let dock = &mut self.views.dock;
        match how {
            Arrange::Tabs => dock.arrange_tabs(&order, active),
            Arrange::Tile => dock.arrange_tiles(&order, None, active),
            Arrange::TileHorizontal => dock.arrange_tiles(&order, Some(false), active),
            Arrange::TileVertical => dock.arrange_tiles(&order, Some(true), active),
            Arrange::ToggleMaximize => {
                dock.sync(&order, active);
                dock.toggle_maximize();
            }
        }
        if let Some(area) = self.views.area {
            self.apply_layout(area);
        }
    }

    /// Follows the panes in the tab layout and gives every pane the rect of its group: the tab
    /// its group shows is `shown`, the others wait behind it.
    pub(super) fn apply_layout(&mut self, area: egui::Rect) {
        let keys: Vec<u64> = self.views.panes.iter().map(|p| p.key).collect();
        let active = self.views.active_pane().map(|p| p.key);
        self.views.dock.sync(&keys, active);
        let layout = self.views.dock.layout(area);
        let maximized = self.views.dock.maximized();
        for pane in &mut self.views.panes {
            let group = self.views.dock.group_of(pane.key);
            let rect = group.and_then(|g| layout.iter().find(|(id, _)| *id == g).map(|(_, r)| *r));
            pane.shown = rect.is_some() && self.views.dock.is_shown(pane.key);
            pane.rect = rect.unwrap_or(area);
            pane.placed = true;
            pane.maximized = maximized.is_some() && maximized == group;
        }
    }

    /// The tabbed work area: the dividers, then each tab group with its strip and the image of
    /// the tab it shows, then focus, tab changes, drops and the active pane's input. With no
    /// pane open it offers to open the default view.
    pub(super) fn work_area(&mut self, ui: &mut egui::Ui, zoom_in: bool, zoom_out: bool) {
        let area = ui.available_rect_before_wrap();
        self.views.area = Some(area);
        // Clicks between panes land here and do nothing.
        let _background = ui.interact(area, egui::Id::unique("work-area"), egui::Sense::click());
        if self.views.panes.is_empty() {
            self.views.dock.sync(&[], None);
            self.empty_work_area(ui, area);
            self.work_area_drops(ui, area);
            return;
        }
        self.apply_layout(area);
        self.views.tabs_drawn.clear();
        let active = self.views.active_pane().map(|p| p.key);
        // A press counts only on the central panel's own layer: a press on a ribbon popup, an
        // inspector combo, Details, the publish dialog, the sheet editor or the command search
        // lying over the work area never focuses a pane.
        let layer = ui.layer_id();
        let dividers = self.views.dock.dividers(area);
        let pressed = ui
            .input(|i| {
                i.pointer
                    .primary_pressed()
                    .then(|| i.pointer.interact_pos())
                    .flatten()
            })
            .filter(|pos| ui.ctx().layer_id_at(*pos) == Some(layer))
            // A press on a divider starts a resize, never a group switch.
            .filter(|pos| !dividers.iter().any(|d| d.handle.contains(*pos)))
            .and_then(|pos| panes::topmost_at(&self.views.panes, pos));
        let mut outcomes = Vec::new();
        for (group, rect) in self.views.dock.layout(area) {
            if let Some(outcome) =
                self.group_window(ui, area, group, rect, active, (zoom_in, zoom_out))
            {
                outcomes.push(outcome);
            }
        }
        // The dividers between groups resize them. They are interacted after the groups so
        // the pane under the handle's overlap cannot take the drag first.
        for divider in dividers {
            let response = ui.interact(
                divider.handle,
                egui::Id::unique(("dock-divider", divider.path.clone())),
                egui::Sense::drag(),
            );
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(if divider.across {
                    egui::CursorIcon::ResizeHorizontal
                } else {
                    egui::CursorIcon::ResizeVertical
                });
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(pos) = response.interact_pointer_pos()
            {
                self.views
                    .dock
                    .drag_divider(&divider.path, divider.room, divider.across, pos);
            }
        }
        // 1. Tab changes.
        let mut draws = Vec::new();
        let mut title_clicked = None;
        let mut toggle_max = false;
        let mut focus = None;
        for outcome in outcomes {
            if let Some(key) = outcome.close {
                self.close_pane(key);
            }
            toggle_max |= outcome.toggle_max;
            focus = focus.or(outcome.focus);
            title_clicked = title_clicked.or(outcome.title_clicked);
            if let Some(draw) = outcome.draw {
                draws.push(draw);
            }
        }
        if let Some(key) = focus
            && self.views.panes.iter().any(|p| p.key == key)
        {
            self.focus_pane(key);
            self.views.dock.select(key);
        }
        if toggle_max {
            self.views.dock.toggle_maximize();
        }
        self.work_area_drops(ui, area);
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

    /// Drops on the work area: a dragged view tab moves into a group or beside it, and a view
    /// dragged from the project browser opens there. While something is dragged over the work
    /// area the place it would go is shaded.
    fn work_area_drops(&mut self, ui: &mut egui::Ui, area: egui::Rect) {
        let ctx = ui.ctx().clone();
        let tab = egui::DragAndDrop::payload::<TabDrag>(&ctx).map(|t| t.0);
        let view = egui::DragAndDrop::payload::<Uuid7>(&ctx).map(|v| *v);
        if tab.is_none() && view.is_none() {
            return;
        }
        let Some(pos) = ctx.pointer_latest_pos().filter(|p| area.contains(*p)) else {
            return;
        };
        let target = if self.views.panes.is_empty() {
            None
        } else {
            self.views.dock.drop_at(area, pos)
        };
        let palette = theme::Palette::current(ui);
        let preview = target.map_or(area, |(_, rect)| rect);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("dock-drop"),
        ));
        painter.rect_filled(preview.shrink(2.0), 4, palette.link.gamma_multiply(0.18));
        painter.rect_stroke(
            preview.shrink(2.0),
            4,
            Stroke::new(2.0, palette.link),
            StrokeKind::Inside,
        );
        if !ctx.input(|i| i.pointer.any_released()) {
            return;
        }
        if let Some(key) = tab {
            egui::DragAndDrop::clear_payload(&ctx);
            if let Some((drop, _)) = target {
                self.views.dock.insert(key, drop);
                self.focus_pane(key);
                self.views.dock.select(key);
                self.apply_layout(area);
            }
        } else if let Some(id) = view {
            egui::DragAndDrop::clear_payload(&ctx);
            self.open_view_at(id, target.map(|(drop, _)| drop));
            self.apply_layout(area);
        }
    }

    /// Opens view `id` where `drop` says: its open tab moves there, otherwise a new tab opens
    /// there (`None`: in the active group).
    pub(crate) fn open_view_at(&mut self, id: Uuid7, drop: Option<crate::dock::Drop>) {
        let existing = self
            .views
            .panes
            .iter()
            .rev()
            .find(|p| p.view == Some(id))
            .map(|p| p.key);
        let key = match existing {
            Some(key) => Some(key),
            None => {
                let before = self.views.panes.len();
                self.open_pane(Some(id));
                (self.views.panes.len() > before)
                    .then(|| self.views.panes.last().map(|p| p.key))
                    .flatten()
            }
        };
        let Some(key) = key else {
            return;
        };
        if let Some(drop) = drop {
            self.views.dock.insert(key, drop);
        }
        self.focus_pane(key);
        self.views.dock.select(key);
    }

    /// One tab group: its strip of tabs (click to show, drag to move, × or a middle click to
    /// close, double-click to maximize) and the image of the tab it shows. `None` when the group
    /// is gone.
    fn group_window(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
        group: u64,
        rect: egui::Rect,
        active: Option<u64>,
        zoom: (bool, bool),
    ) -> Option<GroupOutcome> {
        let (tabs, current) = self
            .views
            .dock
            .group(group)
            .map(|g| (g.tabs.clone(), g.current))?;
        let chrome = panes::chrome(rect);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .id_salt(("tab-group", group)),
        );
        child.set_clip_rect(rect.intersect(area).intersect(ui.clip_rect()));
        let palette = theme::Palette::current(ui);
        let holds_active = active.is_some_and(|a| tabs.contains(&a));
        let painter = child.painter().clone();
        painter.rect_filled(chrome.title, 0, palette.head);
        let mut out = GroupOutcome {
            draw: None,
            close: None,
            focus: None,
            toggle_max: false,
            title_clicked: None,
        };
        // The image first, so the tabs win where they meet it.
        if let Some(key) = current {
            let body = child.interact(
                chrome.body,
                egui::Id::unique(("pane-body", key)),
                viewport_sense(),
            );
            let keys = (Some(key) == active).then_some(zoom);
            out.draw = self.draw_pane(&mut child, key, chrome.body, body, keys);
        }
        let mut x = chrome.title.min.x + 2.0;
        for key in &tabs {
            let Some(title) = self
                .views
                .panes
                .iter()
                .find(|p| p.key == *key)
                .map(|p| p.title.clone())
            else {
                continue;
            };
            let width = tab_width(&child, &title);
            let tab = egui::Rect::from_min_size(
                pos2(x, chrome.title.min.y + 2.0),
                vec2(width, chrome.title.height() - 2.0),
            );
            x += width + 2.0;
            // The close glyph sits where the platform's tabs keep it: first on macOS, last
            // elsewhere.
            let close_x = if theme::TAB_CLOSE_LEADING {
                tab.min.x + 12.0
            } else {
                tab.max.x - 12.0
            };
            let close_rect =
                egui::Rect::from_center_size(pos2(close_x, tab.center().y), vec2(16.0, 16.0));
            let response = child.interact(
                tab,
                egui::Id::unique(("view-tab", *key)),
                egui::Sense::click_and_drag(),
            );
            let close = child.interact(
                close_rect,
                egui::Id::unique(("view-tab-close", *key)),
                egui::Sense::click(),
            );
            response.dnd_set_drag_payload(TabDrag(*key));
            self.views.tabs_drawn.push((*key, tab, close_rect));
            let shown = current == Some(*key);
            let fill = if shown && holds_active {
                palette.select
            } else if shown {
                palette.surface
            } else if response.hovered() {
                palette.rule
            } else {
                palette.head
            };
            painter.rect_filled(
                tab,
                CornerRadius {
                    nw: 4,
                    ne: 4,
                    sw: 0,
                    se: 0,
                },
                fill,
            );
            if shown {
                let accent = if holds_active {
                    palette.link
                } else {
                    palette.muted
                };
                painter.line_segment(
                    [
                        tab.left_top() + vec2(3.0, 0.5),
                        tab.right_top() + vec2(-3.0, 0.5),
                    ],
                    Stroke::new(2.0, accent),
                );
            }
            let (text_rect, text_inset) = if theme::TAB_CLOSE_LEADING {
                (
                    egui::Rect::from_min_max(pos2(close_rect.max.x + 2.0, tab.min.y), tab.max),
                    24.0,
                )
            } else {
                (
                    egui::Rect::from_min_max(tab.min, pos2(close_rect.min.x - 2.0, tab.max.y)),
                    8.0,
                )
            };
            let text = painter.with_clip_rect(text_rect.intersect(painter.clip_rect()));
            text.text(
                tab.left_center() + vec2(text_inset, 0.0),
                egui::Align2::LEFT_CENTER,
                &title,
                egui::FontId::proportional(12.0),
                if shown { palette.text } else { palette.muted },
            );
            if close.hovered() {
                painter.rect_filled(close_rect, 3, palette.rule);
            }
            // macOS tabs show their close glyph only under the pointer; elsewhere it stays.
            let glyph = !theme::TAB_CLOSE_LEADING || response.hovered() || close.hovered();
            if glyph {
                let line = Stroke::new(1.0, palette.text);
                let c = close_rect.center();
                painter.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], line);
                painter.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], line);
            }
            if close.clicked() || response.middle_clicked() {
                out.close = Some(*key);
            } else if response.double_clicked() {
                out.toggle_max = true;
            } else if response.clicked() {
                if Some(*key) == active {
                    out.title_clicked = Some(*key);
                }
                out.focus = Some(*key);
            }
            response.on_hover_text(&title);
        }
        let stroke = if holds_active {
            Stroke::new(2.0, palette.link)
        } else {
            Stroke::new(1.0, palette.rule)
        };
        // The frame marks the view itself: it runs round the image, under the tab strip, so
        // the tabs sit above the view as a CAD window's do.
        painter.rect_stroke(chrome.body, 0, stroke, StrokeKind::Inside);
        Some(out)
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
        // A view with filter overrides draws its own scene with their colours and hidden elements.
        let view_overlay = shown
            .and_then(|v| self.views.overlays.get(&v.id))
            .map(|o| (o.key, std::sync::Arc::clone(&o.overlay)));
        let overlay_key = view_overlay.as_ref().map_or(0, |(k, _)| *k);
        let own_scene = hidden != self.views.shown || view_overlay.is_some();
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
            body.height(),
            hovered,
            scroll_y,
            pinch,
            zoom_in,
            zoom_out,
            self.invert_zoom,
        );
        // A contour grip holding the pointer takes the primary drag from the camera (V18): a
        // trackpad's primary drag would otherwise orbit while the grip is being dragged.
        let (orbit, pan) = if self.contour.holds_pointer() {
            ([0.0; 2], [0.0; 2])
        } else {
            (orbit, pan)
        };
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
        // The same clear colours `screenshot-view` answers with (issue 562 item 36): the dark
        // theme sits on the renderer's dark background, the light one on the paper tone.
        let background = if dark {
            crate::render::DARK_BACKGROUND
        } else {
            crate::render::LIGHT_BACKGROUND
        };
        if let Some(renderer) = pane.target.renderer_mut() {
            renderer.background = background;
        }
        if let Some(l) = &self.loaded {
            if own_scene {
                let token = l.snapshot.token().as_str().to_owned();
                let stale = pane
                    .filtered
                    .as_ref()
                    .is_none_or(|(t, h, k, _)| *t != token || *h != hidden || *k != overlay_key);
                if stale {
                    let overlay = view_overlay
                        .as_ref()
                        .map(|(_, o)| std::sync::Arc::clone(o))
                        .or_else(|| self.colour.as_ref().map(|(_, o)| std::sync::Arc::clone(o)));
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
                        Ok(uploaded) => {
                            pane.filtered = Some((token, hidden, overlay_key, uploaded))
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
            }
            let uploaded = match &pane.filtered {
                Some((_, h, k, uploaded)) if own_scene && *h == hidden && *k == overlay_key => {
                    uploaded
                }
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
                // The selected plate's contour grips (V18): dormant while a creation tool or a
                // view pick owns the pointer, and on any bench other than Select, whose click
                // pickers (weld pairs, annotations) would lose their clicks to a grip.
                if let Some(contour) = selected.contour.as_ref()
                    && !self.beam_mode
                    && !self.plate_mode
                    && self.views.picking.is_none()
                    && self.bench == workbench::Bench::Select
                {
                    contour_grips::paint(
                        &painter,
                        response.rect,
                        &view,
                        l.summary.origin_mm,
                        contour,
                        &self.contour,
                    );
                }
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
        // The contour grips (V18) see the pointer before any click: a grip drag is not a
        // selection or a camera move, and its release commits one contour edit. The corner
        // editor's window belongs to the active pane too.
        let grip_hold = !swallow && self.contour_input(ui, response, view);
        self.contour_editor_ui(ui.ctx());
        let finished_creation = !grip_hold && self.finish_creation_gesture(response, view);
        // A closure gesture consumes the click before it can append another point.
        if !swallow
            && !grip_hold
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
        // The Bolt bench previews the row and the holes the form would commit: the engine
        // decides over the snapshot, the paint projects (V19 slice 3). Display only.
        if self.bench == workbench::Bench::Bolt
            && let Some(loaded) = self.loaded.as_ref()
            && let Some(draft) = self.bolt_draft()
        {
            super::bolt_panel::paint_bolt_draft(
                ui,
                response.rect,
                view,
                loaded.summary.origin_mm,
                &draft,
            );
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
            } else if self.bench == workbench::Bench::Bolt {
                self.bolt_click(ray);
            } else if !self.other_command_running() {
                self.start_select(ray);
            }
        }
    }
}
