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

fn text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(110.0)
                .char_limit(crate::views::FIELD),
        );
    });
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

    /// Shows `id`, or the free 3D view for `None`: a pane that shows it already is focused;
    /// otherwise the active pane switches to it, and with no pane open one opens. The camera of a
    /// view opened before comes back as the user left it; its render mode, grid guides, hidden
    /// kinds and plane (the input frame) come from the view.
    pub(crate) fn activate_view(&mut self, id: Option<Uuid7>) {
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
        match current {
            Some(current) => {
                self.views.cameras.insert(current, camera);
            }
            None => {
                if let Some(pane) = self.views.active_pane_mut() {
                    pane.spatial = Some((camera, mode, grids));
                }
                self.views_input = Some(input);
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
                self.install_plane(Some(view.plane));
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

    /// Opens the view panel on a new view built by `construction`; the constructions that pick
    /// in the model start picking at once, and Apply creates the view.
    pub(crate) fn begin_view(&mut self, construction: Construction) {
        self.set_bench(Bench::View);
        if self.bench != Bench::View {
            self.view_note(Err("Wait for the running command to finish.".into()));
            return;
        }
        let mut form = Form {
            construction,
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
        form.name = self.views.book.next_default_name();
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
                self.views.form = Form {
                    construction: Construction::CurrentCamera,
                    name: self.views.book.next_default_name(),
                    ..Form::default()
                };
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
            self.install_plane(Some(view.plane));
            self.tool_note = None;
            if self.views.shown != self.views.active_hidden() {
                self.recolour();
            }
        }
        Ok(Some("The view was updated; the model is unchanged.".into()))
    }

    fn create_views_from_form(&mut self) -> Result<Option<String>, String> {
        let form = self.views.form.clone();
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
                        let projection = if side == PartSide::Spatial {
                            Projection::Perspective
                        } else {
                            Projection::Orthographic
                        };
                        Ok((name, plane, Some(span), Some(projection)))
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
            each.apply(&mut view)?;
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

    /// The browser: views grouped by folder, with search, quick access, favourites,
    /// multi-select, a context menu, cut/copy/paste and drag to a folder.
    pub(crate) fn view_browser(&mut self, ui: &mut egui::Ui) {
        use crate::views::BrowserTab;
        ui.horizontal(|ui| {
            for (tab, label) in [
                (BrowserTab::Views, "Views"),
                (BrowserTab::Drawings, "Drawings"),
                (BrowserTab::Schedules, "Schedules"),
            ] {
                ui.selectable_value(&mut self.views.tab, tab, label);
            }
        });
        if self.views.tab != BrowserTab::Views {
            ui.label(
                egui::RichText::new("Not listed in this browser yet.")
                    .size(10.0)
                    .color(crate::theme::Palette::current(ui).muted),
            );
            return;
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.views.search)
                .hint_text("Find views")
                .char_limit(128)
                .desired_width(ui.available_width()),
        );
        ui.horizontal(|ui| {
            ui.menu_button("New view", |ui| {
                for (construction, label) in Construction::ALL {
                    if ui.button(label).clicked() {
                        self.begin_view(construction);
                        ui.close();
                    }
                }
            });
            if ui.small_button("New folder").clicked() {
                self.views.new_folder = !self.views.new_folder;
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.views.quick_access, "Quick access");
            ui.checkbox(&mut self.views.favorites_only, "Favourites");
            ui.checkbox(&mut self.views.previews_on, "Previews");
        });
        if self.views.new_folder {
            text(ui, "Folder", &mut self.views.destination);
            ui.horizontal(|ui| {
                if ui.button("Create folder").clicked() {
                    let folder = self.views.destination.trim().to_owned();
                    match self.views.book.create_folder(&folder) {
                        Ok(()) => {
                            self.views.new_folder = false;
                            self.persist_views();
                        }
                        Err(e) => self.view_note(Err(e.to_string())),
                    }
                }
                if ui.button("Cancel").clicked() {
                    self.views.new_folder = false;
                }
            });
        }
        if let Some((id, cut)) = self.views.clipboard {
            ui.horizontal(|ui| {
                ui.label(if cut { "Move to" } else { "Copy to" });
                ui.add(
                    egui::TextEdit::singleline(&mut self.views.destination)
                        .hint_text("folder")
                        .desired_width(90.0)
                        .char_limit(crate::views::FIELD),
                );
                if ui.small_button("Paste").clicked() {
                    self.paste_view(id, cut);
                }
                if ui.small_button("×").clicked() {
                    self.views.clipboard = None;
                }
            });
        }
        if !self.views.selected.is_empty() {
            ui.horizontal(|ui| {
                ui.label(format!("{} selected", self.views.selected.len()));
                if ui.small_button("Open in windows").clicked() {
                    for id in self.views.selected.clone() {
                        self.open_pane(Some(id));
                    }
                }
            });
        }
        ui.separator();
        if ui
            .selectable_label(self.views.active().is_none(), "{3D}")
            .on_hover_text("The free 3D view of the whole model")
            .clicked()
        {
            self.activate_view(None);
        }
        let groups: Vec<(String, Vec<ModelView>)> = self
            .views
            .browser_groups()
            .into_iter()
            .map(|(f, v)| (f, v.into_iter().cloned().collect()))
            .collect();
        let mut actions: Vec<BrowserAction> = Vec::new();
        let mut budget = true;
        egui::ScrollArea::vertical()
            .id_salt("view-browser")
            .max_height(360.0)
            .show(ui, |ui| {
                for (folder, list) in &groups {
                    let order: Vec<Uuid7> = list.iter().map(|v| v.id).collect();
                    let header = egui::CollapsingHeader::new(if folder.is_empty() {
                        "Model views"
                    } else {
                        folder.as_str()
                    })
                    .id_salt(("view-folder", folder.as_str()))
                    .default_open(true)
                    .show(ui, |ui| {
                        if list.is_empty() && !folder.is_empty() {
                            if ui.small_button("Delete empty folder").clicked() {
                                actions.push(BrowserAction::DeleteFolder(folder.clone()));
                            }
                        } else {
                            ui.horizontal(|ui| {
                                if ui.small_button("Open all").clicked() {
                                    actions.push(BrowserAction::OpenAll(order.clone()));
                                }
                                if ui.small_button("Close all").clicked() {
                                    actions.push(BrowserAction::CloseAll(order.clone()));
                                }
                            });
                        }
                        for view in list {
                            if self.views.previews_on {
                                match self.view_preview(view, &mut budget) {
                                    Some(texture) => {
                                        ui.image((texture, egui::vec2(160.0, 100.0)));
                                    }
                                    None => {
                                        ui.small("Preparing the preview…");
                                    }
                                }
                            }
                            let selected = self.views.selected.contains(&view.id)
                                || self.views.active() == Some(view.id);
                            let label = format!(
                                "{}{}{}",
                                if view.favorite { "★ " } else { "" },
                                view.name,
                                if view.temporary { " (temporary)" } else { "" }
                            );
                            let response = ui.add(
                                egui::Button::selectable(selected, label)
                                    .sense(egui::Sense::click_and_drag()),
                            );
                            response.dnd_set_drag_payload(view.id);
                            if response.clicked() {
                                let m = ui.input(|i| i.modifiers);
                                actions.push(BrowserAction::Click(
                                    view.id,
                                    m.command,
                                    m.shift,
                                    order.clone(),
                                ));
                            }
                            if response.double_clicked() {
                                actions.push(BrowserAction::Properties(view.id));
                            }
                            response.context_menu(|ui| {
                                let mut item = |label: &str, action: BrowserAction| {
                                    if ui.button(label).clicked() {
                                        actions.push(action);
                                        ui.close();
                                    }
                                };
                                item("Open", BrowserAction::Activate(view.id));
                                item("Open in a new window", BrowserAction::Window(view.id));
                                item("Properties…", BrowserAction::Properties(view.id));
                                item(
                                    if view.favorite {
                                        "Remove from favourites"
                                    } else {
                                        "Add to favourites"
                                    },
                                    BrowserAction::Favorite(view.id),
                                );
                                item("Duplicate", BrowserAction::Duplicate(view.id));
                                item("Cut", BrowserAction::Clip(view.id, true));
                                item("Copy", BrowserAction::Clip(view.id, false));
                                item("Close", BrowserAction::Close(view.id));
                                item("Delete…", BrowserAction::Delete(view.id));
                            });
                        }
                    });
                    if let Some(id) = header.header_response.dnd_release_payload::<Uuid7>() {
                        actions.push(BrowserAction::Move(*id, folder.clone()));
                    }
                }
            });
        if let Some(id) = self.views.confirm_delete {
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
            ui.colored_label(crate::theme::Palette::current(ui).error, note);
        }
    }

    pub(crate) fn browser_action(&mut self, action: BrowserAction) {
        match action {
            BrowserAction::Click(id, ctrl, shift, order) => {
                if self.views.click(id, ctrl, shift, &order) {
                    self.activate_view(Some(id));
                }
            }
            BrowserAction::Activate(id) => self.activate_view(Some(id)),
            BrowserAction::Window(id) => self.open_pane(Some(id)),
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
            BrowserAction::Clip(id, cut) => {
                self.views.clipboard = Some((id, cut));
                if let Some(view) = self.views.book.get(&id) {
                    view.folder.clone_into(&mut self.views.destination);
                }
            }
            // Closing a view closes its panes; the work area may end up empty.
            BrowserAction::Close(id) => self.close_view(id),
            BrowserAction::Delete(id) => self.views.confirm_delete = Some(id),
            BrowserAction::ConfirmDelete(id) => {
                self.views.confirm_delete = None;
                self.close_view(id);
                self.views.forget(&id);
                self.views.book.remove(&id);
                self.persist_views();
            }
            BrowserAction::DeleteFolder(folder) => match self.views.book.delete_folder(&folder) {
                Ok(()) => self.persist_views(),
                Err(e) => self.view_note(Err(e.to_string())),
            },
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
        match self.views.book.insert(copy) {
            Ok(()) => self.persist_views(),
            Err(e) => self.view_note(Err(e.to_string())),
        }
    }

    fn paste_view(&mut self, id: Uuid7, cut: bool) {
        let folder = self.views.destination.trim().to_owned();
        let Some(view) = self.views.book.get(&id).cloned() else {
            self.views.clipboard = None;
            return;
        };
        if cut {
            let mut moved = view;
            moved.folder = folder;
            self.keep_view(moved);
        } else {
            let name = self
                .views
                .book
                .unique_name(&format!("{} copy", view.name), &[]);
            self.copy_view(&view, name, folder);
        }
        self.views.clipboard = None;
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
        if matches!(self.views.subject, Subject::FreeView(_)) {
            self.free_view_panel(ui);
            return;
        }
        let mut submit = false;
        let mut reset = false;
        let mut save_camera = false;
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
        let footer = if self.views.note.is_some() {
            140.0
        } else {
            90.0
        };
        egui::ScrollArea::vertical()
            .id_salt("view-form")
            .max_height((ui.available_height() - footer).max(0.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    let form = &mut self.views.form;
                    text(ui, "Name", &mut form.name);
                    text(ui, "Folder", &mut form.folder);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut form.favorite, "Favourite");
                        ui.checkbox(&mut form.temporary, "Temporary");
                    });
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
                    } else {
                        ui.horizontal(|ui| {
                            if ui
                                .button("Re-pick the plane")
                                .on_hover_text("Pick three points for a new view plane")
                                .clicked()
                            {
                                restart_pick = true;
                                form.construction = Construction::ThreePoints;
                            }
                            save_camera = ui
                                .button("Take the current camera")
                                .on_hover_text("The view plane and height become the camera's")
                                .clicked();
                        });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Projection");
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
                    text(ui, "Visible height (mm)", &mut form.span);
                    ui.label("Camera rotation (degrees)");
                    ui.horizontal(|ui| {
                        let [z, x] = &mut form.rotation;
                        ui.label("about Z");
                        ui.add(egui::TextEdit::singleline(z).desired_width(50.0));
                        ui.label("about X");
                        ui.add(egui::TextEdit::singleline(x).desired_width(50.0));
                    });
                    ui.checkbox(&mut form.depth_on, "Limit the view depth");
                    if form.depth_on {
                        let [above, below] = &mut form.depth;
                        text(ui, "Towards the viewer (mm)", above);
                        text(ui, "Behind the plane (mm)", below);
                    }
                    ui.horizontal(|ui| {
                        ui.label(match form.area {
                            Some(_) => "Work area: box",
                            None => "Work area: whole model",
                        });
                        let mut area = None;
                        if ui
                            .add_enabled(model_bounds.is_some(), egui::Button::new("Model"))
                            .clicked()
                        {
                            area = model_bounds;
                        }
                        if ui
                            .add_enabled(selection_bounds.is_some(), egui::Button::new("Selection"))
                            .clicked()
                        {
                            area = selection_bounds;
                        }
                        if let Some(b) = area {
                            form.area = beamcraft_engine::views::WorkArea::new(b.min, b.max).ok();
                        }
                        if ui.button("Clear").clicked() {
                            form.area = None;
                        }
                    });
                    egui::ComboBox::from_id_salt("view-render-mode")
                        .selected_text(form.mode.as_token())
                        .show_ui(ui, |ui| {
                            for mode in RenderMode::ALL {
                                ui.selectable_value(&mut form.mode, mode, mode.as_token());
                            }
                        });
                    ui.checkbox(&mut form.grids, "Show grids");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut form.color_on, "Colour");
                        if form.color_on {
                            ui.color_edit_button_rgb(&mut form.color);
                        }
                    });
                    ui.add(egui::Slider::new(&mut form.opacity, 0.0..=1.0).text("Opacity"));
                    all_appearance = form.id.is_some()
                        && ui
                            .small_button("Apply colour and opacity to every view")
                            .clicked();
                    ui.collapsing("Object visibility", |ui| {
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
                });
            });
        ui.add_space(8.0);
        let mut cancel = false;
        ui.horizontal(|ui| {
            let waiting = self
                .views
                .picking
                .as_ref()
                .is_some_and(|p| p.done() < p.construction.picks());
            let apply = theme::button(ui, theme::Kind::Primary, "Apply", !busy && !waiting);
            submit = if self.views.form.id.is_none() {
                apply.on_hover_text("Create the view")
            } else {
                apply
            }
            .clicked();
            reset = self.views.form.id.is_some()
                && theme::button(ui, theme::Kind::Secondary, "Reset", !busy).clicked();
            cancel = theme::button(ui, theme::Kind::Secondary, "Cancel", true).clicked();
            if busy {
                ui.spinner();
            }
        });
        self.view_panel_note(ui);
        if cancel {
            self.views.picking = None;
            self.set_bench(Bench::Select);
            return;
        }
        if let Some(source) = copy_from.and_then(|id| self.views.book.get(&id).cloned()) {
            self.views.form.copy_display(&source);
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
        if save_camera && let Some(camera) = self.views.active_pane().map(|p| p.camera) {
            match camera.frame(self.view_origin()) {
                Ok((plane, span)) => {
                    self.views.form.plane = plane;
                    self.views.form.span = format!("{}", span.round().max(1.0));
                    self.views.form.rotation = ["0".into(), "0".into()];
                    submit = true;
                }
                Err(e) => self.view_note(Err(e.to_string())),
            }
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

    /// The last view note under the panel's buttons.
    fn view_panel_note(&self, ui: &mut egui::Ui) {
        if let Some(note) = &self.views.note {
            let palette = theme::Palette::current(ui);
            match note {
                Ok(t) => ui.label(t.as_str()),
                Err(t) => ui.colored_label(palette.error, t.as_str()),
            };
        }
    }

    /// The panel of the free 3D view: its name for "Save as view", and its render mode and
    /// grid guides, which change the active pane at once as the ribbon's Display group does.
    fn free_view_panel(&mut self, ui: &mut egui::Ui) {
        let Some((camera, mode, grids)) = self
            .views
            .active_pane()
            .map(|p| (p.camera, p.mode, p.grids))
        else {
            ui.label("No view is open.");
            return;
        };
        if self.views.active().is_some() {
            // The active pane switched to a saved view: the panel follows it.
            self.show_view_properties();
            return;
        }
        let busy = self.views.job.is_some();
        text(ui, "Name", &mut self.views.form.name);
        ui.separator();
        let mut chosen = mode;
        egui::ComboBox::from_id_salt("free-view-render-mode")
            .selected_text(chosen.as_token())
            .show_ui(ui, |ui| {
                for mode in RenderMode::ALL {
                    ui.selectable_value(&mut chosen, mode, mode.as_token());
                }
            });
        if chosen != mode {
            self.set_active_mode(chosen);
        }
        let mut shown = grids;
        if ui.checkbox(&mut shown, "Grid guides").changed() {
            self.set_grids_shown(shown);
            if !shown {
                self.grid_edit = None;
            }
        }
        let projection = if camera.is_orthographic() {
            Projection::Orthographic
        } else {
            Projection::Perspective
        };
        ui.label(format!(
            "Projection: {}",
            match projection {
                Projection::Orthographic => "orthographic",
                Projection::Perspective => "perspective",
            }
        ));
        ui.add_space(8.0);
        let save = theme::button(ui, theme::Kind::Primary, "Save as view", !busy)
            .on_hover_text("Keep the camera as it is now as a saved view")
            .clicked();
        self.view_panel_note(ui);
        if save {
            let origin = self.view_origin();
            let (mode, grids) = (self.active_mode(), self.grids_shown());
            let form = &mut self.views.form;
            form.id = None;
            form.construction = Construction::CurrentCamera;
            form.projection = projection;
            form.mode = mode;
            form.grids = grids;
            if let Ok((_, span)) = camera.frame(origin) {
                form.span = format!("{}", span.round().max(1.0));
            }
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
        let view = id.and_then(|id| self.views.book.get(&id)).cloned();
        let camera = match &view {
            Some(view) => self.views.cameras.get(&view.id).copied().map_or_else(
                || Camera::from_view(view, self.view_origin(), self.view_extent()),
                Ok,
            ),
            None => self
                .views
                .active_pane()
                .map_or_else(|| Camera::new([0.0; 3], 1000.0, 1.6), |p| Ok(p.camera)),
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
        let mut pane = ViewPane::new(view.as_ref(), camera, key);
        if let Some(area) = self.views.area {
            pane.rect = crate::panes::opening_rect(area, self.views.panes.len());
            pane.placed = true;
        }
        self.views.panes.push(pane);
        if let Some(view) = view
            && !self.views.open.contains(&view.id)
        {
            self.views.open.push(view.id);
        }
        self.pane_focused(from);
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
    /// that was active before (`Some(None)` the free 3D view, `None` no pane at all). A plane
    /// change cancels an unfinished view pick and the tool drafts, as `activate_view` does.
    fn pane_focused(&mut self, from: Option<Option<Uuid7>>) {
        let to = self.views.active_pane().map(|p| p.view);
        let plane_of = |views: &crate::views::Views, id: Option<Uuid7>| {
            id.and_then(|id| views.book.get(&id)).map(|v| v.plane)
        };
        let from_view = from.flatten();
        let to_view = to.flatten();
        let mut changed = false;
        match (from_view, to_view) {
            // Free to view: the free work plane is kept until the free view comes back.
            (None, Some(_)) => {
                if let Some(plane) = plane_of(&self.views, to_view) {
                    self.views_input = Some(self.tools.plane);
                    self.install_plane(Some(plane));
                    changed = true;
                }
            }
            // View to free (or to no pane): the free work plane comes back.
            (Some(_), None) => {
                if let Some(plane) = self.views_input.take() {
                    self.install_plane(plane);
                    changed = true;
                }
            }
            // View to view: only a different plane is installed.
            (Some(_), Some(_)) => {
                let next = plane_of(&self.views, to_view);
                if next.is_some() && next != plane_of(&self.views, from_view) {
                    self.install_plane(next);
                    changed = true;
                }
            }
            (None, None) => {}
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

/// What a browser click asks for; run after the browser releases its borrows.
pub(crate) enum BrowserAction {
    Click(Uuid7, bool, bool, Vec<Uuid7>),
    Activate(Uuid7),
    Window(Uuid7),
    Properties(Uuid7),
    Favorite(Uuid7),
    Duplicate(Uuid7),
    Clip(Uuid7, bool),
    Close(Uuid7),
    Delete(Uuid7),
    ConfirmDelete(Uuid7),
    DeleteFolder(String),
    OpenAll(Vec<Uuid7>),
    CloseAll(Vec<Uuid7>),
    Move(Uuid7, String),
}
