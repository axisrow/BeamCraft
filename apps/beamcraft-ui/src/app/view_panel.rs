// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The window side of the V12 model views: the browser, the view-properties window, view
//! construction by picking in the model, activation and the 3D/plane toggle, and the extra view
//! windows. Everything here is display state: the model is never written, and the engine admits
//! every view before it is kept.

use super::App;
use super::workbench::Bench;
use crate::camera::Camera;
use crate::views::{
    AxisBatch, Construction, Direction, Dock, Form, MAX_WINDOWS, Picking, SideChoice, ViewWindow,
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

    /// Shows `id` in the main viewport, or the free 3D view for `None`. The camera of a view
    /// opened before comes back as the user left it; its render mode, grid guides, hidden kinds
    /// and plane (the input frame) come from the view.
    pub(crate) fn activate_view(&mut self, id: Option<Uuid7>) {
        if self.views.active == id {
            return;
        }
        let input = self.tools.plane;
        match self.views.active {
            Some(current) => {
                self.views.cameras.insert(current, self.camera);
            }
            None => {
                self.views.spatial = Some((self.camera, self.mode, self.grids_visible));
                self.views_input = Some(input);
            }
        }
        self.views.toggled = None;
        match id.and_then(|id| self.views.book.get(&id).cloned()) {
            Some(view) => {
                let camera = match self.views.cameras.get(&view.id) {
                    Some(camera) => Ok(*camera),
                    None => Camera::from_view(&view, self.view_origin(), self.view_extent()),
                };
                match camera {
                    Ok(camera) => self.camera = camera,
                    Err(e) => {
                        self.view_note(Err(e.to_string()));
                        return;
                    }
                }
                self.mode = view.render_mode;
                self.grids_visible = view.grids_visible;
                self.views.active = Some(view.id);
                if !self.views.open.contains(&view.id) {
                    self.views.open.push(view.id);
                }
                self.install_plane(Some(view.plane));
                self.tool_note = None;
            }
            None => {
                self.views.active = None;
                if let Some((camera, mode, grids)) = self.views.spatial.take() {
                    self.camera = camera;
                    self.mode = mode;
                    self.grids_visible = grids;
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
        if let Some(previous) = self.views.toggled.take() {
            self.camera = previous;
            return;
        }
        let origin = self.view_origin();
        let plane = self
            .views
            .active_view()
            .map(|v| v.plane)
            .or(self.tools.plane)
            .unwrap_or_default();
        let span = self.camera.frame(origin).map_or(10_000.0, |(_, s)| s);
        match Camera::aligned_to(plane, origin, span, self.view_extent()) {
            Ok(aligned) => {
                self.views.toggled = Some(self.camera);
                self.camera = aligned;
            }
            Err(e) => self.view_note(Err(e.to_string())),
        }
    }

    /// One of the six directional cameras about the current target; the input plane stays.
    pub(crate) fn orient_view(&mut self, direction: Direction) {
        let origin = self.view_origin();
        let Ok((frame, span)) = self.camera.frame(origin) else {
            return;
        };
        let aligned = direction.plane(frame.origin()).and_then(|plane| {
            Camera::aligned_to(plane, origin, span, self.view_extent()).map_err(|e| e.to_string())
        });
        match aligned {
            Ok(aligned) => {
                if self.views.toggled.is_none() {
                    self.views.toggled = Some(self.camera);
                }
                self.camera = aligned;
            }
            Err(e) => self.view_note(Err(e)),
        }
    }

    /// Opens the properties window on a new view built by `construction`; the constructions
    /// that pick in the model start picking at once.
    pub(crate) fn begin_view(&mut self, construction: Construction) {
        self.set_bench(Bench::Select);
        let mut form = Form {
            construction,
            ..Form::default()
        };
        if let Ok((_, span)) = self.camera.frame(self.view_origin()) {
            form.span = format!("{}", span.round().max(1.0));
        }
        if construction == Construction::CurrentCamera {
            form.projection = if self.camera.is_orthographic() {
                Projection::Orthographic
            } else {
                Projection::Perspective
            };
            form.mode = self.mode;
            form.grids = self.grids_visible;
        }
        let taken = self.views.book.views().len() + 1;
        form.name = self.views.book.unique_name(&format!("View {taken}"), &[]);
        if matches!(construction, Construction::GridAxes | Construction::Level) {
            form.name = match construction {
                Construction::Level => "Plan".into(),
                _ => "Grid".into(),
            };
        }
        self.views.form = form;
        self.views.editing = true;
        self.views.picking = (construction.picks() > 0).then(|| Picking {
            construction,
            ..Picking::default()
        });
    }

    /// Opens the properties window on an existing view.
    pub(crate) fn edit_view(&mut self, id: Uuid7) {
        if let Some(view) = self.views.book.get(&id) {
            self.views.form = Form::from_view(view);
            self.views.editing = true;
            self.views.picking = None;
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

    /// One click while a view pick runs: a face, then points. The last pick builds the view.
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
                self.views.picking = None;
                self.submit_view();
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
        if self.views.active == Some(id) {
            self.views.active = None;
            self.views
                .spatial
                .get_or_insert((self.camera, self.mode, self.grids_visible));
            self.activate_view(Some(id));
        }
        for window in self.views.windows.iter_mut().filter(|w| w.view == Some(id)) {
            if let Some(view) = self.views.book.get(&id) {
                window.mode = view.render_mode;
                window.grids = view.grids_visible;
                view.name.clone_into(&mut window.title);
                if let Ok(camera) = Camera::from_view(
                    view,
                    self.loaded
                        .as_ref()
                        .map_or([0.0; 3], |l| l.summary.origin_mm),
                    1e6,
                ) {
                    window.camera = camera;
                }
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
                if let Some(picking) = &self.views.picking
                    && picking.done() < picking.construction.picks()
                {
                    return Err("Pick the points in the model first.".into());
                }
                single(form.plane)
            }
            Construction::WorkPlane => single(self.tools.plane.unwrap_or_default()),
            Construction::CurrentCamera => {
                let (plane, _) = self.camera.frame(origin).map_err(|e| e.to_string())?;
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
        self.views.editing = false;
        if let Some(first) = first {
            self.views.selected.clear();
            self.views.selected.insert(first);
            self.activate_view(Some(first));
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
                self.views.editing = false;
                self.view_note(Ok(format!(
                    "{} view(s) were created; the model is unchanged.",
                    added.len()
                )));
                if let Some(first) = added.first().copied() {
                    self.activate_view(Some(first));
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
                        self.open_view_window(Some(id));
                    }
                }
            });
        }
        ui.separator();
        if ui
            .selectable_label(self.views.active.is_none(), "{3D}")
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
                                || self.views.active == Some(view.id);
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
            BrowserAction::Window(id) => self.open_view_window(Some(id)),
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
            BrowserAction::Close(id) => {
                if self.views.active == Some(id) {
                    self.activate_view(None);
                }
                self.close_view(id);
            }
            BrowserAction::Delete(id) => self.views.confirm_delete = Some(id),
            BrowserAction::ConfirmDelete(id) => {
                self.views.confirm_delete = None;
                if self.views.active == Some(id) {
                    self.activate_view(None);
                }
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
                    self.open_view_window(Some(id));
                }
            }
            BrowserAction::CloseAll(ids) => {
                for id in ids {
                    if self.views.active == Some(id) {
                        self.activate_view(None);
                    }
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

    /// Closes a view in the session: its windows and its kept camera; the definition stays.
    fn close_view(&mut self, id: Uuid7) {
        for window in self.views.windows.iter_mut().filter(|w| w.view == Some(id)) {
            window.release(&self.state);
        }
        self.views.windows.retain(|w| w.view != Some(id));
        self.views.cameras.remove(&id);
        self.views.open.retain(|v| *v != id);
    }

    /// The preview of `view`: drawn off screen with the view's own camera, cut, tint and hidden
    /// kinds, kept until the snapshot or the view changes. At most one preview is drawn per
    /// frame (`budget`), and at most [`crate::views::MAX_PREVIEWS`] are kept.
    fn view_preview(&mut self, view: &ModelView, budget: &mut bool) -> Option<egui::TextureId> {
        let loaded = self.loaded.as_ref()?;
        let key = crate::views::preview_key(loaded.snapshot.token().as_str(), view);
        if let Some((drawn, window)) = self.views.previews.get(&view.id)
            && *drawn == key
        {
            return window.texture();
        }
        if !std::mem::take(budget) {
            return self
                .views
                .previews
                .get(&view.id)
                .and_then(|(_, w)| w.texture());
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
        let (_, window) = self
            .views
            .previews
            .entry(view.id)
            .or_insert_with(|| (0, ViewWindow::new(Some(view), camera, 0)));
        window.camera = camera;
        let [width, height] = crate::views::PREVIEW_SIZE;
        let aspect = window
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
        let mut camera_view = window.camera.view(aspect).ok()?;
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
        window.renderer()?.draw(
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
            .and_then(|(_, w)| w.texture())
    }

    /// The view-properties window: the name, folder and marks, how a new view is built, and
    /// every display property. Nothing changes until Create or Apply.
    pub(crate) fn view_properties_window(&mut self, ctx: &egui::Context) {
        if !self.views.editing {
            return;
        }
        let mut open = true;
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
        let title = if self.views.form.id.is_some() {
            "View properties"
        } else {
            "New view"
        };
        egui::Window::new(title)
            .id(egui::Id::unique("view-properties"))
            .open(&mut open)
            .default_width(320.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(420.0)
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
                                                for (i, family) in
                                                    ["U", "V"].into_iter().enumerate()
                                                {
                                                    if let (Some(pick), Some(prefix)) = (
                                                        form.families.get_mut(i),
                                                        form.prefixes.get_mut(i),
                                                    ) {
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
                                        ui.small(
                                            "Batch views are named by the prefix and the level.",
                                        );
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
                                        ui.small(
                                            "Select the part first; the views frame its geometry.",
                                        );
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
                                        .on_hover_text(
                                            "The view plane and height become the camera's",
                                        )
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
                                    .add_enabled(
                                        selection_bounds.is_some(),
                                        egui::Button::new("Selection"),
                                    )
                                    .clicked()
                                {
                                    area = selection_bounds;
                                }
                                if let Some(b) = area {
                                    form.area =
                                        beamcraft_engine::views::WorkArea::new(b.min, b.max).ok();
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
                ui.separator();
                ui.horizontal(|ui| {
                    let label = if self.views.form.id.is_some() {
                        "Apply"
                    } else {
                        "Create"
                    };
                    let waiting = self
                        .views
                        .picking
                        .as_ref()
                        .is_some_and(|p| p.done() < p.construction.picks());
                    submit = ui
                        .add_enabled(!busy && !waiting, egui::Button::new(label))
                        .clicked();
                    reset = self.views.form.id.is_some() && ui.button("Reset").clicked();
                    if ui.button("Close").clicked() {
                        self.views.editing = false;
                        self.views.picking = None;
                    }
                    if busy {
                        ui.spinner();
                    }
                });
                if let Some(note) = &self.views.note {
                    let palette = crate::theme::Palette::current(ui);
                    match note {
                        Ok(t) => ui.label(t.as_str()),
                        Err(t) => ui.colored_label(palette.error, t.as_str()),
                    };
                }
            });
        if !open {
            self.views.editing = false;
            self.views.picking = None;
        }
        if let Some(source) = copy_from.and_then(|id| self.views.book.get(&id).cloned()) {
            self.views.form.copy_display(&source);
        }
        if restart_pick {
            let construction = self.views.form.construction;
            self.views.picking = Some(Picking {
                construction,
                ..Picking::default()
            });
        }
        if reset && let Some(id) = self.views.form.id {
            self.edit_view(id);
        }
        if save_camera {
            match self.camera.frame(self.view_origin()) {
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

    /// Opens an extra window on `view` (or the free 3D view), up to [`MAX_WINDOWS`].
    pub(crate) fn open_view_window(&mut self, id: Option<Uuid7>) {
        if self.views.windows.len() >= MAX_WINDOWS {
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
            None => Ok(self.camera),
        };
        match camera {
            Ok(camera) => {
                let index = self.views.windows.len();
                self.views.window_serial += 1;
                let mut window = ViewWindow::new(view.as_ref(), camera, index);
                window.key = self.views.window_serial;
                self.views.windows.push(window);
                if let Some(view) = view
                    && !self.views.open.contains(&view.id)
                {
                    self.views.open.push(view.id);
                }
            }
            Err(e) => self.view_note(Err(e.to_string())),
        }
    }

    /// Draws the extra windows: the loaded scene through each window's own camera, with its
    /// view's cut, tint, render mode, grid guides and hidden kinds. They navigate like the main
    /// viewport; selection and tools stay in the main viewport.
    pub(crate) fn draw_view_windows(&mut self, ctx: &egui::Context) {
        let content = ctx.content_rect();
        if std::mem::take(&mut self.views.tile) {
            let count = self.views.windows.len();
            for (index, window) in self.views.windows.iter_mut().enumerate() {
                window.dock = Dock::Float;
                window.place = Some(crate::views::tile_rect(content, index, count));
            }
        }
        let mut released = Vec::new();
        let mut activate = None;
        for index in 0..self.views.windows.len() {
            let Some(window) = self.views.windows.get_mut(index) else {
                continue;
            };
            let view = window.view.and_then(|id| self.views.book.get(&id)).cloned();
            let mut open = true;
            let mut builder = egui::Window::new(window.title.clone())
                .id(egui::Id::unique(("view-window", window.key)))
                .open(&mut open)
                .default_size([480.0, 320.0]);
            if let Some(rect) = crate::views::dock_rect(content, window.dock) {
                builder = builder.fixed_pos(rect.min).fixed_size(rect.size());
            } else if let Some(rect) = window.place.take() {
                builder = builder.current_pos(rect.min).default_size(rect.size());
            }
            builder.show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.small_button("Fit").clicked()
                        && let Some(l) = &self.loaded
                    {
                        let (target, extent) = self.grid_scene.framing(&l.summary);
                        let _ = window.camera.fit(target, extent, 1.5);
                    }
                    if window.view.is_some() && ui.small_button("Show in main view").clicked() {
                        activate = window.view;
                    }
                    ui.menu_button("Dock", |ui| {
                        for (dock, label) in [
                            (Dock::Float, "Float"),
                            (Dock::Left, "Left"),
                            (Dock::Right, "Right"),
                            (Dock::Top, "Top"),
                            (Dock::Bottom, "Bottom"),
                        ] {
                            if ui.selectable_label(window.dock == dock, label).clicked() {
                                window.dock = dock;
                                ui.close();
                            }
                        }
                    });
                    egui::ComboBox::from_id_salt(("window-mode", window.key))
                        .selected_text(window.mode.as_token())
                        .show_ui(ui, |ui| {
                            for mode in RenderMode::ALL {
                                ui.selectable_value(&mut window.mode, mode, mode.as_token());
                            }
                        });
                    ui.checkbox(&mut window.grids, "Grids");
                });
                let size = ui.available_size();
                let scale = ui.ctx().pixels_per_point();
                let [width, height] = super::target_size([size.x, size.y], scale);
                let Ok(aspect) =
                    window.resize(&self.state, width, height, self.source.samples, &self.stats)
                else {
                    return;
                };
                let Some(loaded) = &self.loaded else {
                    return;
                };
                // The selected-only mode belongs to the main viewport's selection.
                let mode = if window.mode == RenderMode::SelectedOnly {
                    RenderMode::Shaded
                } else {
                    window.mode
                };
                let hidden = view
                    .as_ref()
                    .map_or(beamcraft_engine::KindFilter::SHOW_ALL, |v| v.hidden_kinds);
                let token = loaded.snapshot.token().as_str().to_owned();
                let stale = window
                    .filtered
                    .as_ref()
                    .is_none_or(|(t, h, _)| *t != token || *h != hidden);
                if hidden != self.views.shown && stale {
                    let overlay = self.colour.as_ref().map(|(_, o)| std::sync::Arc::clone(o));
                    let built = crate::scene::Scene::from_snapshot_showing(
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
                    });
                    match built {
                        Ok(uploaded) => window.filtered = Some((token, hidden, uploaded)),
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                let uploaded = match &window.filtered {
                    Some((_, h, uploaded)) if hidden != self.views.shown && *h == hidden => {
                        uploaded
                    }
                    _ => &loaded.uploaded,
                };
                let Ok(mut camera_view) = window.camera.view(aspect) else {
                    return;
                };
                camera_view.tint = crate::views::tint_of(view.as_ref());
                if let (Some(renderer), Some(texture)) = (window.renderer(), window.texture()) {
                    let clip = match crate::views::display_clip(self.cut.as_ref(), view.as_ref()) {
                        Some(cut) => crate::cut_plane::ClipGpu::of(&cut, loaded.summary.origin_mm),
                        None => crate::cut_plane::ClipGpu::none(),
                    };
                    renderer.draw(
                        &self.state.device,
                        &self.state.queue,
                        uploaded,
                        camera_view,
                        &self.stats,
                        mode,
                        &clip,
                    );
                    let response =
                        ui.add(egui::Image::new((texture, size)).sense(super::viewport_sense()));
                    if window.grids {
                        self.grid_scene.paint(
                            ui,
                            response.rect,
                            &camera_view,
                            loaded.summary.origin_mm,
                            None,
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
                    let (orbit, pan, zoom) = super::navigation(
                        self.preset,
                        delta,
                        response.dragged_by(egui::PointerButton::Primary),
                        response.dragged_by(egui::PointerButton::Secondary),
                        response.dragged_by(egui::PointerButton::Middle),
                        ctrl,
                        shift,
                        window.camera.distance(),
                        hovered,
                        scroll_y,
                        pinch,
                        false,
                        false,
                        self.invert_zoom,
                    );
                    if let Err(e) = window.camera.navigate(orbit, pan, zoom) {
                        self.error = Some(e.to_string());
                    }
                }
            });
            window.open &= open;
            if !window.open {
                released.push(index);
            }
        }
        for index in released.into_iter().rev() {
            if index < self.views.windows.len() {
                let mut window = self.views.windows.remove(index);
                if let Some(id) = window.view {
                    self.views.cameras.insert(id, window.camera);
                }
                window.release(&self.state);
            }
        }
        if let Some(id) = activate {
            self.activate_view(Some(id));
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
