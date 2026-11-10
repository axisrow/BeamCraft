// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The window side of the V12 model views: the browser, the view panel in the inspector, view
//! construction by picking in the model, activation and the 3D/plane toggle, and opening,
//! focusing and closing the view panes of the work area. Everything here is display state: the
//! model is never written, and the engine admits every view before it is kept.

use super::App;
use super::workbench::Bench;
use crate::camera::Camera;
use crate::panes::{RenderTarget, ViewPane};
use crate::theme;
use crate::views::{
    AxisBatch, Construction, Direction, Form, MAX_WINDOWS, Picking, SideChoice, Subject,
};
use beamcraft_engine::RenderMode;
use beamcraft_engine::views::{self, AxisPick, ModelView, PartSide, Projection};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_model::id::Uuid7;
use std::sync::mpsc::TryRecvError;

fn fresh_id() -> Result<Uuid7, String> {
    beamcraft_host::ids::IdSource::system()
        .next_id()
        .map_err(|_| "no view identity could be made".to_owned())
}

/// A labelled text field: the label has a fixed width so the fields of a panel line up, and
/// the field takes the rest of the row.
fn text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    row(ui, label, |ui| {
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width((ui.available_width() - FIT).max(40.0))
                .char_limit(crate::views::FIELD),
        );
    });
}

/// Slack left at the end of a row that fills the panel (the kit's).
const FIT: f32 = crate::kit::FIT;

/// One property row of the properties grid (the kit's).
fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    crate::kit::prop_row(ui, label, add);
}

/// A collapsible group of the properties grid (the kit's group bar).
fn section(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    let _ = crate::kit::section(ui, "view", title, open, add);
}

/// One tab of the project browser: its icon, and its name beside the icon while it is the
/// active tab, so the row stays narrow.
fn browser_tab(ui: &mut egui::Ui, icon: theme::Icon, label: &str, active: bool) -> egui::Response {
    let palette = theme::Palette::current(ui);
    let font = egui::FontId::proportional(12.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, palette.text);
    let width = if active { 40.0 + galley.size().x } else { 32.0 };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 4, palette.head);
    }
    let ink = if active { palette.text } else { palette.muted };
    theme::paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            rect.left_center() + egui::vec2(16.0, -1.0),
            egui::Vec2::splat(18.0),
        ),
        icon,
        ink,
    );
    if active {
        ui.painter().galley(
            egui::pos2(
                rect.left() + 30.0,
                rect.center().y - galley.size().y / 2.0 - 1.0,
            ),
            galley,
            palette.text,
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 4.0, rect.bottom() - 2.0),
                egui::pos2(rect.right() - 4.0, rect.bottom()),
            ),
            1,
            palette.link,
        );
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, active, label));
    response.on_hover_text(label)
}

/// A small icon button of a browser row, shown under the pointer.
fn small_icon(ui: &mut egui::Ui, icon: theme::Icon, tip: &str) -> egui::Response {
    let palette = theme::Palette::current(ui);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3, palette.select);
    }
    theme::paint_icon(
        ui.painter(),
        rect.shrink(3.0),
        icon,
        if response.hovered() {
            palette.text
        } else {
            palette.muted
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    response.on_hover_text(tip)
}

/// The number of views under a browser node, as a small rounded count at the end of its row.
fn count_pill(ui: &mut egui::Ui, count: usize) {
    let palette = theme::Palette::current(ui);
    let text = count.to_string();
    let galley = ui
        .painter()
        .layout_no_wrap(text, egui::FontId::proportional(10.5), palette.muted);
    let size = egui::vec2((galley.size().x + 10.0).max(18.0), 16.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 8, palette.head);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, palette.muted);
}

/// What one row of the project browser shows.
#[derive(Default)]
struct BrowserRow<'a> {
    icon: Option<theme::Icon>,
    name: &'a str,
    selected: bool,
    open: bool,
    favorite: bool,
    temporary: bool,
    default: bool,
}

/// One row: the icon, the name, and on the right the marks of the view (default house,
/// favourite star, temporary clock, open dot).
fn browser_row(ui: &mut egui::Ui, row: BrowserRow<'_>) -> egui::Response {
    let palette = theme::Palette::current(ui);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(
            (ui.available_width() - crate::kit::FIT).max(40.0),
            crate::kit::ROW_HEIGHT,
        ),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter();
    if row.selected {
        painter.rect_filled(rect, 3, palette.select);
    } else if response.hovered() {
        painter.rect_filled(rect, 3, palette.head);
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            3,
            egui::Stroke::new(1.0, palette.link),
            egui::StrokeKind::Inside,
        );
    }
    // An open view carries an accent bar at its left edge, the way an open tab is marked.
    if row.open {
        painter.rect_filled(
            egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(0.0, 4.0),
                egui::vec2(2.0, rect.height() - 8.0),
            ),
            1,
            palette.link,
        );
    }
    let centre = |x: f32| egui::pos2(x, rect.center().y);
    let ink = if row.selected {
        palette.text
    } else {
        palette.muted
    };
    if let Some(icon) = row.icon {
        theme::paint_icon(
            painter,
            egui::Rect::from_center_size(centre(rect.left() + 14.0), egui::Vec2::splat(16.0)),
            icon,
            ink,
        );
    }
    let mut right = rect.right() - 6.0;
    let mut mark = |icon: theme::Icon, colour: egui::Color32| {
        theme::paint_icon(
            painter,
            egui::Rect::from_center_size(centre(right - 6.0), egui::Vec2::splat(12.0)),
            icon,
            colour,
        );
        right -= 16.0;
    };
    if row.favorite {
        mark(theme::Icon::Star, palette.link);
    }
    if row.temporary {
        mark(theme::Icon::Recent, palette.muted);
    }
    let colour = if row.temporary && !row.selected {
        palette.muted
    } else {
        palette.text
    };
    painter
        .with_clip_rect(egui::Rect::from_min_max(
            rect.min,
            egui::pos2(right - 2.0, rect.max.y),
        ))
        .text(
            egui::pos2(rect.left() + 28.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            row.name,
            egui::FontId::proportional(12.0),
            colour,
        );
    let mut hint = row.name.to_owned();
    if row.default {
        hint.push_str(" — the default view");
    }
    if row.temporary {
        hint.push_str(" — temporary, not saved");
    }
    if row.open {
        hint.push_str(" — open");
    }
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, row.selected, &hint));
    response.on_hover_text(hint)
}

fn axis_combo(ui: &mut egui::Ui, id: &str, label: &str, value: &mut AxisPick) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(id)
            .selected_text(value.as_str())
            .show_ui(ui, |ui| {
                for pick in [
                    AxisPick::None,
                    AxisPick::First,
                    AxisPick::Last,
                    AxisPick::All,
                ] {
                    ui.selectable_value(value, pick, pick.as_str());
                }
            });
    });
}

impl App {
    /// The display origin of the loaded scene, model millimetres.
    pub(super) fn view_origin(&self) -> [f64; 3] {
        self.loaded
            .as_ref()
            .map_or([0.0; 3], |l| l.summary.origin_mm)
    }

    /// The scene diagonal, the reach of an orthographic eye.
    fn view_extent(&self) -> f32 {
        self.loaded.as_ref().map_or(1000.0, |l| {
            let [a, b, c] = l.summary.min;
            let [d, e, f] = l.summary.max;
            ((d - a).hypot(e - b).hypot(f - c) * 2.0).max(1000.0)
        })
    }

    fn view_note(&mut self, result: Result<String, String>) {
        self.views.note = Some(result);
    }

    /// Writes the book beside the store; a failure is a note.
    pub(super) fn persist_views(&mut self) {
        if let Err(e) = crate::views::persist(&self.source.store, &self.views.book) {
            self.view_note(Err(e));
        }
    }

    /// The default view's identity: the view a model opens on, and the one that stands for
    /// "no particular view" (`None`) wherever a view is asked for.
    pub(crate) fn default_view_id(&self) -> Option<Uuid7> {
        self.views.book.default_view().map(|v| v.id)
    }

    /// A model always has a default view. When the book has none (a new model, or a file from
    /// before the default existed) a "3D View" is made from the first pane's camera and saved.
    pub(crate) fn ensure_default_view(&mut self) {
        if self.views.book.default_view().is_some() {
            return;
        }
        let origin = self.view_origin();
        let (plane, span, orthographic) = self
            .views
            .active_pane()
            .and_then(|p| {
                p.camera
                    .frame(origin)
                    .ok()
                    .map(|(plane, span)| (plane, span, p.camera.is_orthographic()))
            })
            .unwrap_or((
                WorkPlane::default(),
                beamcraft_engine::views::DEFAULT_SPAN_MM,
                false,
            ));
        let id = match fresh_id() {
            Ok(id) => id,
            Err(e) => return self.view_note(Err(e)),
        };
        let name = self.views.book.unique_name("3D View", &[]);
        let mut view = ModelView::new(id, &name, plane, span);
        view.projection = if orthographic {
            Projection::Orthographic
        } else {
            Projection::Perspective
        };
        view.is_default = true;
        match self.views.book.insert(view) {
            Ok(()) => self.persist_views(),
            Err(e) => self.view_note(Err(e.to_string())),
        }
    }

    /// Puts the first pane, which opens as a free camera, on the default view.
    pub(crate) fn adopt_default_view(&mut self) {
        let Some(view) = self.views.book.default_view().cloned() else {
            return;
        };
        if let Some(pane) = self.views.panes.iter_mut().find(|p| p.view.is_none()) {
            pane.mode = view.render_mode;
            pane.grids = view.grids_visible;
            pane.set_view(Some(&view));
        }
        if !self.views.open.contains(&view.id) {
            self.views.open.push(view.id);
        }
    }

    /// The input frame a view installs: its plane, except for the default view, which leaves the
    /// model's work plane as it is (a 3D view has no plane of its own to work on).
    fn input_plane_of(view: &ModelView) -> Option<Option<WorkPlane>> {
        (!view.is_default).then_some(Some(view.plane))
    }

    /// Shows `id`, or the default view for `None`: a pane that shows it already is focused;
    /// otherwise the active pane switches to it, and with no pane open one opens. The camera of a
    /// view opened before comes back as the user left it; its render mode, grid guides, hidden
    /// kinds and plane (the input frame) come from the view.
    pub(crate) fn activate_view(&mut self, id: Option<Uuid7>) {
        let default = self.default_view_id();
        let id = id.or(default);
        if self.views.active_pane().is_some_and(|p| p.view == id) {
            return;
        }
        if let Some(key) = self
            .views
            .panes
            .iter()
            .rev()
            .find(|p| p.view == id)
            .map(|p| p.key)
        {
            self.focus_pane(key);
            return;
        }
        if self.views.panes.is_empty() {
            self.open_pane(id);
            return;
        }
        let input = self.tools.plane;
        let Some((current, camera, mode, grids)) = self
            .views
            .active_pane()
            .map(|p| (p.view, p.camera, p.mode, p.grids))
        else {
            return;
        };
        let leaving_free = current.is_none() || current == default;
        match current {
            Some(current) => {
                self.views.cameras.insert(current, camera);
            }
            None => {
                if let Some(pane) = self.views.active_pane_mut() {
                    pane.spatial = Some((camera, mode, grids));
                }
            }
        }
        if let Some(pane) = self.views.active_pane_mut() {
            pane.toggled = None;
        }
        match id.and_then(|id| self.views.book.get(&id).cloned()) {
            Some(view) => {
                let camera = match self.views.cameras.get(&view.id) {
                    Some(camera) => Ok(*camera),
                    None => Camera::from_view(&view, self.view_origin(), self.view_extent()),
                };
                let camera = match camera {
                    Ok(camera) => camera,
                    Err(e) => {
                        self.view_note(Err(e.to_string()));
                        return;
                    }
                };
                if let Some(pane) = self.views.active_pane_mut() {
                    pane.camera = camera;
                    pane.mode = view.render_mode;
                    pane.grids = view.grids_visible;
                    pane.set_view(Some(&view));
                }
                if !self.views.open.contains(&view.id) {
                    self.views.open.push(view.id);
                }
                match Self::input_plane_of(&view) {
                    Some(plane) => {
                        if leaving_free {
                            self.views_input = Some(input);
                        }
                        self.install_plane(plane);
                    }
                    None => {
                        if let Some(plane) = self.views_input.take() {
                            self.install_plane(plane);
                        }
                    }
                }
                self.tool_note = None;
            }
            None => {
                if let Some(pane) = self.views.active_pane_mut() {
                    pane.set_view(None);
                    if let Some((camera, mode, grids)) = pane.spatial.take() {
                        pane.camera = camera;
                        pane.mode = mode;
                        pane.grids = grids;
                    }
                }
                if let Some(plane) = self.views_input.take() {
                    self.install_plane(plane);
                    self.tool_note = None;
                }
            }
        }
        if self.views.shown != self.views.active_hidden() {
            self.recolour();
        }
    }

    /// Ctrl+P: looks straight at the active view's plane (or the work plane), and back again.
    pub(crate) fn toggle_view_plane(&mut self) {
        if let Some(pane) = self.views.active_pane_mut()
            && let Some(previous) = pane.toggled.take()
        {
            pane.camera = previous;
            return;
        }
        let Some(camera) = self.views.active_pane().map(|p| p.camera) else {
            return;
        };
        let origin = self.view_origin();
        let plane = self
            .views
            .active_view()
            .filter(|v| !v.is_default)
            .map(|v| v.plane)
            .or(self.tools.plane)
            .unwrap_or_default();
        let span = camera.frame(origin).map_or(10_000.0, |(_, s)| s);
        match Camera::aligned_to(plane, origin, span, self.view_extent()) {
            Ok(aligned) => {
                if let Some(pane) = self.views.active_pane_mut() {
                    pane.toggled = Some(pane.camera);
                    pane.camera = aligned;
                }
            }
            Err(e) => self.view_note(Err(e.to_string())),
        }
    }

    /// One of the six directional cameras about the current target; the input plane stays.
    pub(crate) fn orient_view(&mut self, direction: Direction) {
        let Some(camera) = self.views.active_pane().map(|p| p.camera) else {
            return;
        };
        let origin = self.view_origin();
        let Ok((frame, span)) = camera.frame(origin) else {
            return;
        };
        let aligned = direction.plane(frame.origin()).and_then(|plane| {
            Camera::aligned_to(plane, origin, span, self.view_extent()).map_err(|e| e.to_string())
        });
        match aligned {
            Ok(aligned) => {
                if let Some(pane) = self.views.active_pane_mut() {
                    if pane.toggled.is_none() {
                        pane.toggled = Some(pane.camera);
                    }
                    pane.camera = aligned;
                }
            }
            Err(e) => self.view_note(Err(e)),
        }
    }

    /// A form for a new view of `construction` that starts from the active pane's camera: its
    /// height, and for the current camera also its projection, render mode and grids.
    fn camera_form(&self, construction: Construction) -> Form {
        let mut form = Form {
            construction,
            name: self.views.book.next_default_name(),
            ..Form::default()
        };
        let pane = self
            .views
            .active_pane()
            .map(|p| (p.camera, p.mode, p.grids));
        if let Some((camera, _, _)) = pane
            && let Ok((_, span)) = camera.frame(self.view_origin())
        {
            form.span = format!("{}", span.round().max(1.0));
        }
        if construction == Construction::CurrentCamera
            && let Some((camera, mode, grids)) = pane
        {
            form.projection = if camera.is_orthographic() {
                Projection::Orthographic
            } else {
                Projection::Perspective
            };
            form.mode = mode;
            form.grids = grids;
        }
        form
    }

    /// The active pane's current visible height, from its camera. The panel's height field
    /// follows this while it is not being edited, so the form never holds a height the user
    /// has zoomed away from and Apply cannot re-zoom the viewport with it.
    fn live_span(&self) -> Option<String> {
        let pane = self.views.active_pane()?;
        let (_, span) = pane.camera.frame(self.view_origin()).ok()?;
        Some(format!("{}", span.round().max(1.0)))
    }

    /// Opens the view panel on a new view built by `construction`; the constructions that pick
    /// in the model start picking at once, and Apply creates the view.
    pub(crate) fn begin_view(&mut self, construction: Construction) {
        self.set_bench(Bench::View);
        if self.bench != Bench::View {
            self.view_note(Err("Wait for the running command to finish.".into()));
            return;
        }
        let mut form = self.camera_form(construction);
        if matches!(construction, Construction::GridAxes | Construction::Level) {
            form.name = match construction {
                Construction::Level => "Plan".into(),
                _ => "Grid".into(),
            };
        }
        self.views.form = form;
        self.views.subject = Subject::NewView;
        self.views.picking = (construction.picks() > 0).then(|| Picking {
            construction,
            ..Picking::default()
        });
    }

    /// Opens the view panel on an existing view; Apply updates it.
    pub(crate) fn edit_view(&mut self, id: Uuid7) {
        let Some(form) = self.views.book.get(&id).map(Form::from_view) else {
            return;
        };
        // The bench stays while another command runs (`set_bench`); the panel then stays shut.
        self.set_bench(Bench::View);
        if self.bench != Bench::View {
            return;
        }
        self.views.form = form;
        self.views.subject = Subject::SavedView(id);
        self.views.picking = None;
    }

    /// Shows the active pane's view in the panel: its properties, or the free 3D view's display.
    /// A panel that already shows that view (or that free pane) is left as it is, so unsaved edits
    /// in the form survive another empty click.
    pub(crate) fn show_view_properties(&mut self) {
        let pane = self.views.active_pane().map(|p| p.key);
        let active = self.views.active();
        let same = self.bench == Bench::View
            && match (self.views.subject, active) {
                (Subject::SavedView(shown), Some(id)) => shown == id,
                (Subject::FreeView(shown), None) => shown == pane,
                _ => false,
            };
        if same {
            return;
        }
        match active {
            Some(id) => self.edit_view(id),
            None => {
                self.set_bench(Bench::View);
                if self.bench != Bench::View {
                    return;
                }
                self.views.form = self.camera_form(Construction::CurrentCamera);
                self.views.picking = None;
                self.views.subject = Subject::FreeView(pane);
            }
        }
    }

    /// The point a view pick takes for `ray`: snapped like a creation point, and on the picked
    /// face for the edge of a surface view.
    fn view_point(&self, ray: crate::edit::Ray) -> Option<[f64; 3]> {
        let point = self.creation_hit(ray, false).ok().map(|(p, _)| p)?;
        match self.views.picking.as_ref().and_then(|p| p.face) {
            Some(face) => {
                let n = face.normal();
                let o = face.origin();
                let d: f64 = point
                    .iter()
                    .zip(o)
                    .zip(n)
                    .map(|((p, o), n)| (p - o) * n)
                    .sum();
                let [x, y, z] = point;
                let [a, b, c] = n;
                Some([x - a * d, y - b * d, z - c * d])
            }
            None => Some(point),
        }
    }

    /// One click while a view pick runs: a face, then points. The last pick fills the plane;
    /// Apply builds the view.
    pub(crate) fn view_click(&mut self, ray: crate::edit::Ray) {
        let Some(mut picking) = self.views.picking.clone() else {
            return;
        };
        let needs_face = matches!(
            picking.construction,
            Construction::Surface | Construction::SurfaceEdge
        ) && picking.face.is_none();
        if needs_face {
            let Some(loaded) = &self.loaded else { return };
            match views::surface_plane(
                &loaded.snapshot,
                self.views.active_view(),
                ray.origin,
                ray.direction,
                None,
            ) {
                Ok(face) => picking.face = Some(face),
                Err(e) => {
                    self.view_note(Err(format!("{e} Pick a planar face.")));
                    return;
                }
            }
        } else {
            match self.view_point(ray) {
                Some(point) => picking.points.push(point),
                None => {
                    self.view_note(Err("Aim at the model or the work plane.".into()));
                    return;
                }
            }
        }
        if picking.done() < picking.construction.picks() {
            self.views.picking = Some(picking);
            self.views.note = None;
            return;
        }
        match self.picked_plane(&picking) {
            Ok(plane) => {
                self.views.form.plane = plane;
                self.views.form.picked = true;
                self.views.picking = None;
                self.view_note(Ok("Picked. Review the properties and press Apply.".into()));
            }
            Err(e) => {
                // A degenerate last point is dropped; the earlier points stay.
                picking.undo();
                self.views.picking = Some(picking);
                self.view_note(Err(format!("{e} Pick a different point.")));
            }
        }
    }

    fn picked_plane(&self, picking: &Picking) -> Result<WorkPlane, String> {
        let input = self.tools.plane.unwrap_or_default();
        let plane = match (picking.construction, picking.points.as_slice()) {
            (Construction::TwoPoints, [a, b]) => views::two_points_in_plane(*a, *b, input),
            (Construction::ThreePoints, [a, b, c]) => views::three_points(*a, *b, *c),
            (Construction::Surface, []) => {
                return picking.face.ok_or_else(|| "pick a face".to_owned());
            }
            (Construction::SurfaceEdge, [a, b]) => {
                let face = picking.face.ok_or_else(|| "pick a face".to_owned())?;
                let x: Vec<f64> = b.iter().zip(a).map(|(b, a)| b - a).collect();
                let [x0, x1, x2] = <[f64; 3]>::try_from(x).map_err(|_| "bad edge".to_owned())?;
                let [n0, n1, n2] = face.normal();
                WorkPlane::new(
                    *a,
                    [x0, x1, x2],
                    [n1 * x2 - n2 * x1, n2 * x0 - n0 * x2, n0 * x1 - n1 * x0],
                )
            }
            _ => return Err("the view needs more picks".into()),
        };
        plane.map_err(|e| e.to_string())
    }

    /// Builds or updates the views the form describes and keeps them.
    pub(crate) fn submit_view(&mut self) {
        if self.views.job.is_some() {
            return;
        }
        // An environment filter the form names comes into the model first.
        for o in self.views.form.overrides.clone() {
            if let Err(e) = self.adopt_filter(&o.filter) {
                return self.view_note(Err(e));
            }
        }
        if self.views.picking.is_some() {
            self.view_note(Err("Pick the points in the model first.".into()));
            return;
        }
        let result = match self.views.form.id {
            Some(id) => self.update_view_from_form(id),
            None => self.create_views_from_form(),
        };
        match result {
            Ok(Some(note)) => {
                self.persist_views();
                if !matches!(self.views.note, Some(Err(_))) {
                    self.view_note(Ok(note));
                }
            }
            Ok(None) => {}
            Err(e) => self.view_note(Err(e)),
        }
    }

    fn update_view_from_form(&mut self, id: Uuid7) -> Result<Option<String>, String> {
        let mut view = self
            .views
            .book
            .get(&id)
            .cloned()
            .ok_or_else(|| "the view is gone".to_owned())?;
        view.plane = self.views.form.plane;
        self.views.form.apply(&mut view)?;
        self.views.book.update(view).map_err(|e| e.to_string())?;
        // The camera follows the new definition; a session camera would hide the change.
        self.views.cameras.remove(&id);
        let Some(view) = self.views.book.get(&id).cloned() else {
            return Err("the view is gone".to_owned());
        };
        let camera = Camera::from_view(&view, self.view_origin(), self.view_extent());
        if let Err(e) = &camera {
            self.view_note(Err(e.to_string()));
        }
        // Every pane on the view takes its new display, camera and name.
        for pane in self.views.panes.iter_mut().filter(|p| p.view == Some(id)) {
            if let Ok(camera) = &camera {
                pane.camera = *camera;
                pane.toggled = None;
            }
            pane.mode = view.render_mode;
            pane.grids = view.grids_visible;
            pane.set_view(Some(&view));
            pane.filtered = None;
        }
        if self.views.active() == Some(id) {
            if let Some(plane) = Self::input_plane_of(&view) {
                self.install_plane(plane);
            }
            self.tool_note = None;
            if self.views.shown != self.views.active_hidden() {
                self.recolour();
            }
        }
        Ok(Some("The view was updated; the model is unchanged.".into()))
    }

    fn create_views_from_form(&mut self) -> Result<Option<String>, String> {
        let mut form = self.views.form.clone();
        // Without a name a view is temporary. The book still needs a name, so it gets a
        // default one; a batch of grid families is named by its prefixes instead.
        let batch_named =
            form.construction == Construction::GridAxes && form.axis_batch == AxisBatch::Families;
        let unnamed = form.name.trim().is_empty() && !batch_named;
        if unnamed && form.default {
            return Err("The default view needs a name.".into());
        }
        if unnamed {
            if matches!(
                form.construction,
                Construction::Level | Construction::Assembly
            ) {
                return Err("These views are saved with the model: give them a name.".into());
            }
            form.name = self.views.book.next_default_name();
        }
        let origin = self.view_origin();
        let single =
            |plane: WorkPlane| -> Vec<(String, WorkPlane, Option<f64>, Option<Projection>)> {
                vec![(form.name.trim().to_owned(), plane, None, None)]
            };
        let planned = match form.construction {
            Construction::PlanXy => single(
                views::coordinate_plane("xy", form.plane_offset()?).map_err(|e| e.to_string())?,
            ),
            Construction::ElevationXz => single(
                views::coordinate_plane("xz", form.plane_offset()?).map_err(|e| e.to_string())?,
            ),
            Construction::ElevationYz => single(
                views::coordinate_plane("yz", form.plane_offset()?).map_err(|e| e.to_string())?,
            ),
            Construction::TwoPoints
            | Construction::ThreePoints
            | Construction::Surface
            | Construction::SurfaceEdge => {
                // A cancelled pick leaves no plane: never a default one.
                if !form.picked {
                    return Err("Pick the points in the model first.".into());
                }
                single(form.plane)
            }
            Construction::WorkPlane => single(self.tools.plane.unwrap_or_default()),
            Construction::CurrentCamera => {
                let camera = self
                    .views
                    .active_pane()
                    .map(|p| p.camera)
                    .ok_or_else(|| "Open a view to take its camera.".to_owned())?;
                let (plane, _) = camera.frame(origin).map_err(|e| e.to_string())?;
                single(plane)
            }
            Construction::GridAxes => {
                let grid = self
                    .grid_scene
                    .query_grids
                    .get(form.grid)
                    .ok_or_else(|| "The model has no grid to place views on.".to_owned())?;
                match form.axis_batch {
                    AxisBatch::Single => {
                        single(views::grid_axis(grid, form.axis.trim()).map_err(|e| e.to_string())?)
                    }
                    AxisBatch::Families => {
                        let [u, v] = form.families;
                        views::grid_views(grid, u, v)
                            .map_err(|e| e.to_string())?
                            .into_iter()
                            .map(|axis| {
                                let prefix = form
                                    .prefixes
                                    .get(usize::from(axis.family))
                                    .map_or("", |p| p.trim());
                                let name = if prefix.is_empty() {
                                    axis.label.clone()
                                } else {
                                    format!("{prefix} {}", axis.label)
                                };
                                (name, axis.plane, None, None)
                            })
                            .collect()
                    }
                }
            }
            Construction::Part => {
                let loaded = self
                    .loaded
                    .as_ref()
                    .ok_or_else(|| "No model is loaded.".to_owned())?;
                let id = self
                    .editor
                    .selected()
                    .and_then(|s| beamcraft_engine::ElementId::parse(&s.id).ok())
                    .ok_or_else(|| "Select the part first, then create its views.".to_owned())?;
                let sides = match form.side {
                    SideChoice::Four => PartSide::FOUR.to_vec(),
                    SideChoice::One(side) => vec![side],
                };
                let four = form.side == SideChoice::Four;
                sides
                    .into_iter()
                    .map(|side| {
                        let (plane, span) = views::part_plane(&loaded.snapshot, &id, side)
                            .map_err(|e| e.to_string())?;
                        let name = if four {
                            format!("{} {}", form.name.trim(), side.as_str())
                        } else {
                            form.name.trim().to_owned()
                        };
                        // Every side, the 3D one too, starts orthographic.
                        Ok((name, plane, Some(span), Some(Projection::Orthographic)))
                    })
                    .collect::<Result<Vec<_>, String>>()?
            }
            Construction::Level | Construction::Assembly => {
                self.run_store_view(&form)?;
                return Ok(None);
            }
        };
        let batch = planned.len() > 1
            || matches!(form.construction, Construction::GridAxes)
                && form.axis_batch == AxisBatch::Families
            || form.side == SideChoice::Four && form.construction == Construction::Part;
        if planned.is_empty() {
            return Err("The batch selects no view.".into());
        }
        let mut names: Vec<String> = Vec::new();
        let mut made = Vec::new();
        for (name, plane, span, projection) in planned {
            let name = if batch {
                self.views.book.unique_name(&name, &names)
            } else {
                name
            };
            names.push(name.clone());
            let mut view = ModelView::new(fresh_id()?, &name, plane, 1.0);
            let mut each = form.clone();
            each.name = name;
            // Only the first view of a batch can take the default mark.
            each.default = form.default && made.is_empty();
            each.apply(&mut view)?;
            view.temporary = unnamed;
            if let Some(span) = span {
                view.span_mm = span;
            }
            if let Some(projection) = projection {
                view.projection = projection;
            }
            made.push(view);
        }
        let first = made.first().map(|v| v.id);
        let count = made.len();
        self.views
            .book
            .insert_all(made)
            .map_err(|e| e.to_string())?;
        if let Some(first) = first {
            self.views.selected.clear();
            self.views.selected.insert(first);
            self.activate_view(Some(first));
            // The panel moves on to the created view's properties.
            self.edit_view(first);
        }
        Ok(Some(if count == 1 {
            "The view was created; the model is unchanged.".into()
        } else {
            format!("{count} views were created; the model is unchanged.")
        }))
    }

    /// Level plans and assembly views read the store: the CLI builds them into the same file,
    /// and the book is read back when it answers.
    fn run_store_view(&mut self, form: &Form) -> Result<(), String> {
        let mut args: Vec<String> = vec![
            "save-view".into(),
            "--file".into(),
            crate::views::session_path(&self.source.store)
                .to_string_lossy()
                .into_owned(),
            "--store".into(),
            self.source.store.to_string_lossy().into_owned(),
            "--name".into(),
            form.name.trim().to_owned(),
        ];
        match form.construction {
            Construction::Level => match form.levels {
                None => args.extend(["--level".into(), form.level.trim().to_owned()]),
                Some(pick) => args.extend(["--levels".into(), pick.as_str().into()]),
            },
            _ => {
                let id = self
                    .editor
                    .selected()
                    .map(|s| s.id.clone())
                    .ok_or_else(|| "Select a part of the assembly first.".to_owned())?;
                let side = match form.side {
                    SideChoice::Four => "four",
                    SideChoice::One(side) => side.as_str(),
                };
                args.extend(["--assembly".into(), id, "--side".into(), side.into()]);
            }
        }
        // The display properties travel as options; the engine admits them again.
        let mut probe = ModelView::new(fresh_id()?, "probe", WorkPlane::default(), 1.0);
        form.apply(&mut probe)?;
        let join = |v: &[f64]| {
            v.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        args.extend([
            "--projection".into(),
            probe.projection.as_str().into(),
            "--span".into(),
            probe.span_mm.to_string(),
            "--depth".into(),
            probe.depth.map_or("none".into(), |d| join(&d)),
            "--rotation".into(),
            join(&probe.rotation_rad.map(f64::to_degrees)),
            "--render-mode".into(),
            probe.render_mode.as_token().into(),
            "--grids".into(),
            probe.grids_visible.to_string(),
            "--hidden-kinds".into(),
            if probe.hidden_kinds == beamcraft_engine::KindFilter::SHOW_ALL {
                "none".into()
            } else {
                probe
                    .hidden_kinds
                    .hidden_kinds()
                    .iter()
                    .map(|k| k.as_token())
                    .collect::<Vec<_>>()
                    .join(",")
            },
            "--work-area".into(),
            probe
                .work_area
                .map_or("none".into(), |a| join(&[a.min, a.max].concat())),
            "--color".into(),
            probe.color.map_or("default".into(), |c| join(&c)),
            "--opacity".into(),
            probe.opacity.to_string(),
            "--favorite".into(),
            probe.favorite.to_string(),
            "--default".into(),
            probe.is_default.to_string(),
        ]);
        if !probe.folder.is_empty() {
            args.extend(["--folder".into(), probe.folder.clone()]);
        }
        let cli = self.source.cli.clone();
        let stats = std::sync::Arc::clone(&self.stats);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("beamcraft-views".into())
            .spawn(move || {
                let reply = crate::command::run(&cli, &args, crate::command::DEADLINE, &stats)
                    .map_err(|e| e.message)
                    .and_then(|out| {
                        let json = crate::tools::decode(&out.stdout).map_err(|e| e.message)?;
                        if out.code == Some(0) {
                            Ok(())
                        } else {
                            Err(json
                                .get("message")
                                .and_then(beamcraft_json::Json::string)
                                .unwrap_or("the views were refused")
                                .to_owned())
                        }
                    });
                let _ = tx.send(reply);
            })
            .map_err(|_| "the view command could not be started".to_owned())?;
        self.views.job = Some(rx);
        Ok(())
    }

    /// Applies an answered store-backed construction: the book is read back from the file,
    /// keeping this session's temporary views.
    pub(crate) fn poll_view_job(&mut self) {
        let Some(rx) = &self.views.job else { return };
        let answer = match rx.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("the view command ended without a result".into())
            }
        };
        self.views.job = None;
        match answer.and_then(|()| crate::views::load(&self.source.store)) {
            Ok(mut book) => {
                let temporary: Vec<ModelView> = self
                    .views
                    .book
                    .views()
                    .iter()
                    .filter(|v| v.temporary)
                    .cloned()
                    .collect();
                for view in temporary {
                    let _ = book.insert(view);
                }
                let before: Vec<Uuid7> = self.views.book.views().iter().map(|v| v.id).collect();
                let added: Vec<Uuid7> = book
                    .views()
                    .iter()
                    .map(|v| v.id)
                    .filter(|id| !before.contains(id))
                    .collect();
                self.views.book = book;
                self.view_note(Ok(format!(
                    "{} view(s) were created; the model is unchanged.",
                    added.len()
                )));
                if let Some(first) = added.first().copied() {
                    self.activate_view(Some(first));
                    self.edit_view(first);
                }
            }
            Err(e) => self.view_note(Err(e)),
        }
    }

    /// Esc, Backspace, Ctrl+P and Ctrl+Tab for views.
    pub(crate) fn view_shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let (toggle, cycle, escape, back) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::P),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Tab),
                self.views.picking.is_some()
                    && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                !typing
                    && self.views.picking.is_some()
                    && i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace),
            )
        });
        if escape {
            self.views.picking = None;
            self.view_note(Ok("The view pick was cancelled.".into()));
        }
        if back && let Some(picking) = &mut self.views.picking {
            picking.undo();
        }
        if toggle {
            self.toggle_view_plane();
        }
        if cycle && let Some(next) = self.views.next_open() {
            self.activate_view(Some(next));
        }
    }

    /// Draws the picked points, the line to the cursor and the prompt while a view pick runs.
    pub(crate) fn paint_view_picking(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        view: &crate::camera::View,
        hover: Option<crate::edit::Ray>,
    ) {
        let Some(picking) = &self.views.picking else {
            return;
        };
        let origin = self.view_origin();
        let palette = crate::theme::Palette::current(ui);
        let painter = ui.painter_at(rect);
        let [ox, oy, oz] = origin;
        let screen = |[px, py, pz]: [f64; 3]| -> Option<egui::Pos2> {
            let local = [(px - ox) as f32, (py - oy) as f32, (pz - oz) as f32];
            let [x, y, z] = view.project(local);
            (z.is_finite() && (0.0..=1.0).contains(&z)).then(|| {
                egui::pos2(
                    rect.center().x + x * rect.width() / 2.0,
                    rect.center().y - y * rect.height() / 2.0,
                )
            })
        };
        let cursor = hover.and_then(|ray| {
            if picking.face.is_none()
                && matches!(
                    picking.construction,
                    Construction::Surface | Construction::SurfaceEdge
                )
            {
                None
            } else {
                self.view_point(ray)
            }
        });
        let mut last = None;
        for (index, point) in picking.points.iter().enumerate() {
            if let Some(at) = screen(*point) {
                if let Some(prev) = last {
                    painter.line_segment([prev, at], egui::Stroke::new(1.5, palette.select));
                }
                painter.circle_filled(at, 5.0, palette.select);
                painter.text(
                    at + egui::vec2(7.0, -7.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{}", index + 1),
                    egui::FontId::proportional(12.0),
                    palette.text,
                );
                last = Some(at);
            }
        }
        if let (Some(prev), Some(cursor)) = (last, cursor.and_then(screen)) {
            painter.line_segment([prev, cursor], egui::Stroke::new(1.0, palette.select));
        }
        if let Some(at) = cursor.and_then(screen) {
            painter.circle_stroke(at, 6.0, egui::Stroke::new(1.5, palette.select));
        }
        let prompt = format!(
            "{} · {}   Esc: cancel · Backspace: undo point",
            picking.construction.label(),
            picking.construction.prompt(picking.done())
        );
        painter.text(
            rect.left_top() + egui::vec2(12.0, 12.0),
            egui::Align2::LEFT_TOP,
            prompt,
            egui::FontId::proportional(13.0),
            palette.text,
        );
    }

    /// The browser: icon tabs, a search field and icon toggles on top, and below them the
    /// views grouped by folder, with quick access, favourites, multi-select, a context menu,
    /// cut/copy/paste and drag to a folder.
    pub(crate) fn view_browser(&mut self, ui: &mut egui::Ui) {
        use crate::views::BrowserTab;
        let palette = theme::Palette::current(ui);
        // Every tab keeps the panel as wide as the Views tab has made it.
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (tab, icon, label) in [
                (BrowserTab::Views, theme::Icon::ViewFrame, "Views"),
                (BrowserTab::Drawings, theme::Icon::Sheet, "Drawings"),
                (BrowserTab::Schedules, theme::Icon::Table, "Schedules"),
            ] {
                if browser_tab(ui, icon, label, self.views.tab == tab).clicked() {
                    self.views.tab = tab;
                }
            }
        });
        ui.add_space(4.0);
        if self.views.tab != BrowserTab::Views {
            ui.label(
                egui::RichText::new("Not listed in this browser yet.")
                    .size(11.0)
                    .color(palette.muted),
            );
            return;
        }
        // One row: the search field, then the new-folder button and the filter menu. A filter
        // that is on lights the funnel, so a short list never looks like a missing view.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            crate::kit::search_field(ui, &mut self.views.search, "Search views", 56.0);
            if theme::icon_toggle(
                ui,
                theme::Icon::NewFolder,
                "New folder (inside the selected view's folder)",
                self.views.new_folder,
            )
            .clicked()
            {
                let parent = self
                    .views
                    .selected
                    .iter()
                    .next()
                    .and_then(|id| self.views.book.get(id))
                    .map_or_else(String::new, |view| view.folder.clone());
                self.views.new_folder = !self.views.new_folder;
                self.views.folder_parent = parent;
                self.views.destination.clear();
            }
            let filtering = self.views.quick_access || self.views.favorites_only;
            let funnel = theme::icon_toggle(
                ui,
                theme::Icon::Filter,
                "Show: quick access, favourites, previews",
                filtering || self.views.previews_on,
            );
            egui::Popup::menu(&funnel)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(220.0);
                    ui.checkbox(
                        &mut self.views.quick_access,
                        "Quick access (favourites, temporary, open)",
                    );
                    ui.checkbox(&mut self.views.favorites_only, "Favourites only");
                    ui.separator();
                    ui.checkbox(&mut self.views.previews_on, "Show previews");
                });
        });
        if self.views.selected.len() > 1 {
            ui.horizontal(|ui| {
                let label = format!("Open {} selected views", self.views.selected.len());
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(label).size(11.0).color(palette.link),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    for id in self.views.selected.clone() {
                        self.open_view_at(id, None);
                    }
                }
            });
        }
        if self.views.new_folder {
            let parent = self.views.folder_parent.clone();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                theme::icon(ui, theme::Icon::Folder, 16.0, palette.muted);
                let place = if parent.is_empty() {
                    "the top level".to_owned()
                } else {
                    parent.clone()
                };
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.views.destination)
                        .hint_text(format!("New folder in {place}"))
                        .char_limit(crate::views::FIELD)
                        .desired_width((ui.available_width() - 60.0 - FIT).max(40.0)),
                );
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if theme::icon_toggle(ui, theme::Icon::Check, "Create the folder", false).clicked()
                    || enter
                {
                    let name = self.views.destination.trim().to_owned();
                    let folder = crate::views::new_folder_path(&parent, &name);
                    match self.views.book.create_folder(&folder) {
                        Ok(()) => {
                            self.views.new_folder = false;
                            self.persist_views();
                        }
                        Err(e) => self.view_note(Err(e.to_string())),
                    }
                }
                if theme::icon_toggle(ui, theme::Icon::Close, "Cancel", false).clicked() {
                    self.views.new_folder = false;
                }
            });
        }
        ui.add_space(2.0);
        ui.separator();
        let tree = self.views.browser_tree();
        let home = self.views.browser_home();
        let order: Vec<Uuid7> = home.into_iter().chain(tree.all_views()).collect();
        let mut actions: Vec<BrowserAction> = Vec::new();
        let mut budget = true;
        let confirm = self.views.confirm_delete;
        let footer = if confirm.is_some() { 72.0 } else { 0.0 };
        let presets: Vec<String> = self.preset_choices().into_iter().map(|p| p.name).collect();
        egui::ScrollArea::vertical()
            .id_salt("view-browser")
            .max_height((ui.available_height() - footer).max(60.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.views.book.views().is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "No views yet. Create one with the View commands in the ribbon.",
                        )
                        .size(11.0)
                        .color(palette.muted),
                    );
                }
                let ctx = BrowserContext {
                    order: &order,
                    presets: &presets,
                };
                let mut listed = false;
                if let Some(view) = home.and_then(|id| self.views.book.get(&id).cloned()) {
                    listed = true;
                    ui.horizontal(|ui| {
                        ui.add_space(crate::kit::CHEVRON - ui.spacing().item_spacing.x);
                        ui.vertical(|ui| {
                            self.browser_view_row(ui, &view, &ctx, &mut actions, &mut budget);
                        });
                    });
                }
                // The top level: the folders, then the views no folder holds, lined up with the
                // folders' names so a view's icon sits under a folder's icon.
                for folder in &tree.folders {
                    listed = true;
                    self.browser_folder(ui, folder, &ctx, &mut actions, &mut budget);
                }
                for view_id in &tree.views {
                    if let Some(view) = self.views.book.get(view_id).cloned() {
                        listed = true;
                        ui.horizontal(|ui| {
                            ui.add_space(crate::kit::CHEVRON - ui.spacing().item_spacing.x);
                            ui.vertical(|ui| {
                                self.browser_view_row(ui, &view, &ctx, &mut actions, &mut budget);
                            });
                        });
                    }
                }
                // The free space under the list takes a dropped view out of its folder.
                let rest = ui.allocate_response(
                    egui::vec2(ui.available_width(), ui.available_height().max(24.0)),
                    egui::Sense::hover(),
                );
                if let Some(view) = rest.dnd_release_payload::<Uuid7>() {
                    actions.push(BrowserAction::Move(*view, String::new()));
                }
                rest.context_menu(|ui| {
                    if ui.button("New folder…").clicked() {
                        actions.push(BrowserAction::NewFolder(String::new()));
                        ui.close();
                    }
                });
                if !listed && !self.views.book.views().is_empty() {
                    ui.label(
                        egui::RichText::new("No views match.")
                            .size(11.0)
                            .color(palette.muted),
                    );
                }
            });
        if let Some(id) = confirm {
            let name = self
                .views
                .book
                .get(&id)
                .map_or_else(String::new, |v| v.name.clone());
            ui.separator();
            ui.label(format!("Delete view “{name}”? The model is unchanged."));
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    actions.push(BrowserAction::ConfirmDelete(id));
                }
                if ui.button("Keep").clicked() {
                    self.views.confirm_delete = None;
                }
            });
        }
        for action in actions {
            self.browser_action(action);
        }
        if let Some(Err(note)) = &self.views.note {
            let note = note.clone();
            ui.colored_label(palette.error, note);
        }
    }
    /// One folder of the browser tree, collapsible, with its subfolders and its views. A view dropped on the node's header moves into it;
    /// its menu makes a folder inside it, opens or closes everything in it, and deletes it when
    /// it is an empty folder.
    #[allow(clippy::too_many_arguments)]
    fn browser_folder(
        &mut self,
        ui: &mut egui::Ui,
        node: &crate::views::FolderNode,
        ctx: &BrowserContext<'_>,
        actions: &mut Vec<BrowserAction>,
        budget: &mut bool,
    ) {
        let palette = theme::Palette::current(ui);
        let salt = format!("view-folder-{}", node.stored);
        let id = ui.make_persistent_id(salt);
        let all = node.all_views();
        let mut toggle = false;
        let dragging = egui::DragAndDrop::has_payload_of_type::<Uuid7>(ui.ctx());
        let (node_ui, header_row, ()) = crate::kit::TreeNode::header(ui, id, true, |ui| {
            theme::icon(ui, theme::Icon::Folder, 16.0, palette.muted);
            let renaming = self
                .views
                .folder_rename
                .as_ref()
                .is_some_and(|(stored, _)| *stored == node.stored);
            if renaming {
                // Renamed in place, as a file browser does: Enter keeps the name, Esc drops it.
                if let Some((_, typed)) = self.views.folder_rename.as_mut() {
                    let field = ui.add(
                        egui::TextEdit::singleline(typed)
                            .desired_width((ui.available_width() - crate::kit::FIT).max(60.0))
                            .char_limit(crate::views::FIELD),
                    );
                    if !field.has_focus() && !field.lost_focus() {
                        field.request_focus();
                    }
                    let (enter, escape) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if escape {
                        actions.push(BrowserAction::CancelRename);
                    } else if field.lost_focus() || enter {
                        actions.push(BrowserAction::RenameFolder(
                            node.stored.clone(),
                            typed.trim().to_owned(),
                        ));
                    }
                }
                return;
            }
            let name = ui.add(
                egui::Label::new(
                    egui::RichText::new(node.name.as_str())
                        .size(12.0)
                        .strong()
                        .color(palette.text),
                )
                .truncate()
                .sense(egui::Sense::click()),
            );
            count_pill(ui, all.len());
            toggle = name.clicked();
            // Rename and delete show on the row under the pointer, as small icons at its end.
            if ui.ui_contains_pointer() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if small_icon(
                        ui,
                        theme::Icon::Delete,
                        "Delete the folder (its views move up)",
                    )
                    .clicked()
                    {
                        actions.push(BrowserAction::DeleteFolder(node.stored.clone()));
                    }
                    if small_icon(ui, theme::Icon::Rename, "Rename the folder").clicked() {
                        actions.push(BrowserAction::StartRename(node.stored.clone()));
                    }
                });
            }
            name.context_menu(|ui| {
                if ui.button("New folder…").clicked() {
                    actions.push(BrowserAction::NewFolder(node.stored.clone()));
                    ui.close();
                }
                if !all.is_empty() {
                    if ui.button("Open all").clicked() {
                        actions.push(BrowserAction::OpenAll(all.clone()));
                        ui.close();
                    }
                    if ui.button("Close all").clicked() {
                        actions.push(BrowserAction::CloseAll(all.clone()));
                        ui.close();
                    }
                }
                if ui.button("Rename folder").clicked() {
                    actions.push(BrowserAction::StartRename(node.stored.clone()));
                    ui.close();
                }
                if ui.button("Delete folder").clicked() {
                    actions.push(BrowserAction::DeleteFolder(node.stored.clone()));
                    ui.close();
                }
            });
        });
        node_ui.body(ui, |ui| {
            for folder in &node.folders {
                self.browser_folder(ui, folder, ctx, actions, budget);
            }
            if all.is_empty() {
                ui.label(
                    egui::RichText::new("Empty folder")
                        .size(11.0)
                        .color(palette.muted),
                );
            }
            for view_id in &node.views {
                if let Some(view) = self.views.book.get(view_id).cloned() {
                    self.browser_view_row(ui, &view, ctx, actions, budget);
                }
            }
        });
        let header = header_row;
        if dragging && header.contains_pointer() {
            ui.painter().rect_stroke(
                header.rect,
                3,
                egui::Stroke::new(1.5, palette.link),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(view) = header.dnd_release_payload::<Uuid7>() {
            actions.push(BrowserAction::Move(*view, node.stored.clone()));
        }
        if toggle {
            crate::kit::toggle_node(ui, id, true);
        }
    }

    /// One view row of the browser: a click selects it and shows its properties, a double
    /// click opens it as a tab, a drag moves it to a folder or opens it in the work area where
    /// it is dropped, and its menu holds the view's actions.
    fn browser_view_row(
        &mut self,
        ui: &mut egui::Ui,
        view: &ModelView,
        ctx: &BrowserContext<'_>,
        actions: &mut Vec<BrowserAction>,
        budget: &mut bool,
    ) {
        if self.views.previews_on {
            match self.view_preview(view, budget) {
                Some(texture) => {
                    ui.image((texture, egui::vec2(160.0, 100.0)));
                }
                None => {
                    ui.small("Preparing the preview…");
                }
            }
        }
        let response = browser_row(
            ui,
            BrowserRow {
                // The default view is the model's home view: the house is its icon.
                icon: Some(if view.is_default {
                    theme::Icon::Home
                } else {
                    crate::views::view_kind(view).icon()
                }),
                name: &view.name,
                selected: self.views.selected.contains(&view.id)
                    || self.views.active() == Some(view.id),
                open: self.views.open.contains(&view.id),
                favorite: view.favorite,
                temporary: view.temporary,
                default: view.is_default,
            },
        );
        response.dnd_set_drag_payload(view.id);
        if response.double_clicked() {
            actions.push(BrowserAction::Activate(view.id));
        } else if response.clicked() {
            let m = ui.input(|i| i.modifiers);
            actions.push(BrowserAction::Click(
                view.id,
                m.command,
                m.shift,
                ctx.order.to_vec(),
            ));
        }
        response.context_menu(|ui| {
            let mut item = |ui: &mut egui::Ui, label: &str, action: BrowserAction| {
                if ui.button(label).clicked() {
                    actions.push(action);
                    ui.close();
                }
            };
            item(ui, "Open", BrowserAction::Activate(view.id));
            item(ui, "Open beside", BrowserAction::Window(view.id));
            item(ui, "Properties", BrowserAction::Properties(view.id));
            ui.separator();
            item(ui, "Duplicate", BrowserAction::Duplicate(view.id));
            if !ctx.presets.is_empty() {
                ui.menu_button("Apply view preset", |ui| {
                    for name in ctx.presets {
                        item(ui, name, BrowserAction::ApplyPreset(view.id, name.clone()));
                    }
                });
            }
            item(
                ui,
                "Save as view preset",
                BrowserAction::SavePreset(view.id),
            );
            item(
                ui,
                if view.favorite {
                    "Remove from favourites"
                } else {
                    "Add to favourites"
                },
                BrowserAction::Favorite(view.id),
            );
            if !view.is_default && !view.temporary {
                item(ui, "Make default", BrowserAction::SetDefault(view.id));
            }
            ui.separator();
            item(ui, "Close", BrowserAction::Close(view.id));
            if !view.is_default {
                item(ui, "Delete…", BrowserAction::Delete(view.id));
            }
        });
    }

    pub(crate) fn browser_action(&mut self, action: BrowserAction) {
        match action {
            // As in Revit's browser: a click selects the view and shows its properties, a double
            // click opens it.
            BrowserAction::Click(id, ctrl, shift, order) => {
                if self.views.click(id, ctrl, shift, &order) {
                    self.edit_view(id);
                }
            }
            BrowserAction::Activate(id) => self.open_view_at(id, None),
            BrowserAction::NewFolder(parent) => {
                self.views.new_folder = true;
                self.views.folder_parent = parent;
                self.views.destination.clear();
            }
            BrowserAction::ApplyPreset(id, name) => {
                match self.preset_choices().into_iter().find(|p| p.name == name) {
                    Some(preset) => self.apply_preset_to(id, &preset),
                    None => self.view_note(Err(format!("No view preset is named “{name}”."))),
                }
            }
            BrowserAction::SavePreset(id) => {
                if let Some(view) = self.views.book.get(&id) {
                    let name = view.name.clone();
                    self.save_preset_from(id, &name);
                }
            }
            BrowserAction::Window(id) => self.open_beside(Some(id)),
            BrowserAction::Properties(id) => self.edit_view(id),
            BrowserAction::Favorite(id) => {
                if let Some(mut view) = self.views.book.get(&id).cloned() {
                    view.favorite = !view.favorite;
                    self.keep_view(view);
                }
            }
            BrowserAction::Duplicate(id) => {
                if let Some(view) = self.views.book.get(&id).cloned() {
                    let name = self
                        .views
                        .book
                        .unique_name(&format!("{} copy", view.name), &[]);
                    self.copy_view(&view, name, view.folder.clone());
                }
            }
            // Closing a view closes its panes; the work area may end up empty.
            BrowserAction::Close(id) => self.close_view(id),
            BrowserAction::Delete(id) => {
                if self.views.book.get(&id).is_some_and(|v| v.is_default) {
                    self.view_note(Err(
                        "The default view cannot be deleted: make another view the default first."
                            .into(),
                    ));
                } else {
                    self.views.confirm_delete = Some(id);
                }
            }
            BrowserAction::ConfirmDelete(id) => {
                self.views.confirm_delete = None;
                if self.views.book.get(&id).is_some_and(|v| v.is_default) {
                    return;
                }
                self.close_view(id);
                self.views.forget(&id);
                if let Err(e) = self.views.book.remove(&id) {
                    self.view_note(Err(e.to_string()));
                }
                self.persist_views();
            }
            BrowserAction::SetDefault(id) => match self.views.book.set_default(&id) {
                Ok(()) => self.persist_views(),
                Err(e) => self.view_note(Err(e.to_string())),
            },
            // A folder is removed with its contents kept: they move up one level.
            BrowserAction::DeleteFolder(folder) => match self.views.book.dissolve_folder(&folder) {
                Ok(moved) => {
                    self.persist_views();
                    self.view_note(Ok(match moved {
                        0 => "Folder deleted.".to_owned(),
                        n => format!("Folder deleted; its {n} views moved up."),
                    }));
                }
                Err(e) => self.view_note(Err(e.to_string())),
            },
            BrowserAction::StartRename(folder) => {
                let name = folder.rsplit('/').next().unwrap_or(&folder).to_owned();
                self.views.folder_rename = Some((folder, name));
            }
            BrowserAction::CancelRename => self.views.folder_rename = None,
            BrowserAction::RenameFolder(folder, name) => {
                self.views.folder_rename = None;
                let parent = folder.rsplit_once('/').map_or("", |(p, _)| p);
                let target = crate::views::new_folder_path(parent, &name);
                if target != folder && !name.is_empty() {
                    match self.views.book.rename_folder(&folder, &target) {
                        Ok(_) => self.persist_views(),
                        Err(e) => self.view_note(Err(e.to_string())),
                    }
                }
            }
            BrowserAction::OpenAll(ids) => {
                for id in ids {
                    self.open_pane(Some(id));
                }
            }
            BrowserAction::CloseAll(ids) => {
                for id in ids {
                    self.close_view(id);
                }
            }
            BrowserAction::Move(id, folder) => {
                if let Some(mut view) = self.views.book.get(&id).cloned() {
                    view.folder = folder;
                    self.keep_view(view);
                }
            }
        }
    }

    /// The view presets a view can be filled from, as the part presets fill a new part: the
    /// user's own first, then the environment's that the user does not shadow by name.
    pub(crate) fn preset_choices(&self) -> Vec<beamcraft_engine::presets::ViewPreset> {
        let mut out: Vec<_> = self.views.presets.presets().to_vec();
        for preset in &self.tools.definitions.view_presets {
            if !out
                .iter()
                .any(|p| p.name.to_lowercase() == preset.name.to_lowercase())
            {
                out.push(preset.clone());
            }
        }
        out
    }

    /// The filters a view can use: the model's own first, then the environment's that the
    /// model does not shadow by name.
    pub(crate) fn filter_choices(&self) -> Vec<beamcraft_engine::views::ViewFilter> {
        let mut out: Vec<_> = self.views.book.filters().to_vec();
        for filter in &self.tools.definitions.view_filters {
            if self.views.book.filter(&filter.name).is_none() {
                out.push(filter.clone());
            }
        }
        out
    }

    /// Makes sure the model has the filter `name`: an environment filter of that name is
    /// copied into the model's views the first time a view uses it.
    pub(crate) fn adopt_filter(&mut self, name: &str) -> Result<(), String> {
        if self.views.book.filter(name).is_some() {
            return Ok(());
        }
        let filter = self
            .tools
            .definitions
            .view_filters
            .iter()
            .find(|f| f.name.to_lowercase() == name.to_lowercase())
            .cloned()
            .ok_or_else(|| format!("No filter is named “{name}”."))?;
        self.views
            .book
            .save_filter(filter, None)
            .map_err(|e| e.to_string())
    }

    /// Fills view `id`'s display from `preset` (its filters come along) and saves it.
    pub(crate) fn apply_preset_to(
        &mut self,
        id: Uuid7,
        preset: &beamcraft_engine::presets::ViewPreset,
    ) {
        for o in &preset.overrides {
            if let Err(e) = self.adopt_filter(&o.filter) {
                return self.view_note(Err(e));
            }
        }
        let Some(mut view) = self.views.book.get(&id).cloned() else {
            return;
        };
        if let Err(e) = preset.apply_to(&mut view) {
            return self.view_note(Err(e.to_string()));
        }
        let name = preset.name.clone();
        self.keep_view(view);
        self.view_note(Ok(format!("The view now shows as “{name}”.")));
        if self.views.subject == Subject::SavedView(id) {
            self.edit_view(id);
        }
        self.refresh_filter_overlays();
    }

    /// Writes the user's view presets into the configuration directory.
    fn persist_presets(&mut self) -> Result<(), String> {
        crate::views::persist_presets(self.source.config_dir.as_deref(), &self.views.presets)
    }

    /// Saves view `id`'s display as the user's view preset `name` (replacing one of that
    /// name).
    pub(crate) fn save_preset_from(&mut self, id: Uuid7, name: &str) {
        let Some(view) = self.views.book.get(&id) else {
            return;
        };
        let preset = beamcraft_engine::presets::ViewPreset::from_view(name, view);
        match self
            .views
            .presets
            .save(preset)
            .map_err(|e| e.to_string())
            .and_then(|()| self.persist_presets())
        {
            Ok(()) => {
                self.view_note(Ok(format!("Saved the view preset “{}”.", name.trim())));
            }
            Err(e) => self.view_note(Err(e.to_string())),
        }
    }

    /// Saves the display the form shows now as the user's view preset `name`; the filters it
    /// names come into the model with it.
    pub(crate) fn save_preset_from_form(&mut self, name: &str) {
        for o in self.views.form.overrides.clone() {
            if let Err(e) = self.adopt_filter(&o.filter) {
                return self.view_note(Err(e));
            }
        }
        let base = self
            .views
            .form
            .id
            .and_then(|id| self.views.book.get(&id).cloned());
        let mut view = match base {
            Some(view) => view,
            None => match fresh_id() {
                Ok(id) => ModelView::new(id, "preset", self.views.form.plane, 10_000.0),
                Err(e) => return self.view_note(Err(e)),
            },
        };
        let mut form = self.views.form.clone();
        if form.name.trim().is_empty() {
            "preset".clone_into(&mut form.name);
        }
        if let Err(e) = form.apply(&mut view) {
            return self.view_note(Err(e));
        }
        let preset = beamcraft_engine::presets::ViewPreset::from_view(name, &view);
        match self
            .views
            .presets
            .save(preset)
            .map_err(|e| e.to_string())
            .and_then(|()| self.persist_presets())
        {
            Ok(()) => {
                self.views.preset_name = None;
                self.view_note(Ok(format!("Saved the view preset “{}”.", name.trim())));
            }
            Err(e) => self.view_note(Err(e.to_string())),
        }
    }

    /// Opens the filter editor in the panel on the filter named `target` (the model's or the
    /// environment's), or on a new filter.
    pub(crate) fn open_filter_editor(&mut self, target: Option<String>) {
        if self.bench != Bench::View {
            self.set_bench(Bench::View);
            if self.bench != Bench::View {
                return;
            }
        }
        let existing = target.as_deref().and_then(|name| {
            self.filter_choices()
                .into_iter()
                .find(|f| f.name.to_lowercase() == name.to_lowercase())
        });
        let previous = existing
            .as_ref()
            .filter(|f| self.views.book.filter(&f.name).is_some())
            .map(|f| f.name.clone());
        let filter = existing.unwrap_or_else(|| {
            let name = (1..=beamcraft_engine::view_filter::MAX_FILTERS + 1)
                .map(|n| format!("Filter {n}"))
                .find(|n| self.views.book.filter(n).is_none())
                .unwrap_or_else(|| "New filter".into());
            beamcraft_engine::views::ViewFilter::new(
                &name,
                vec![beamcraft_engine::views::FilterCondition::new(
                    beamcraft_engine::views::FilterField::Type,
                    beamcraft_engine::filter::Operator::Eq,
                    "beam",
                )],
            )
        });
        self.views.filter_edit = Some((filter, previous));
        self.views.note = None;
    }

    /// The filter editor, in the panel in place of the view properties: the filter's name,
    /// whether all or any of its conditions must hold, and one row per condition (field,
    /// operator, value), as Tekla's selection filter rows read; Save keeps it in the model's
    /// views, and a new filter opened from a view's table joins that view.
    pub(crate) fn filter_editor(&mut self, ui: &mut egui::Ui) {
        use beamcraft_engine::views::{FilterCondition, FilterField, FilterMatch};
        let Some((mut filter, previous)) = self.views.filter_edit.clone() else {
            return;
        };
        let palette = theme::Palette::current(ui);
        let choices: Vec<String> = self.filter_choices().into_iter().map(|f| f.name).collect();
        let mut switch_to: Option<Option<String>> = None;
        let mut save = false;
        let mut delete = false;
        let mut back = false;
        egui::ScrollArea::vertical()
            .id_salt("filter-editor")
            .max_height((ui.available_height() - 90.0).max(0.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                type_box(
                    ui,
                    &TypeBox {
                        icon: theme::Icon::Select,
                        kind: if previous.is_some() {
                            "Filter".into()
                        } else {
                            "New filter".into()
                        },
                    },
                );
                ui.add_space(4.0);
                row(ui, "Edit filter", |ui| {
                    egui::ComboBox::from_id_salt("filter-pick")
                        .selected_text(filter.name.clone())
                        .width((ui.available_width() - FIT).max(40.0))
                        .show_ui(ui, |ui| {
                            for name in &choices {
                                if ui.button(name).clicked() {
                                    switch_to = Some(Some(name.clone()));
                                }
                            }
                            ui.separator();
                            if ui.button("New filter").clicked() {
                                switch_to = Some(None);
                            }
                        });
                });
                text(ui, "Name", &mut filter.name);
                row(ui, "Elements", |ui| {
                    // Wrapped: two toggles exceed the value cell on a narrow panel, and an
                    // unwrapped row would break the second label into letters.
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(&mut filter.matching, FilterMatch::All, "match all");
                        ui.selectable_value(&mut filter.matching, FilterMatch::Any, "match any");
                    });
                });
                section(ui, "Conditions", true, |ui| {
                    let mut remove = None;
                    for (at, condition) in filter.conditions.iter_mut().enumerate() {
                        ui.push_id(("filter-row", at), |ui| {
                            ui.horizontal(|ui| {
                                let current = match &condition.field {
                                    FilterField::Type => "type",
                                    FilterField::Name => "name",
                                    FilterField::Class => "class",
                                    FilterField::Attribute(_) => "attribute",
                                };
                                let label = FIELDS
                                    .iter()
                                    .find(|(t, _)| *t == current)
                                    .map_or("Field", |(_, l)| l);
                                egui::ComboBox::from_id_salt("field")
                                    .selected_text(label)
                                    .width(76.0)
                                    .show_ui(ui, |ui| {
                                        for (token, label) in FIELDS {
                                            if ui
                                                .selectable_label(token == current, label)
                                                .clicked()
                                                && token != current
                                            {
                                                condition.field = match token {
                                                    "type" => FilterField::Type,
                                                    "name" => FilterField::Name,
                                                    "class" => FilterField::Class,
                                                    _ => FilterField::Attribute(String::new()),
                                                };
                                                condition.op =
                                                    beamcraft_engine::filter::Operator::Eq;
                                                condition.value = match token {
                                                    "type" => "beam".into(),
                                                    "class" => "2".into(),
                                                    _ => String::new(),
                                                };
                                            }
                                        }
                                    });
                                let ops = operators(&condition.field);
                                let op_label = ops
                                    .iter()
                                    .find(|(op, _)| *op == condition.op)
                                    .map_or("?", |(_, l)| l);
                                egui::ComboBox::from_id_salt("op")
                                    .selected_text(op_label)
                                    .width(96.0)
                                    .show_ui(ui, |ui| {
                                        for (op, label) in ops {
                                            ui.selectable_value(&mut condition.op, op, label);
                                        }
                                    });
                                if theme::icon_toggle(
                                    ui,
                                    theme::Icon::Close,
                                    "Remove the condition",
                                    false,
                                )
                                .clicked()
                                {
                                    remove = Some(at);
                                }
                            });
                            ui.horizontal(|ui| {
                                if let FilterField::Attribute(name) = &mut condition.field {
                                    ui.add(
                                        egui::TextEdit::singleline(name)
                                            .hint_text("attribute")
                                            .desired_width(90.0)
                                            .char_limit(128),
                                    );
                                }
                                let typed = matches!(condition.field, FilterField::Type)
                                    && matches!(
                                        condition.op,
                                        beamcraft_engine::filter::Operator::Eq
                                            | beamcraft_engine::filter::Operator::Ne
                                    );
                                if typed {
                                    egui::ComboBox::from_id_salt("value")
                                        .selected_text(condition.value.clone())
                                        .width((ui.available_width() - FIT).max(60.0))
                                        .show_ui(ui, |ui| {
                                            for kind in beamcraft_engine::ElementKind::ALL {
                                                ui.selectable_value(
                                                    &mut condition.value,
                                                    kind.as_token().to_owned(),
                                                    kind.as_token(),
                                                );
                                            }
                                        });
                                } else {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut condition.value)
                                            .hint_text("value, e.g. 250 mm")
                                            .desired_width((ui.available_width() - FIT).max(60.0))
                                            .char_limit(128),
                                    );
                                }
                            });
                            ui.separator();
                        });
                    }
                    if let Some(at) = remove
                        && at < filter.conditions.len()
                    {
                        filter.conditions.remove(at);
                    }
                    if filter.conditions.len() < beamcraft_engine::view_filter::MAX_CONDITIONS
                        && ui.button("Add condition").clicked()
                    {
                        filter.conditions.push(FilterCondition::new(
                            FilterField::Name,
                            beamcraft_engine::filter::Operator::Contains,
                            "",
                        ));
                    }
                    ui.label(
                        egui::RichText::new(match filter.matching {
                            FilterMatch::All if filter.conditions.is_empty() => {
                                "No condition: the filter matches every element."
                            }
                            FilterMatch::All => "An element matches when every condition holds.",
                            FilterMatch::Any => "An element matches when any condition holds.",
                        })
                        .size(11.0)
                        .color(palette.muted),
                    );
                });
            });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            save = theme::button(ui, theme::Kind::Primary, "Save filter", true).clicked();
            delete = previous.is_some()
                && theme::button(ui, theme::Kind::Secondary, "Delete", true).clicked();
            back = theme::button(ui, theme::Kind::Secondary, "Back", true).clicked();
        });
        self.views.filter_edit = Some((filter.clone(), previous.clone()));
        if let Some(target) = switch_to {
            self.open_filter_editor(target);
            return;
        }
        if back {
            self.views.filter_edit = None;
            self.views.note = None;
            return;
        }
        if delete && let Some(name) = previous {
            match self.views.book.remove_filter(&name) {
                Ok(changed) => {
                    self.views
                        .form
                        .overrides
                        .retain(|o| o.filter.to_lowercase() != name.to_lowercase());
                    self.persist_views();
                    self.views.filter_edit = None;
                    self.refresh_filter_overlays();
                    self.view_note(Ok(format!(
                        "Deleted the filter “{name}”; {changed} view(s) no longer use it."
                    )));
                }
                Err(e) => self.view_note(Err(e.to_string())),
            }
            return;
        }
        if save {
            filter.name = filter.name.trim().to_owned();
            let new = previous.is_none();
            match self
                .views
                .book
                .save_filter(filter.clone(), previous.as_deref())
            {
                Ok(()) => {
                    // A rename follows into the form the panel goes back to.
                    if let Some(old) = &previous {
                        for o in &mut self.views.form.overrides {
                            if o.filter.to_lowercase() == old.to_lowercase() {
                                o.filter.clone_from(&filter.name);
                            }
                        }
                    }
                    let joins = new
                        && !matches!(self.views.subject, Subject::FreeView(_))
                        && !self
                            .views
                            .form
                            .overrides
                            .iter()
                            .any(|o| o.filter == filter.name)
                        && self.views.form.overrides.len()
                            < beamcraft_engine::view_filter::MAX_OVERRIDES;
                    if joins {
                        self.views
                            .form
                            .overrides
                            .push(beamcraft_engine::views::FilterOverride::new(&filter.name));
                    }
                    self.persist_views();
                    self.views.filter_edit = None;
                    self.refresh_filter_overlays();
                    self.view_note(Ok(if joins {
                        format!(
                            "Saved the filter “{}”; Apply adds it to the view.",
                            filter.name
                        )
                    } else {
                        format!("Saved the filter “{}”.", filter.name)
                    }));
                }
                Err(e) => self.view_note(Err(e.to_string())),
            }
        }
    }

    /// Builds the overlays again on the next frame: a filter or an override changed.
    pub(crate) fn refresh_filter_overlays(&mut self) {
        self.views.overlays.clear();
    }

    /// Keeps the filter overlays of the open views current: follows the head on screen, takes
    /// finished evaluations in, asks for the rules still missing (on a worker, one batch at a
    /// time) and rebuilds the overlay of a view whose overrides, filters, matches or colour set
    /// changed. Views without overrides need none.
    pub(crate) fn poll_filter_overlays(&mut self) {
        let Some(token) = self
            .loaded
            .as_ref()
            .map(|l| l.snapshot.token().as_str().to_owned())
        else {
            self.views.overlays.clear();
            return;
        };
        self.views.matches.follow(&token);
        self.views.matches.poll();
        let base = self.colour.as_ref().map(|(_, o)| std::sync::Arc::clone(o));
        let open: Vec<Uuid7> = self
            .views
            .panes
            .iter()
            .filter_map(|p| p.view)
            .filter(|id| {
                self.views
                    .book
                    .get(id)
                    .is_some_and(|v| !v.overrides.is_empty())
            })
            .collect();
        self.views.overlays.retain(|id, _| open.contains(id));
        let mut wanted = Vec::new();
        for id in open {
            let Some(view) = self.views.book.get(&id) else {
                continue;
            };
            let key = crate::view_overlay::overlay_key(
                view,
                &self.views.book,
                &self.views.matches,
                base.as_ref(),
            );
            let same = self
                .views
                .overlays
                .get(&id)
                .map(|c| (c.key == key, c.complete));
            if same == Some((true, true)) {
                continue;
            }
            let (overlay, missing, refused) = crate::view_overlay::overlay_of(
                view,
                &self.views.book,
                &self.views.matches,
                base.as_deref(),
            );
            let complete = missing.is_empty();
            wanted.extend(missing);
            // Nothing new arrived since it was built: keep it, so the panes keep their scene.
            if same.is_some_and(|(key_same, _)| key_same) {
                continue;
            }
            self.views.overlays.insert(
                id,
                crate::views::ViewOverlay {
                    key,
                    overlay: std::sync::Arc::new(overlay),
                    complete,
                    refused,
                },
            );
        }
        if !wanted.is_empty() {
            let (cli, store) = (self.source.cli.clone(), self.source.store.clone());
            self.views
                .matches
                .request(&cli, &store, &self.stats, wanted);
        }
    }

    fn keep_view(&mut self, view: ModelView) {
        match self.views.book.update(view) {
            Ok(()) => self.persist_views(),
            Err(e) => self.view_note(Err(e.to_string())),
        }
    }

    fn copy_view(&mut self, source: &ModelView, name: String, folder: String) {
        let Ok(id) = fresh_id() else {
            self.view_note(Err("no view identity could be made".into()));
            return;
        };
        let mut copy = source.clone();
        copy.id = id;
        copy.name = name;
        copy.folder = folder;
        copy.is_default = false;
        match self.views.book.insert(copy) {
            Ok(()) => self.persist_views(),
            Err(e) => self.view_note(Err(e.to_string())),
        }
    }

    /// Closes a view in the session: every pane showing it and its kept camera; the definition
    /// stays.
    fn close_view(&mut self, id: Uuid7) {
        let keys: Vec<u64> = self
            .views
            .panes
            .iter()
            .filter(|p| p.view == Some(id))
            .map(|p| p.key)
            .collect();
        for key in keys {
            self.close_pane(key);
        }
        self.views.cameras.remove(&id);
        self.views.open.retain(|v| *v != id);
    }

    /// The preview of `view`: drawn off screen with the view's own camera, cut, tint and hidden
    /// kinds, kept until the snapshot or the view changes. At most one preview is drawn per
    /// frame (`budget`), and at most [`crate::views::MAX_PREVIEWS`] are kept.
    fn view_preview(&mut self, view: &ModelView, budget: &mut bool) -> Option<egui::TextureId> {
        let loaded = self.loaded.as_ref()?;
        let key = crate::views::preview_key(loaded.snapshot.token().as_str(), view);
        if let Some((drawn, target)) = self.views.previews.get(&view.id)
            && *drawn == key
        {
            return target.texture();
        }
        if !std::mem::take(budget) {
            return self
                .views
                .previews
                .get(&view.id)
                .and_then(|(_, t)| t.texture());
        }
        let origin = loaded.summary.origin_mm;
        let extent = self.view_extent();
        let camera = Camera::from_view(view, origin, extent).ok()?;
        if !self.views.previews.contains_key(&view.id)
            && self.views.previews.len() >= crate::views::MAX_PREVIEWS
            && let Some(oldest) = self.views.previews.keys().next().copied()
            && let Some((_, mut gone)) = self.views.previews.remove(&oldest)
        {
            gone.release(&self.state);
        }
        let (_, target) = self
            .views
            .previews
            .entry(view.id)
            .or_insert_with(|| (0, RenderTarget::empty()));
        let [width, height] = crate::views::PREVIEW_SIZE;
        let aspect = target
            .resize(&self.state, width, height, Some(1), &self.stats)
            .ok()?;
        let hidden = view.hidden_kinds;
        let filtered = if hidden == self.views.shown {
            None
        } else {
            let overlay = self.colour.as_ref().map(|(_, o)| std::sync::Arc::clone(o));
            crate::scene::Scene::from_snapshot_showing(
                &loaded.snapshot,
                beamcraft_engine::Representation::Full,
                &loaded.uploaded.resident(),
                overlay.as_deref(),
                hidden,
            )
            .and_then(|scene| {
                crate::render::Uploaded::upload(
                    &self.state.device,
                    &self.state.queue,
                    &scene,
                    Some(&loaded.uploaded),
                )
            })
            .ok()
        };
        let mut camera_view = camera.view(aspect).ok()?;
        camera_view.tint = crate::views::tint_of(Some(view));
        let clip = match crate::views::display_clip(None, Some(view)) {
            Some(cut) => crate::cut_plane::ClipGpu::of(&cut, origin),
            None => crate::cut_plane::ClipGpu::none(),
        };
        let mode = if view.render_mode == RenderMode::SelectedOnly {
            RenderMode::Shaded
        } else {
            view.render_mode
        };
        target.renderer()?.draw(
            &self.state.device,
            &self.state.queue,
            filtered.as_ref().unwrap_or(&loaded.uploaded),
            camera_view,
            &self.stats,
            mode,
            &clip,
        );
        if let Some((drawn, _)) = self.views.previews.get_mut(&view.id) {
            *drawn = key;
        }
        self.views
            .previews
            .get(&view.id)
            .and_then(|(_, t)| t.texture())
    }

    /// The view panel of the inspector: for a new or a saved view the name, folder and marks,
    /// how a new view is built, and every display property, where nothing changes until Apply;
    /// for the free 3D view its display and "Save as view".
    pub(crate) fn view_panel(&mut self, ui: &mut egui::Ui) {
        if self.views.filter_edit.is_some() {
            self.filter_editor(ui);
            return;
        }
        if matches!(self.views.subject, Subject::FreeView(_)) && self.views.active().is_some() {
            // The active pane switched to a saved view: the panel follows it.
            self.show_view_properties();
        }
        let free = matches!(self.views.subject, Subject::FreeView(_));
        let mut submit = false;
        let mut reset = false;
        let mut all_appearance = false;
        let mut restart_pick = false;
        let mut copy_from: Option<Uuid7> = None;
        let busy = self.views.job.is_some();
        let model_bounds = self
            .loaded
            .as_ref()
            .and_then(|l| l.snapshot.engineering_bounds());
        let selection_bounds = self
            .editor
            .selected()
            .and_then(|s| beamcraft_engine::ElementId::parse(&s.id).ok())
            .and_then(|id| {
                self.loaded
                    .as_ref()
                    .and_then(|l| l.snapshot.element_bounds(&id).ok())
            });
        let grids: Vec<(String, Vec<String>)> = self
            .grid_scene
            .query_grids
            .iter()
            .map(|g| {
                let labels = beamcraft_engine::grid_query::segments(g)
                    .map(|s| s.into_iter().map(|l| l.label).collect())
                    .unwrap_or_default();
                (g.name.clone(), labels)
            })
            .collect();
        let sources: Vec<(Uuid7, String)> = self
            .views
            .book
            .views()
            .iter()
            .map(|v| (v.id, v.name.clone()))
            .collect();
        let picking = self.views.picking.clone();
        let default_locked = self
            .views
            .form
            .id
            .and_then(|id| self.views.book.get(&id))
            .is_some_and(|v| v.is_default);
        // The action bar keeps one height: the outcome goes to the status bar, so the buttons
        // stay pinned at the bottom whatever was said.
        let footer = 40.0;
        let header = {
            let shown = self.views.form.id.and_then(|id| self.views.book.get(&id));
            let kind = shown.map(crate::views::view_kind);
            TypeBox {
                icon: kind.map_or(theme::Icon::NewView, crate::views::ViewKind::icon),
                kind: match (kind, self.views.subject) {
                    (Some(kind), _) => kind_name(kind).to_owned(),
                    (None, Subject::NewView) => "New view".to_owned(),
                    (None, _) => "3D view".to_owned(),
                },
            }
        };
        let folders: Vec<String> = self.views.book.folders().map(str::to_owned).collect();
        let presets = self.preset_choices();
        let filters = self.filter_choices();
        let overlay_state = self
            .views
            .form
            .id
            .and_then(|id| self.views.overlays.get(&id))
            .map_or(OverlayState::default(), |o| OverlayState {
                waiting: !o.complete,
                refused: o.refused.clone(),
            });
        let mut edit_filter: Option<Option<String>> = None;
        let mut chosen_preset: Option<beamcraft_engine::presets::ViewPreset> = None;
        let mut save_preset: Option<String> = None;
        let mut preset_name = self.views.preset_name.clone();
        let preset_field = preset_name.is_some();
        let mut toggle_preset_field = false;
        let vg_id = egui::Id::unique("view-vg-open");
        let mut vg_open = ui.data(|d| d.get_temp::<bool>(vg_id).unwrap_or(false));
        let display_id = egui::Id::unique("view-display-open");
        let mut display_open = ui.data(|d| d.get_temp::<bool>(display_id).unwrap_or(false));
        let scheme_id = egui::Id::unique("view-scheme-open");
        let mut scheme_open = ui.data(|d| d.get_temp::<bool>(scheme_id).unwrap_or(false));
        egui::ScrollArea::vertical()
            .id_salt("view-form")
            .max_height((ui.available_height() - footer).max(0.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    // The saved view the form edits: a row whose value differs from it is marked.
                    let saved = self
                        .views
                        .form
                        .id
                        .and_then(|id| self.views.book.get(&id))
                        .map(crate::views::Form::from_view);
                    let live_span = self.live_span();
                    let form = &mut self.views.form;
                    // The type box at the top, as Revit's properties palette shows the kind of
                    // what is selected: the view's kind and its name.
                    type_box(ui, &header);
                    if form.id.is_none() {
                        ui.separator();
                        ui.label(egui::RichText::new(form.construction.label()).strong());
                        match form.construction {
                            Construction::PlanXy
                            | Construction::ElevationXz
                            | Construction::ElevationYz => {
                                text(ui, "Plane offset (mm)", &mut form.offset)
                            }
                            Construction::TwoPoints
                            | Construction::ThreePoints
                            | Construction::Surface
                            | Construction::SurfaceEdge => {
                                let done = picking.as_ref().map_or(0, Picking::done);
                                ui.label(format!(
                                    "{done} of {} picked in the model",
                                    form.construction.picks()
                                ));
                                if let Some(p) = &picking {
                                    ui.small(p.construction.prompt(p.done()));
                                }
                                restart_pick = ui.button("Restart picking").clicked();
                            }
                            Construction::WorkPlane => {
                                ui.small("The view lies on the current work plane.");
                            }
                            Construction::CurrentCamera => {
                                ui.small("The view keeps the camera as it is now.");
                            }
                            Construction::GridAxes => {
                                if grids.is_empty() {
                                    ui.small("The model has no grid.");
                                }
                                form.grid = form.grid.min(grids.len().saturating_sub(1));
                                egui::ComboBox::from_id_salt("view-grid")
                                    .selected_text(
                                        grids.get(form.grid).map_or("choose…", |(n, _)| n),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (i, (name, _)) in grids.iter().enumerate() {
                                            ui.selectable_value(&mut form.grid, i, name);
                                        }
                                    });
                                ui.horizontal(|ui| {
                                    ui.selectable_value(
                                        &mut form.axis_batch,
                                        AxisBatch::Single,
                                        "One axis",
                                    );
                                    ui.selectable_value(
                                        &mut form.axis_batch,
                                        AxisBatch::Families,
                                        "Batch by family",
                                    );
                                });
                                match form.axis_batch {
                                    AxisBatch::Single => {
                                        let labels = grids
                                            .get(form.grid)
                                            .map(|(_, l)| l.clone())
                                            .unwrap_or_default();
                                        egui::ComboBox::from_id_salt("view-axis")
                                            .selected_text(if form.axis.is_empty() {
                                                "choose an axis"
                                            } else {
                                                form.axis.as_str()
                                            })
                                            .show_ui(ui, |ui| {
                                                for label in labels {
                                                    ui.selectable_value(
                                                        &mut form.axis,
                                                        label.clone(),
                                                        label,
                                                    );
                                                }
                                            });
                                    }
                                    AxisBatch::Families => {
                                        for (i, family) in ["U", "V"].into_iter().enumerate() {
                                            if let (Some(pick), Some(prefix)) =
                                                (form.families.get_mut(i), form.prefixes.get_mut(i))
                                            {
                                                axis_combo(
                                                    ui,
                                                    &format!("view-family-{family}"),
                                                    &format!("{family} axes"),
                                                    pick,
                                                );
                                                text(ui, "Name prefix", prefix);
                                            }
                                        }
                                    }
                                }
                            }
                            Construction::Level => {
                                let mut batch = form.levels.is_some();
                                ui.horizontal(|ui| {
                                    ui.selectable_value(&mut batch, false, "One level");
                                    ui.selectable_value(&mut batch, true, "Batch");
                                });
                                match (batch, form.levels) {
                                    (false, _) => {
                                        form.levels = None;
                                        text(ui, "Level name", &mut form.level);
                                    }
                                    (true, current) => {
                                        let mut pick = current.unwrap_or(AxisPick::All);
                                        axis_combo(ui, "view-levels", "Levels", &mut pick);
                                        form.levels = Some(pick);
                                    }
                                }
                                ui.small("Batch views are named by the prefix and the level.");
                            }
                            Construction::Part | Construction::Assembly => {
                                let label = match form.side {
                                    SideChoice::Four => "four views",
                                    SideChoice::One(side) => side.as_str(),
                                };
                                egui::ComboBox::from_id_salt("view-side")
                                    .selected_text(label)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut form.side,
                                            SideChoice::Four,
                                            "four views",
                                        );
                                        for side in [
                                            PartSide::Front,
                                            PartSide::Back,
                                            PartSide::Top,
                                            PartSide::Bottom,
                                            PartSide::End,
                                            PartSide::Spatial,
                                        ] {
                                            ui.selectable_value(
                                                &mut form.side,
                                                SideChoice::One(side),
                                                side.as_str(),
                                            );
                                        }
                                    });
                                ui.small("Select the part first; the views frame its geometry.");
                            }
                        }
                        egui::ComboBox::from_id_salt("view-copy-from")
                            .selected_text("Copy display settings from…")
                            .show_ui(ui, |ui| {
                                for (id, name) in &sources {
                                    if ui.button(name).clicked() {
                                        copy_from = Some(*id);
                                    }
                                }
                            });
                    }
                    ui.add_space(4.0);
                    // As Revit's properties palette: the view's graphics are summarised in the table
                    // and edited in their own windows.
                    section(ui, "Graphics", true, |ui| {
                        let hidden = crate::views::listed_kinds()
                            .filter(|k| !form.hidden.is_visible(*k))
                            .count();
                        let vg = match (form.overrides.len(), hidden) {
                            (0, 0) => "Edit…".to_owned(),
                            (f, h) => format!("Edit… ({f} filters, {h} hidden)"),
                        };
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| {
                                o.hidden != form.hidden || o.overrides != form.overrides
                            }),
                        );
                        if crate::kit::prop_button(ui, "Visibility/Graphics", &vg) {
                            vg_open = !vg_open;
                        }
                        let display = format!("{} · Edit…", mode_name(form.mode));
                        crate::kit::mark_next_changed(
                            ui,
                            saved
                                .as_ref()
                                .is_some_and(|o| o.mode != form.mode || o.grids != form.grids),
                        );
                        if crate::kit::prop_button(ui, "Graphic Display", &display) {
                            display_open = !display_open;
                        }
                        let scheme = if form.color_on {
                            "Override · Edit…".to_owned()
                        } else {
                            "By model · Edit…".to_owned()
                        };
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| {
                                o.color_on != form.color_on
                                    || o.color != form.color
                                    || o.opacity != form.opacity
                            }),
                        );
                        if crate::kit::prop_button(ui, "Color Scheme", &scheme) {
                            scheme_open = !scheme_open;
                        }
                    });
                    section(ui, "Camera", true, |ui| {
                        crate::kit::mark_next_changed(
                            ui,
                            saved
                                .as_ref()
                                .is_some_and(|o| o.projection != form.projection),
                        );
                        row(ui, "Projection", |ui| {
                            egui::ComboBox::from_id_salt("view-projection")
                                .selected_text(match form.projection {
                                    Projection::Orthographic => "Orthographic",
                                    Projection::Perspective => "Perspective",
                                })
                                .width((ui.available_width() - FIT).max(40.0))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut form.projection,
                                        Projection::Orthographic,
                                        "Orthographic",
                                    );
                                    ui.selectable_value(
                                        &mut form.projection,
                                        Projection::Perspective,
                                        "Perspective",
                                    );
                                });
                        });
                        // The field follows the pane's zoom while it has never been edited,
                        // so the form never holds a height the user zoomed away from and
                        // Apply cannot re-zoom the viewport with a stale one. Once the user
                        // types a height, their text stays until the panel reopens.
                        let span_id = egui::Id::unique("view-form-span");
                        let edited_id = egui::Id::unique(("view-form-span-edited", form.id));
                        let mut span_edited =
                            ui.data(|d| d.get_temp::<bool>(edited_id).unwrap_or(false));
                        if !span_edited
                            && !ui.memory(|m| m.has_focus(span_id))
                            && let Some(live) = &live_span
                        {
                            form.span.clone_from(live);
                        }
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| o.span != form.span),
                        );
                        let mut typed = None;
                        row(ui, "Height, mm", |ui| {
                            typed = Some(
                                ui.add(
                                    egui::TextEdit::singleline(&mut form.span)
                                        .id_salt(span_id)
                                        .desired_width((ui.available_width() - FIT).max(40.0))
                                        .char_limit(crate::views::FIELD),
                                ),
                            );
                        });
                        if typed.is_some_and(|r| r.changed()) {
                            span_edited = true;
                        }
                        ui.data_mut(|d| d.insert_temp(edited_id, span_edited));
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| o.rotation != form.rotation),
                        );
                        row(ui, "Rotation, °", |ui| {
                            let [z, x] = &mut form.rotation;
                            let half = ((ui.available_width() - 8.0 - FIT) / 2.0).max(30.0);
                            ui.add(
                                egui::TextEdit::singleline(z)
                                    .hint_text("about Z")
                                    .desired_width(half),
                            );
                            ui.add(
                                egui::TextEdit::singleline(x)
                                    .hint_text("about X")
                                    .desired_width(half),
                            );
                        });
                    });
                    section(ui, "Extents", true, |ui| {
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| {
                                o.depth_on != form.depth_on || o.depth != form.depth
                            }),
                        );
                        row(ui, "Depth", |ui| {
                            let label = if form.depth_on {
                                "Limited"
                            } else {
                                "Unlimited"
                            };
                            egui::ComboBox::from_id_salt("view-depth")
                                .selected_text(label)
                                .width((ui.available_width() - FIT).max(40.0))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut form.depth_on, false, "Unlimited");
                                    ui.selectable_value(&mut form.depth_on, true, "Limited");
                                });
                        });
                        if form.depth_on {
                            let [above, below] = &mut form.depth;
                            text(ui, "Towards, mm", above);
                            text(ui, "Behind, mm", below);
                        }
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| o.area != form.area),
                        );
                        row(ui, "Work area", |ui| {
                            let label = if form.area.is_some() {
                                "Box"
                            } else {
                                "Whole model"
                            };
                            let mut area = None;
                            egui::ComboBox::from_id_salt("view-area")
                                .selected_text(label)
                                .width((ui.available_width() - FIT).max(40.0))
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(form.area.is_none(), "Whole model")
                                        .clicked()
                                    {
                                        form.area = None;
                                    }
                                    if ui
                                        .add_enabled(
                                            model_bounds.is_some(),
                                            egui::Button::selectable(false, "Bounds of the model"),
                                        )
                                        .clicked()
                                    {
                                        area = model_bounds;
                                    }
                                    if ui
                                        .add_enabled(
                                            selection_bounds.is_some(),
                                            egui::Button::selectable(
                                                false,
                                                "Bounds of the selection",
                                            ),
                                        )
                                        .clicked()
                                    {
                                        area = selection_bounds;
                                    }
                                });
                            if let Some(b) = area {
                                form.area =
                                    beamcraft_engine::views::WorkArea::new(b.min, b.max).ok();
                            }
                        });
                    });
                    section(ui, "Identity Data", true, |ui| {
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| o.name != form.name),
                        );
                        row(ui, "View name", |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut form.name)
                                    .hint_text("Unnamed: temporary")
                                    .desired_width((ui.available_width() - FIT).max(60.0))
                                    .char_limit(crate::views::FIELD),
                            )
                            .on_hover_text("A view without a name is temporary: it is not saved");
                        });
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| o.folder != form.folder),
                        );
                        row(ui, "Folder", |ui| {
                            let shown = if form.folder.is_empty() {
                                "(top level)".to_owned()
                            } else {
                                form.folder.clone()
                            };
                            egui::ComboBox::from_id_salt("view-folder")
                                .selected_text(shown)
                                .width((ui.available_width() - FIT).max(40.0))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut form.folder,
                                        String::new(),
                                        "(top level)",
                                    );
                                    for folder in &folders {
                                        ui.selectable_value(
                                            &mut form.folder,
                                            folder.clone(),
                                            folder.as_str(),
                                        );
                                    }
                                });
                        });
                        // Like the part presets of a beam or a column: picking one fills the form,
                        // every field stays editable, and Apply keeps it.
                        row(ui, "View preset", |ui| {
                            let width = (ui.available_width() - 30.0 - FIT).max(40.0);
                            egui::ComboBox::from_id_salt("view-preset")
                                .selected_text("Fill from…")
                                .width(width)
                                .show_ui(ui, |ui| {
                                    if presets.is_empty() {
                                        ui.label("No view presets yet.");
                                    }
                                    for preset in &presets {
                                        if ui.button(&preset.name).clicked() {
                                            chosen_preset = Some(preset.clone());
                                        }
                                    }
                                });
                            if theme::icon_toggle(
                                ui,
                                theme::Icon::Plus,
                                "Save this view's display as a view preset",
                                preset_field,
                            )
                            .clicked()
                            {
                                toggle_preset_field = true;
                            }
                        });
                        if let Some(name) = preset_name.as_mut() {
                            row(ui, "Preset name", |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(name)
                                        .hint_text("Name of the preset")
                                        .desired_width(
                                            (ui.available_width() - 60.0 - FIT).max(40.0),
                                        )
                                        .char_limit(64),
                                );
                                if ui.small_button("Save").clicked() {
                                    save_preset = Some(name.clone());
                                }
                            });
                        }
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| o.favorite != form.favorite),
                        );
                        row(ui, "Favourite", |ui| {
                            ui.checkbox(&mut form.favorite, "")
                                .on_hover_text("Listed in quick access");
                        });
                        crate::kit::mark_next_changed(
                            ui,
                            saved.as_ref().is_some_and(|o| o.default != form.default),
                        );
                        row(ui, "Default view", |ui| {
                            ui.add_enabled(
                                !default_locked,
                                egui::Checkbox::new(&mut form.default, ""),
                            )
                            .on_hover_text(if default_locked {
                                "The default view: mark another view to move it"
                            } else {
                                "The view the model opens on"
                            });
                        });
                    });
                    // Visibility/Graphics overrides, as Revit edits them in their own dialog:
                    // the element kinds the view shows and its filters.
                    let mut open = vg_open;
                    crate::kit::dialog(
                        "Visibility / Graphics",
                        "view-vg-window",
                        &mut open,
                        ui.ctx(),
                    )
                    .show(ui.ctx(), |ui| {
                        crate::kit::section(ui, "vg", "Model categories", true, |ui| {
                            for kind in crate::views::listed_kinds() {
                                let mut visible = form.hidden.is_visible(kind);
                                if ui.checkbox(&mut visible, kind.as_token()).changed() {
                                    form.hidden = if visible {
                                        form.hidden.showing(kind)
                                    } else {
                                        form.hidden.hiding(kind)
                                    };
                                }
                            }
                        });
                        crate::kit::section(ui, "vg", "Filters", true, |ui| {
                            filters_table(
                                ui,
                                &mut form.overrides,
                                &filters,
                                &overlay_state,
                                &mut edit_filter,
                            );
                        });
                        ui.add_space(crate::kit::SPACE_S);
                        ui.label(
                            egui::RichText::new("Changes apply to the view with Apply.")
                                .size(11.0)
                                .color(theme::Palette::current(ui).muted),
                        );
                    });
                    vg_open = open;
                    let mut open = display_open;
                    crate::kit::dialog(
                        "Graphic Display Options",
                        "view-display-window",
                        &mut open,
                        ui.ctx(),
                    )
                    .show(ui.ctx(), |ui| {
                        crate::kit::section(ui, "display", "Model display", true, |ui| {
                            row(ui, "Visual style", |ui| {
                                egui::ComboBox::from_id_salt("view-mode")
                                    .selected_text(mode_name(form.mode))
                                    .width((ui.available_width() - FIT).max(40.0))
                                    .show_ui(ui, |ui| {
                                        for mode in RenderMode::ALL {
                                            ui.selectable_value(
                                                &mut form.mode,
                                                mode,
                                                mode_name(mode),
                                            );
                                        }
                                    });
                            });
                            row(ui, "Grids", |ui| {
                                ui.checkbox(&mut form.grids, "Shown");
                            });
                        });
                    });
                    display_open = open;
                    let mut open = scheme_open;
                    crate::kit::dialog("Color Scheme", "view-scheme-window", &mut open, ui.ctx())
                        .show(ui.ctx(), |ui| {
                            crate::kit::section(ui, "scheme", "Model colour", true, |ui| {
                                row(ui, "Colour", |ui| {
                                    ui.checkbox(&mut form.color_on, "Override");
                                    if form.color_on {
                                        ui.color_edit_button_rgb(&mut form.color);
                                    }
                                });
                                row(ui, "Opacity", |ui| {
                                    ui.add(
                                        egui::Slider::new(&mut form.opacity, 0.0..=1.0)
                                            .fixed_decimals(2),
                                    );
                                });
                                if form.id.is_some() {
                                    row(ui, "", |ui| {
                                        let link = egui::RichText::new("Use in every view")
                                            .size(11.0)
                                            .color(theme::Palette::current(ui).link);
                                        if ui
                                            .add(egui::Button::new(link).frame(false))
                                            .on_hover_text(
                                                "Give every view this colour and opacity",
                                            )
                                            .clicked()
                                        {
                                            all_appearance = true;
                                        }
                                    });
                                }
                            });
                        });
                    scheme_open = open;
                });
            });
        ui.data_mut(|d| {
            d.insert_temp(vg_id, vg_open);
            d.insert_temp(display_id, display_open);
            d.insert_temp(scheme_id, scheme_open);
        });
        ui.add_space(4.0);
        let mut cancel = false;
        // A saved view changes only on Apply; the buttons wake up with the first edit.
        let dirty = self
            .views
            .form
            .id
            .and_then(|id| self.views.book.get(&id))
            .is_none_or(|v| self.views.form != crate::views::Form::from_view(v));
        ui.horizontal(|ui| {
            let waiting = self
                .views
                .picking
                .as_ref()
                .is_some_and(|p| p.done() < p.construction.picks());
            let apply = theme::button(
                ui,
                theme::Kind::Primary,
                if free { "Save as view" } else { "Apply" },
                !busy && !waiting && dirty,
            );
            submit = if self.views.form.id.is_none() {
                apply.on_hover_text(if free {
                    "Keep the 3D view as a saved view"
                } else {
                    "Create the view"
                })
            } else {
                apply
            }
            .clicked();
            reset = self.views.form.id.is_some()
                && theme::button(ui, theme::Kind::Secondary, "Reset", !busy && dirty).clicked();
            cancel = theme::button(ui, theme::Kind::Secondary, "Close", true).clicked();
            if busy {
                ui.spinner();
            }
        });
        if cancel {
            self.views.picking = None;
            self.set_bench(Bench::Select);
            return;
        }
        if let Some(source) = copy_from.and_then(|id| self.views.book.get(&id).cloned()) {
            self.views.form.copy_display(&source);
        }
        self.views.preset_name = if toggle_preset_field {
            (!preset_field).then(|| self.views.form.name.trim().to_owned())
        } else {
            preset_name
        };
        if let Some(preset) = chosen_preset {
            self.views.form.copy_preset(&preset);
            self.view_note(Ok(format!(
                "Filled from the view preset “{}”; Apply keeps it.",
                preset.name
            )));
        }
        if let Some(name) = save_preset {
            self.save_preset_from_form(&name);
        }
        if let Some(target) = edit_filter {
            self.open_filter_editor(target);
        }
        if restart_pick {
            let construction = self.views.form.construction;
            self.views.form.picked = false;
            self.views.picking = Some(Picking {
                construction,
                ..Picking::default()
            });
        }
        if reset && let Some(id) = self.views.form.id {
            self.edit_view(id);
        }
        if all_appearance {
            let color = self
                .views
                .form
                .color_on
                .then(|| self.views.form.color.map(f64::from));
            let opacity = f64::from(self.views.form.opacity).clamp(0.0, 1.0);
            let all: Vec<ModelView> = self.views.book.views().to_vec();
            for mut view in all {
                view.color = color;
                view.opacity = opacity;
                if let Err(e) = self.views.book.update(view) {
                    self.view_note(Err(e.to_string()));
                }
            }
            self.persist_views();
        }
        if submit {
            self.submit_view();
        }
    }

    /// Opens a pane on `view` (or the free 3D view) and focuses it, up to [`MAX_WINDOWS`] panes
    /// in all. A view brings its session camera, or one built from its definition; the free 3D
    /// view copies the active pane's camera. Before the first work-area frame the pane is left
    /// unplaced, and that frame places it.
    pub(crate) fn open_pane(&mut self, id: Option<Uuid7>) {
        if self.views.panes.len() >= MAX_WINDOWS {
            self.view_note(Err(format!(
                "At most {MAX_WINDOWS} view windows are open at once."
            )));
            return;
        }
        let id = id.or_else(|| self.default_view_id());
        let view = id.and_then(|id| self.views.book.get(&id)).cloned();
        let active_camera = self.views.active_pane().map(|p| p.camera);
        let camera = match &view {
            // The default view starts where the user is looking; the others where they were.
            Some(view) => match self.views.cameras.get(&view.id).copied() {
                Some(camera) => Ok(camera),
                None => match active_camera.filter(|_| view.is_default) {
                    Some(camera) => Ok(camera),
                    None => Camera::from_view(view, self.view_origin(), self.view_extent()),
                },
            },
            None => active_camera.map_or_else(|| Camera::new([0.0; 3], 1000.0, 1.6), Ok),
        };
        let camera = match camera {
            Ok(camera) => camera,
            Err(e) => {
                self.view_note(Err(e.to_string()));
                return;
            }
        };
        let from = self.views.active_pane().map(|p| p.view);
        self.views.pane_serial = self.views.pane_serial.saturating_add(1);
        let key = self.views.pane_serial;
        // The tab layout places it: a new tab of the active group (see `apply_layout`).
        let pane = ViewPane::new(view.as_ref(), camera, key);
        self.views.panes.push(pane);
        if let Some(view) = view
            && !self.views.open.contains(&view.id)
        {
            self.views.open.push(view.id);
        }
        self.pane_focused(from);
    }

    /// Opens `id` (or the default view) in a new tab group on the right of the active one, so
    /// two views sit side by side.
    pub(crate) fn open_beside(&mut self, id: Option<Uuid7>) {
        let group = self.views.dock.active_group();
        let before = self.views.panes.len();
        self.open_pane(id);
        if self.views.panes.len() == before {
            return;
        }
        if let (Some(key), Some(group)) = (self.views.panes.last().map(|p| p.key), group) {
            self.views.dock.insert(
                key,
                crate::dock::Drop::Beside(group, crate::dock::Side::Right),
            );
            self.views.dock.select(key);
        }
        if let Some(area) = self.views.area {
            self.apply_layout(area);
        }
    }

    /// Closes every view tab but the active one, as Revit's Close Inactive does.
    pub(crate) fn close_inactive(&mut self) {
        let active = self.views.active_pane().map(|p| p.key);
        let others: Vec<u64> = self
            .views
            .panes
            .iter()
            .map(|p| p.key)
            .filter(|k| Some(*k) != active)
            .collect();
        for key in others {
            self.close_pane(key);
        }
    }

    /// Makes pane `key` the active one: raises it to the top and installs its view's plane as
    /// the input frame (the free 3D view gets its own work plane back).
    pub(crate) fn focus_pane(&mut self, key: u64) {
        let Some(at) = self.views.panes.iter().position(|p| p.key == key) else {
            return;
        };
        if at + 1 == self.views.panes.len() {
            return;
        }
        let from = self.views.active_pane().map(|p| p.view);
        let pane = self.views.panes.remove(at);
        self.views.panes.push(pane);
        self.pane_focused(from);
    }

    /// Closes pane `key`: its view keeps the camera for the session, its target is freed, and
    /// the pane under it becomes active. The last pane may be closed.
    pub(crate) fn close_pane(&mut self, key: u64) {
        let Some(at) = self.views.panes.iter().position(|p| p.key == key) else {
            return;
        };
        let was_active = at + 1 == self.views.panes.len();
        let mut pane = self.views.panes.remove(at);
        if let Some(id) = pane.view {
            self.views.cameras.insert(id, pane.camera);
        }
        pane.target.release(&self.state);
        if self.views.focus_press == Some(key) {
            self.views.focus_press = None;
        }
        if was_active {
            self.pane_focused(Some(pane.view));
        }
    }

    /// The input frame and display follow a newly active pane. `from` is the view of the pane
    /// that was active before (`Some(None)` a pane without a view, `None` no pane at all). The
    /// default view and no view leave the model's own work plane; any other view installs its
    /// plane. A plane change cancels an unfinished view pick and the tool drafts, as
    /// `activate_view` does.
    fn pane_focused(&mut self, from: Option<Option<Uuid7>>) {
        let to = self.views.active_pane().map(|p| p.view);
        let default = self.default_view_id();
        let free = |id: Option<Uuid7>| id.is_none() || id == default;
        let plane_of = |views: &crate::views::Views, id: Option<Uuid7>| {
            id.filter(|id| Some(*id) != default)
                .and_then(|id| views.book.get(&id))
                .map(|v| v.plane)
        };
        let from_view = from.flatten();
        let to_view = to.flatten();
        let mut changed = false;
        match (free(from_view), free(to_view)) {
            // Free to view: the free work plane is kept until a free view comes back.
            (true, false) => {
                if let Some(plane) = plane_of(&self.views, to_view) {
                    self.views_input = Some(self.tools.plane);
                    self.install_plane(Some(plane));
                    changed = true;
                }
            }
            // View to free (or to no pane): the free work plane comes back.
            (false, true) => {
                if let Some(plane) = self.views_input.take() {
                    self.install_plane(plane);
                    changed = true;
                }
            }
            // View to view: only a different plane is installed.
            (false, false) => {
                let next = plane_of(&self.views, to_view);
                if next.is_some() && next != plane_of(&self.views, from_view) {
                    self.install_plane(next);
                    changed = true;
                }
            }
            (true, true) => {}
        }
        if changed {
            self.tool_note = None;
            if self.views.picking.take().is_some() {
                self.view_note(Ok("The view pick was cancelled.".into()));
            }
        }
        self.clear_member_preview();
        self.plate_hover = None;
        if self.views.shown != self.views.active_hidden() {
            self.recolour();
        }
        if self.bench == Bench::View && self.views.subject != Subject::NewView {
            self.show_view_properties();
        }
    }
}

/// What the type box at the top of the view properties shows.
struct TypeBox {
    icon: theme::Icon,
    kind: String,
}

/// The name of a display mode, as the visual style of a view.
fn mode_name(mode: RenderMode) -> &'static str {
    match mode {
        RenderMode::Wireframe => "Wireframe",
        RenderMode::Transparent => "Transparent",
        RenderMode::Grey => "Grey shaded",
        RenderMode::Shaded => "Shaded",
        RenderMode::SelectedOnly => "Selected only",
        _ => "Other",
    }
}

/// The kind of a view in a few words, for the type box.
fn kind_name(kind: crate::views::ViewKind) -> &'static str {
    match kind {
        crate::views::ViewKind::Plan => "Plan",
        crate::views::ViewKind::Vertical => "Elevation / section",
        crate::views::ViewKind::Inclined => "Inclined view",
    }
}

/// The type box: a framed icon with the view's kind and name beside it, as the top of Revit's
/// properties palette shows the type of what is selected.
fn type_box(ui: &mut egui::Ui, header: &TypeBox) {
    crate::kit::type_box(ui, header.icon, &header.kind);
}

/// What the filters table knows of the evaluation of the shown view's filters.
#[derive(Default)]
struct OverlayState {
    /// Some filters are still being evaluated.
    waiting: bool,
    /// Filters the evaluator refused, with the reason.
    refused: Vec<(String, String)>,
}

/// The view's filters table, as Revit's Visibility/Graphics Filters tab: one row per filter in
/// order (the first whose filter matches an element decides) with whether what it matches is
/// visible, its colour and its transparency, and buttons to reorder or remove it; under it the
/// filters that can be added, and a new filter. A click on a filter's name edits the filter.
fn filters_table(
    ui: &mut egui::Ui,
    overrides: &mut Vec<beamcraft_engine::views::FilterOverride>,
    filters: &[beamcraft_engine::views::ViewFilter],
    state: &OverlayState,
    edit: &mut Option<Option<String>>,
) {
    let palette = theme::Palette::current(ui);
    if overrides.is_empty() {
        ui.label(
            egui::RichText::new("No filters: the view shows every element as it draws it.")
                .size(11.0)
                .color(palette.muted),
        );
    }
    let mut remove = None;
    let mut raise = None;
    let count = overrides.len();
    for (at, o) in overrides.iter_mut().enumerate() {
        let refused = state.refused.iter().find(|(name, _)| *name == o.filter);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.checkbox(&mut o.visible, "")
                .on_hover_text("Visible: unchecked hides what the filter matches");
            let name = egui::RichText::new(&o.filter)
                .size(12.0)
                .color(if refused.is_some() {
                    palette.error
                } else if o.visible {
                    palette.text
                } else {
                    palette.muted
                });
            let label = ui
                .add(
                    egui::Label::new(name)
                        .truncate()
                        .sense(egui::Sense::click()),
                )
                .on_hover_text(match refused {
                    Some((_, reason)) => format!("Not applied: {reason}"),
                    None => "Click to edit the filter".to_owned(),
                });
            if label.clicked() {
                *edit = Some(Some(o.filter.clone()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if theme::icon_toggle(ui, theme::Icon::Close, "Remove from the view", false)
                    .clicked()
                {
                    remove = Some(at);
                }
                if at > 0
                    && ui
                        .small_button("↑")
                        .on_hover_text("Earlier: decides first")
                        .clicked()
                {
                    raise = Some(at);
                }
                let mut percent = (o.transparency * 100.0).round();
                if ui
                    .add_enabled(
                        o.visible,
                        egui::DragValue::new(&mut percent)
                            .range(0.0..=100.0)
                            .speed(1.0)
                            .suffix(" %"),
                    )
                    .on_hover_text("Transparency (seen in the transparent display)")
                    .changed()
                {
                    o.transparency = (percent / 100.0).clamp(0.0, 1.0);
                }
                let mut colour = o.color.map_or([0.85, 0.45, 0.2], |c| c.map(|v| v as f32));
                if o.color.is_some() && ui.color_edit_button_rgb(&mut colour).changed() {
                    o.color = Some(colour.map(f64::from));
                }
                let mut coloured = o.color.is_some();
                if ui
                    .add_enabled(o.visible, egui::Checkbox::without_text(&mut coloured))
                    .on_hover_text("Colour what the filter matches")
                    .changed()
                {
                    o.color = coloured.then(|| colour.map(f64::from));
                }
            });
        });
    }
    if let Some(at) = remove
        && at < overrides.len()
    {
        overrides.remove(at);
    }
    if let Some(at) = raise
        && at > 0
        && at < overrides.len()
    {
        overrides.swap(at, at - 1);
    }
    ui.horizontal(|ui| {
        let unused: Vec<&beamcraft_engine::views::ViewFilter> = filters
            .iter()
            .filter(|f| {
                !overrides
                    .iter()
                    .any(|o| o.filter.to_lowercase() == f.name.to_lowercase())
            })
            .collect();
        ui.add_enabled_ui(
            !unused.is_empty() && count < beamcraft_engine::view_filter::MAX_OVERRIDES,
            |ui| {
                egui::ComboBox::from_id_salt("view-add-filter")
                    .selected_text("Add filter…")
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for filter in &unused {
                            if ui.button(&filter.name).clicked() {
                                overrides.push(beamcraft_engine::views::FilterOverride::new(
                                    &filter.name,
                                ));
                            }
                        }
                    });
            },
        );
        if ui.button("New filter…").clicked() {
            *edit = Some(None);
        }
    });
    if state.waiting {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.small("Finding what the filters match…");
        });
    }
}

/// The field choices of the filter editor.
const FIELDS: [(&str, &str); 4] = [
    ("type", "Type"),
    ("name", "Name"),
    ("class", "Class"),
    ("attribute", "Attribute"),
];

/// The operators a field takes, with their labels.
fn operators(
    field: &beamcraft_engine::views::FilterField,
) -> Vec<(beamcraft_engine::filter::Operator, &'static str)> {
    use beamcraft_engine::filter::Operator as Op;
    use beamcraft_engine::views::FilterField;
    let text = [
        (Op::Eq, "equals"),
        (Op::Ne, "does not equal"),
        (Op::Contains, "contains"),
        (Op::StartsWith, "starts with"),
    ];
    let ordered = [
        (Op::Eq, "equals"),
        (Op::Ne, "does not equal"),
        (Op::Lt, "is less than"),
        (Op::Le, "is at most"),
        (Op::Gt, "is greater than"),
        (Op::Ge, "is at least"),
    ];
    match field {
        FilterField::Type | FilterField::Name => text.to_vec(),
        FilterField::Class => ordered.to_vec(),
        FilterField::Attribute(_) => {
            let mut all = ordered.to_vec();
            all.extend(text.into_iter().skip(2));
            all
        }
    }
}

/// What every row of one browser frame shares: the listed order (for Shift ranges) and the
/// names of the view presets a row's menu can apply.
pub(crate) struct BrowserContext<'a> {
    order: &'a [Uuid7],
    presets: &'a [String],
}

/// What a browser click asks for; run after the browser releases its borrows.
pub(crate) enum BrowserAction {
    Click(Uuid7, bool, bool, Vec<Uuid7>),
    Activate(Uuid7),
    Window(Uuid7),
    Properties(Uuid7),
    Favorite(Uuid7),
    SetDefault(Uuid7),
    Duplicate(Uuid7),
    NewFolder(String),
    ApplyPreset(Uuid7, String),
    SavePreset(Uuid7),
    Close(Uuid7),
    Delete(Uuid7),
    ConfirmDelete(Uuid7),
    DeleteFolder(String),
    /// Starts renaming the folder with this stored path in place.
    StartRename(String),
    /// Drops the rename in progress.
    CancelRename,
    /// Renames the folder (stored path) to the typed name, in the same parent.
    RenameFolder(String, String),
    OpenAll(Vec<Uuid7>),
    CloseAll(Vec<Uuid7>),
    Move(Uuid7, String),
}
