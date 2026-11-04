// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V18 slice 1: contour grips on the selected flat plate. The selected plate's authored
//! contour ([`PlateContour`], read from the same `inspect-elements` reply as the rest of the
//! panel) draws a vertex grip on every vertex and a grip on the middle of every edge; dragging
//! a vertex grip moves that vertex, dragging an edge grip inserts a sharp vertex there, and an
//! Alt-click on a vertex grip deletes it. Every grip commits the same A12.3 command a user
//! types ([`ContourEdit`]), so the drag is one CLI command, one revision and one undo unit, and
//! the saved result is the CLI's by construction.
//!
//! The preview during a drag is the same point arithmetic the CLI edit performs; the commit
//! happens once, on release. A drag that never leaves the grip (a confirming click) commits
//! nothing. Corner treatments are not drawn or edited here; the corner editor is a later slice.

use super::{App, field, workbench};
use crate::command;
use crate::edit::{ContourEdit, PlateContour, Ray, ndc_of, ray_through};
use crate::theme::{BRAND, BRAND_LIGHT, TEAL};
use beamcraft_engine::solid::rotate_vector;
use beamcraft_model::limits::COORDINATE_CAP_MM;
#[cfg(test)]
use egui::Color32;
use egui::{Pos2, Rect, Response, Stroke, Vec2};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use steel::model::command::CornerTreatment;

/// A grip the pointer grabbed: the vertex or edge it belongs to and the in-plane point the
/// drag currently names. The point is what the release commits.
#[derive(Debug, Clone, PartialEq)]
struct Drag {
    /// The grip the drag began on.
    grip: Grip,
    /// The drag's in-plane point, plate-local millimetres.
    to: [f64; 2],
}

/// A press over a grip that has not yet become a drag: a confirming click commits nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pending {
    /// The grip under the press.
    grip: Grip,
    /// Where the press landed.
    at: Pos2,
}

/// Which grip of the contour the pointer works on: a vertex, or an edge by its middle.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Grip {
    /// The contour vertex at this position.
    Vertex(usize),
    /// The edge at this position (it joins vertices `i` and `i + 1`).
    Edge(usize),
}

/// What the corner editor authors on one vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CornerMode {
    /// No treatment: the corner is sharp again.
    Sharp,
    /// `chamfer:D1[,D2]`: the corner is replaced by a straight cut.
    Chamfer,
    /// `round:R`: the corner is replaced by a tangent arc.
    Round,
}

/// The corner editor open on one vertex (V18 slice 2): the element it was opened on, the
/// vertex, the chosen kind, the typed parameters and the reason Apply is currently refused.
/// Prefilled from the corner's present treatment. The element id is part of the state: the
/// commit always targets the current selection, so an editor that outlived its selection or an
/// accepted edit that shifted the vertex indices would author on the wrong plate or vertex.
#[derive(Debug, Clone, PartialEq)]
struct CornerEditor {
    /// The id of the selection the editor was opened on.
    element: String,
    /// The position of the vertex in the authored contour.
    vertex: usize,
    /// The kind the form edits.
    mode: CornerMode,
    /// The chamfer cut along the previous edge, as typed.
    d1: String,
    /// The chamfer cut along the next edge, as typed; empty means "same as the first".
    d2: String,
    /// The arc radius, as typed.
    radius: String,
    /// Why Apply is refused, shown under the fields.
    error: Option<String>,
}

/// Opens the corner editor on `vertex` of the plate `element`, prefilled with the treatment
/// the contour already authors there, if any.
fn corner_editor(element: &str, contour: &PlateContour, vertex: usize) -> CornerEditor {
    let (mode, d1, d2, radius) = match contour.corner_of(vertex) {
        Some(CornerTreatment::Chamfer { d1_mm, d2_mm }) => (
            CornerMode::Chamfer,
            format!("{d1_mm:?}"),
            if d1_mm != d2_mm {
                format!("{d2_mm:?}")
            } else {
                String::new()
            },
            String::new(),
        ),
        Some(CornerTreatment::Round { radius_mm }) => (
            CornerMode::Round,
            String::new(),
            String::new(),
            format!("{radius_mm:?}"),
        ),
        None => (
            CornerMode::Sharp,
            String::new(),
            String::new(),
            String::new(),
        ),
    };
    CornerEditor {
        element: element.to_owned(),
        vertex,
        mode,
        d1,
        d2,
        radius,
        error: None,
    }
}

/// The treatment the editor's fields name, checked the way the CLI will check them: numbers
/// first (`enter a number`), then positive and inside the coordinate cap. `None` is a sharp
/// corner, which has nothing to check.
fn editor_treatment(editor: &CornerEditor) -> Result<Option<CornerTreatment>, String> {
    let number = |field: &str, text: &str| -> Result<f64, String> {
        let value = text
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("{field} is not a number"))?;
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("{field} must be positive"));
        }
        if value > COORDINATE_CAP_MM {
            return Err(format!("{field} exceeds the coordinate cap"));
        }
        Ok(value)
    };
    match editor.mode {
        CornerMode::Sharp => Ok(None),
        CornerMode::Chamfer => {
            let d1 = number("D1", &editor.d1)?;
            let d2 = if editor.d2.trim().is_empty() {
                d1
            } else {
                number("D2", &editor.d2)?
            };
            Ok(Some(CornerTreatment::Chamfer {
                d1_mm: d1,
                d2_mm: d2,
            }))
        }
        CornerMode::Round => Ok(Some(CornerTreatment::Round {
            radius_mm: number("Radius", &editor.radius)?,
        })),
    }
}

/// A screen hit radius in points: a press within this distance of a grip takes the grip.
const GRIP_RADIUS: f32 = 8.0;

/// A press becomes a drag once the pointer moves this far, so a confirming click never runs a
/// command.
const PROMOTE_DISTANCE: f32 = 4.0;

/// The state of the contour grips: the drag in progress, the corner editor and the commit in
/// flight.
#[derive(Default)]
pub(crate) struct Grips {
    /// The active grip drag, if any.
    drag: Option<Drag>,
    /// A press over a grip that has not yet been promoted to a drag.
    pending: Option<Pending>,
    /// The corner editor open on one vertex, if any.
    corner: Option<CornerEditor>,
    /// The contour edit running on a worker thread.
    job: Option<
        Receiver<(
            ContourEdit,
            Result<command::Output, crate::tools::ToolError>,
        )>,
    >,
}

impl Grips {
    /// Whether the grips hold the pointer right now: a promoted drag or a press over a grip.
    /// While they do, the viewport takes no camera navigation from the primary drag and no
    /// selection from the click.
    pub(crate) fn holds_pointer(&self) -> bool {
        self.drag.is_some() || self.pending.is_some()
    }

    /// Whether a contour edit command is running on a worker: part of the window's
    /// single-flight rule, so no other write starts while one is in the air, and no second
    /// spawn orphans this job's receiver.
    pub(crate) fn running(&self) -> bool {
        self.job.is_some()
    }
}

/// The grips of one contour in screen points: every vertex as `(position, point)` and every
/// edge middle as `(position, point)`. A vertex whose projection is behind the camera or not
/// finite gets no grip; an edge needs both of its ends projected.
type ScreenGrips = (Vec<(usize, Pos2)>, Vec<(usize, Pos2)>);

/// The contour's grips in screen points. A vertex whose projection is behind the camera or not
/// finite gets no grip; an edge needs both of its ends projected.
fn grips_screen(
    contour: &PlateContour,
    view: &crate::camera::View,
    rect: Rect,
    origin: [f64; 3],
) -> ScreenGrips {
    let placed = contour.placed_points();
    let count = placed.len();
    let projected: Vec<Option<Pos2>> = placed
        .iter()
        .map(|p| project(p, origin, view, rect))
        .collect();
    let mut edges = Vec::new();
    for (i, a) in projected.iter().enumerate() {
        if let (Some(a), Some(Some(b))) = (a, projected.get((i + 1) % count)) {
            edges.push((i, Pos2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)));
        }
    }
    let vertices = projected
        .into_iter()
        .enumerate()
        .filter_map(|(i, p)| p.map(|p| (i, p)))
        .collect();
    (vertices, edges)
}

/// The screen point of a world-millimetre point in the pane, or `None` when it is behind the
/// camera or not finite. The same projection the selection outline uses.
fn project(
    point: &[f64; 3],
    origin: [f64; 3],
    view: &crate::camera::View,
    rect: Rect,
) -> Option<Pos2> {
    let local = std::array::from_fn(|i| {
        (point.get(i).copied().unwrap_or(0.0) - origin.get(i).copied().unwrap_or(0.0)) as f32
    });
    let [x, y, z] = view.project(local);
    ((0.0..1.0).contains(&z) && x.is_finite() && y.is_finite()).then(|| {
        Pos2::new(
            rect.min.x + (x * 0.5 + 0.5) * rect.width(),
            rect.min.y + (0.5 - y * 0.5) * rect.height(),
        )
    })
}

/// The grip under `pos`, if any: the nearest grip inside the radius wins, and a vertex grip
/// wins over an edge middle at the same spot, so a short edge's grips never fight.
fn grip_at(vertices: &[(usize, Pos2)], edges: &[(usize, Pos2)], pos: Pos2) -> Option<Grip> {
    let nearest = |points: &[(usize, Pos2)]| {
        points
            .iter()
            .filter(|(_, p)| p.distance(pos) <= GRIP_RADIUS)
            .min_by(|a, b| a.1.distance(pos).total_cmp(&b.1.distance(pos)))
            .map(|(i, _)| *i)
    };
    if let Some(vertex) = nearest(vertices) {
        return Some(Grip::Vertex(vertex));
    }
    nearest(edges).map(Grip::Edge)
}

/// Where `ray` crosses the plate's plane (the placement translation with the placement's local
/// `z` axis as its normal), or `None` when the ray is parallel to it, crosses it behind the
/// camera or lands outside the coordinate cap.
fn plane_point(contour: &PlateContour, ray: &Ray) -> Option<[f64; 3]> {
    let normal = rotate_vector([0.0, 0.0, 1.0], contour.orientation_xyzw);
    let [nx, ny, nz] = normal;
    let [dx, dy, dz] = ray.direction;
    let [ox, oy, oz] = ray.origin;
    let [tx, ty, tz] = contour.translation_mm;
    let denom = nx * dx + ny * dy + nz * dz;
    if !denom.is_finite() || denom.abs() < 1e-9 {
        return None;
    }
    let t = ((tx - ox) * nx + (ty - oy) * ny + (tz - oz) * nz) / denom;
    if !t.is_finite() || t <= 0.0 {
        return None;
    }
    let point = [ox + t * dx, oy + t * dy, oz + t * dz];
    (point
        .iter()
        .all(|v| v.is_finite() && v.abs() <= COORDINATE_CAP_MM))
    .then_some(point)
}

/// The in-plane point the pointer names on the plate plane: pixel to ray to plane to the
/// plate-local `x`, `y`. `None` when any step refuses; the drag keeps its last valid point.
fn drag_point(
    contour: &PlateContour,
    view: &crate::camera::View,
    rect: Rect,
    origin: [f64; 3],
    pos: Pos2,
) -> Option<[f64; 2]> {
    let ndc = ndc_of(
        [pos.x, pos.y],
        [rect.min.x, rect.min.y],
        [rect.width(), rect.height()],
    )?;
    let ray = ray_through(view, ndc, origin)?;
    let world = plane_point(contour, &ray)?;
    contour.local_point(world)
}

/// The contour a drag preview shows: the authored points with the drag's edit applied, the same
/// arithmetic the CLI edit runs. The corner list is not part of this slice and stays untouched.
fn preview_points(contour: &PlateContour, drag: &Drag) -> PlateContour {
    let mut preview = contour.clone();
    let count = preview.points.len();
    match drag.grip {
        Grip::Vertex(i) => {
            if let Some(point) = preview.points.get_mut(i) {
                point[0] = drag.to[0];
                point[1] = drag.to[1];
            }
        }
        Grip::Edge(i) => {
            let z = preview.points.get(i).map_or(0.0, |p| p[2]);
            preview
                .points
                .insert((i + 1).min(count), [drag.to[0], drag.to[1], z]);
        }
    }
    preview
}

/// Draws the selected plate's contour grips: vertex squares, edge-middle dots and, while a drag
/// is live, the preview contour that the release will commit. A preview with any vertex behind
/// the camera is skipped: a partial outline would suggest a shape the release will not commit.
pub(crate) fn paint(
    painter: &egui::Painter,
    rect: Rect,
    view: &crate::camera::View,
    origin: [f64; 3],
    contour: &PlateContour,
    grips: &Grips,
) {
    let (vertices, edges) = grips_screen(contour, view, rect, origin);
    if let Some(drag) = &grips.drag {
        let points = preview_points(contour, drag).placed_points();
        let mut screen = points
            .iter()
            .filter_map(|p| project(p, origin, view, rect))
            .collect::<Vec<_>>();
        if screen.len() == points.len()
            && screen.len() >= 3
            && let Some(first) = screen.first().copied()
        {
            screen.push(first);
            painter.add(egui::Shape::line(screen, Stroke::new(2.0, BRAND_LIGHT)));
        }
    }
    for (_, pos) in &edges {
        painter.circle_filled(*pos, 3.5, TEAL);
    }
    let active = grips.drag.as_ref().map(|d| d.grip);
    for (i, pos) in &vertices {
        let half = if active == Some(Grip::Vertex(*i)) {
            5.0
        } else {
            4.0
        };
        let rect = Rect::from_center_size(*pos, Vec2::splat(half * 2.0));
        if contour.corner_of(*i).is_some() {
            // A treated corner draws hollow: the authored vertex is editor-only, the evaluated
            // body shows the chamfer or the arc itself.
            painter.rect_stroke(
                rect,
                1.0,
                Stroke::new(2.0, BRAND_LIGHT),
                egui::StrokeKind::Outside,
            );
        } else {
            painter.rect_filled(rect, 1.0, BRAND);
        }
    }
}

/// What a grip draws as: the vertex colour and the edge colour, in that order.
#[cfg(test)]
fn grip_colours() -> [Color32; 2] {
    [BRAND, TEAL]
}

impl App {
    /// Feeds one frame of the pane's pointer to the contour grips. Returns true when the grips
    /// consumed the input: while a promoted drag runs nothing else may take the click, and the
    /// frame that releases a grip commits its edit. Grips stay dormant while a creation tool is
    /// armed, a view pick is in progress, or another bench than Select owns the clicks (the
    /// weld pair pick and the annotation placement would lose them); a selection without a
    /// plate contour resets them.
    pub(crate) fn contour_input(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        view: &crate::camera::View,
    ) -> bool {
        let dormant = self.beam_mode
            || self.plate_mode
            || self.views.picking.is_some()
            || self.bench != workbench::Bench::Select;
        let Some(contour) = self
            .editor
            .selected()
            .and_then(|s| s.contour.clone())
            .filter(|_| !dormant)
        else {
            self.contour.drag = None;
            self.contour.pending = None;
            self.contour.corner = None;
            return false;
        };
        let origin = self
            .loaded
            .as_ref()
            .map_or([0.0; 3], |l| l.summary.origin_mm);
        let rect = response.rect;
        let (esc, alt) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.modifiers.alt));
        let pointer = response.interact_pointer_pos();
        let (vertices, edges) = grips_screen(&contour, view, rect, origin);

        // Esc also closes the corner editor; the pane keeps its click.
        if esc {
            self.contour.corner = None;
        }

        // A live drag: steer it, finish it, or drop it on Esc.
        if let Some(drag) = self.contour.drag.clone() {
            if esc {
                self.contour.drag = None;
                return true;
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                let edit = match drag.grip {
                    Grip::Vertex(i) => ContourEdit::MoveVertex {
                        vertex: i,
                        to: drag.to,
                    },
                    Grip::Edge(i) => ContourEdit::InsertVertex {
                        edge: i,
                        at: drag.to,
                    },
                };
                self.contour.drag = None;
                self.start_contour_commit(edit);
                return true;
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(pos) = pointer
                && let Some(to) = drag_point(&contour, view, rect, origin, pos)
                && let Some(live) = self.contour.drag.as_mut()
            {
                live.to = to;
            }
            return true;
        }

        // A press waiting to become a drag: promote on movement, and a confirming click on a
        // vertex grip opens the corner editor, while one on an edge falls through.
        if let Some(pending) = self.contour.pending {
            if esc {
                self.contour.pending = None;
                return false;
            }
            let moved = pointer.is_some_and(|pos| pos.distance(pending.at) >= PROMOTE_DISTANCE);
            if moved && let Some(pos) = pointer {
                self.contour.pending = None;
                if let Some(to) = drag_point(&contour, view, rect, origin, pos) {
                    self.contour.drag = Some(Drag {
                        grip: pending.grip,
                        to,
                    });
                    return true;
                }
                // The pointer named no in-plane point (a view parallel to the plate): no drag,
                // no commit.
                return false;
            }
            if !response.dragged_by(egui::PointerButton::Primary) {
                self.contour.pending = None;
                if let Grip::Vertex(i) = pending.grip
                    && let Some(selected) = self.editor.selected()
                {
                    self.contour.corner = Some(corner_editor(&selected.id, &contour, i));
                }
            }
            return false;
        }

        // A fresh press over a grip: Alt deletes the vertex at once, otherwise the press waits
        // for movement before it becomes a drag.
        let Some(pos) = pointer else {
            return false;
        };
        if response.drag_started_by(egui::PointerButton::Primary) {
            match grip_at(&vertices, &edges, pos) {
                Some(Grip::Vertex(i)) if alt => {
                    self.start_contour_commit(ContourEdit::DeleteVertex { vertex: i });
                    return true;
                }
                Some(grip) => {
                    self.contour.pending = Some(Pending { grip, at: pos });
                    return false;
                }
                None => return false,
            }
        }
        false
    }

    /// Runs one contour edit through the same CLI command a user types, on a worker thread.
    /// One command is one revision, so a released drag is exactly one undo unit.
    pub(crate) fn start_contour_commit(&mut self, edit: ContourEdit) {
        if self.other_command_running() {
            self.edit_note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let Some(selected) = self.editor.selected() else {
            return;
        };
        let args = edit.args(&self.source.store, &selected.id);
        let cli = self.source.cli.clone();
        let (tx, rx) = channel();
        self.contour.job = Some(rx);
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-contour".into())
            .spawn(move || {
                let output = command::run(&cli, &args, command::DEADLINE, &stats);
                let _ = tx.send((edit, output));
            });
        if spawned.is_err() {
            self.contour.job = None;
            self.edit_note = Some(Err("the contour edit thread could not be started".into()));
        }
    }

    /// Draws the corner editor's window for the vertex whose grip was clicked (V18 slice 2).
    /// The form shows the present treatment prefilled; Apply commits the same
    /// `set-plate-corner` a user types, `sharp` included, and the window closes itself. The
    /// fields are checked here first (numbers, positive, inside the cap); the CLI's own refusal
    /// still reaches the status line if the kernel disagrees. The editor is bound to the
    /// selection it was opened on: the commit targets the current selection, so a different
    /// selection closes the editor instead of authoring on the wrong plate.
    pub(crate) fn contour_editor_ui(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.contour.corner.clone() else {
            return;
        };
        if self.editor.selected().map(|s| s.id.as_str()) != Some(editor.element.as_str()) {
            self.contour.corner = None;
            return;
        }
        let mut open = true;
        let mut apply: Option<Option<CornerTreatment>> = None;
        egui::Window::new(format!("Corner {}", editor.vertex))
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut editor.mode, CornerMode::Sharp, "sharp");
                    ui.radio_value(&mut editor.mode, CornerMode::Chamfer, "chamfer");
                    ui.radio_value(&mut editor.mode, CornerMode::Round, "round");
                });
                match editor.mode {
                    CornerMode::Sharp => {
                        ui.label("No treatment: the corner keeps its authored point.");
                    }
                    CornerMode::Chamfer => {
                        field(ui, "D1", &mut editor.d1, 64.0);
                        field(ui, "D2", &mut editor.d2, 64.0);
                        ui.label("D2 empty cuts the same distance on both edges.");
                    }
                    CornerMode::Round => {
                        field(ui, "R", &mut editor.radius, 64.0);
                    }
                }
                editor.error = editor_treatment(&editor).err();
                if let Some(message) = &editor.error {
                    ui.colored_label(egui::Color32::RED, message);
                }
                if ui
                    .add_enabled(editor.error.is_none(), egui::Button::new("Apply"))
                    .clicked()
                {
                    apply = editor_treatment(&editor).ok();
                }
            });
        if !open {
            self.contour.corner = None;
            return;
        }
        if let Some(treatment) = apply {
            let vertex = editor.vertex;
            self.contour.corner = None;
            self.start_contour_commit(ContourEdit::SetCorner { vertex, treatment });
            return;
        }
        self.contour.corner = Some(editor);
    }

    /// Reads a finished contour edit: the accepted revision reloads the scene and the selection;
    /// a refusal reaches the status line with the engine's code, and an unknown outcome still
    /// reloads, because the command may have committed before it was stopped.
    pub(crate) fn poll_contour(&mut self) {
        let Some(rx) = &self.contour.job else {
            return;
        };
        let received = match rx.try_recv() {
            Ok(received) => received,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                // The thread died before sending: nothing was committed by it, so nothing is
                // reloaded; the note says the run failed.
                (
                    ContourEdit::DeleteVertex { vertex: usize::MAX },
                    Err(crate::tools::ToolError::local(
                        "CLI_FAILED",
                        "the contour edit thread ended without a result",
                    )),
                )
            }
        };
        self.contour.job = None;
        let (edit, output) = received;
        let parsed =
            output.and_then(|o| crate::edit::parse_contour_edited(&edit, o.code, &o.stdout));
        match parsed {
            Ok(_) => {
                // An accepted edit may have shifted the vertex indices (an insert or a
                // delete): an open corner editor would name the wrong vertex after this.
                self.contour.corner = None;
                self.invalidate_select();
                self.reload();
                self.edit_note = Some(Ok(format!("{}.", edit.done())));
                self.refresh_selection();
                self.refresh_status();
            }
            Err(e) => {
                if e.code == "OUTCOME_UNKNOWN" {
                    // The command may have committed before it was stopped: the same index
                    // hazard as an accepted edit.
                    self.contour.corner = None;
                    self.invalidate_select();
                    self.reload();
                    self.refresh_selection();
                    self.refresh_status();
                }
                self.edit_note = Some(Err(e.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use std::path::Path;

    /// A unit-placement square, plate-local millimetres.
    fn square() -> PlateContour {
        PlateContour {
            points: vec![
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 100.0, 0.0],
                [0.0, 100.0, 0.0],
            ],
            corners: Vec::new(),
            translation_mm: [10.0, 20.0, 30.0],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn preview_moves_the_dragged_vertex_and_keeps_the_rest() {
        let contour = square();
        let preview = preview_points(
            &contour,
            &Drag {
                grip: Grip::Vertex(1),
                to: [40.0, 5.0],
            },
        );
        assert_eq!(preview.points.len(), 4);
        assert_eq!(preview.points.get(1), Some(&[40.0, 5.0, 0.0]));
        assert_eq!(preview.points.first(), Some(&[0.0, 0.0, 0.0]));
    }

    #[test]
    fn preview_inserts_a_vertex_after_the_dragged_edge() {
        let contour = square();
        let preview = preview_points(
            &contour,
            &Drag {
                grip: Grip::Edge(2),
                to: [50.0, 100.0],
            },
        );
        assert_eq!(preview.points.len(), 5);
        assert_eq!(preview.points.get(3), Some(&[50.0, 100.0, 0.0]));
    }

    #[test]
    fn plane_point_crosses_only_ahead_of_the_camera() {
        let contour = square();
        let straight = Ray {
            origin: [10.0, 20.0, 100.0],
            direction: [0.0, 0.0, -1.0],
        };
        assert_eq!(plane_point(&contour, &straight), Some([10.0, 20.0, 30.0]));
        let parallel = Ray {
            origin: [0.0, 0.0, 100.0],
            direction: [1.0, 0.0, 0.0],
        };
        assert_eq!(plane_point(&contour, &parallel), None);
        let behind = Ray {
            origin: [10.0, 20.0, -100.0],
            direction: [0.0, 0.0, -1.0],
        };
        assert_eq!(plane_point(&contour, &behind), None);
    }

    #[test]
    fn a_vertex_grip_wins_over_an_edge_middle_at_the_same_spot() {
        let spot = Pos2::new(100.0, 100.0);
        let vertices = vec![(2, spot)];
        let edges = vec![(1, spot)];
        assert_eq!(grip_at(&vertices, &edges, spot), Some(Grip::Vertex(2)));
        let only_edge = (vec![], edges.clone());
        assert_eq!(
            grip_at(&only_edge.0, &only_edge.1, spot),
            Some(Grip::Edge(1))
        );
        let far = Pos2::new(100.0 + GRIP_RADIUS + 1.0, 100.0);
        assert_eq!(grip_at(&vertices, &edges, far), None);
    }

    #[test]
    fn every_grip_of_a_placed_contour_projects_inside_the_pane() {
        let mut camera = Camera::new([300.0, 300.0, 3000.0], 4000.0, 1.6).unwrap();
        camera.navigate([0.0, 0.0], [0.0, 0.0], 0.0).unwrap();
        let view = camera.view(1.6).unwrap();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(800.0));
        let (vertices, edges) = grips_screen(&square(), &view, rect, [0.0; 3]);
        assert_eq!(vertices.len(), 4, "every vertex gets a grip");
        assert_eq!(edges.len(), 4, "every edge gets a middle grip");
        for (i, pos) in &vertices {
            assert!(
                rect.contains(*pos),
                "vertex {i} projects outside the pane at {pos:?}"
            );
        }
    }

    #[test]
    fn local_point_round_trips_through_a_quarter_turn() {
        let mut contour = square();
        let half_root_two = std::f64::consts::FRAC_1_SQRT_2;
        contour.orientation_xyzw = [0.0, 0.0, half_root_two, half_root_two];
        let placed = contour.placed_points();
        for (authored, world) in contour.points.iter().zip(placed.iter()) {
            let local = contour
                .local_point(*world)
                .expect("a placed vertex maps back into the plane");
            for (axis, value) in local.iter().zip(authored.iter()) {
                assert!(
                    (axis - value).abs() < 1e-6,
                    "the local point of the placed vertex is the authored one: {local:?} vs {authored:?}"
                );
            }
        }
    }

    #[test]
    fn the_grips_draw_in_their_own_colours() {
        let [vertex, edge] = grip_colours();
        assert_eq!(vertex, BRAND);
        assert_eq!(edge, TEAL);
    }

    #[test]
    fn args_match_what_a_user_types() {
        let store = Path::new("/tmp/store");
        let move_edit = ContourEdit::MoveVertex {
            vertex: 2,
            to: [1.0, -2.0],
        };
        assert_eq!(
            move_edit.args(store, "id-1"),
            vec![
                "move-plate-vertex",
                "--store",
                "/tmp/store",
                "--id",
                "id-1",
                "--vertex",
                "2",
                "--to",
                "1.0,-2.0"
            ]
        );
        let insert = ContourEdit::InsertVertex {
            edge: 0,
            at: [3.5, 4.0],
        };
        assert_eq!(
            insert.args(store, "id-1")[..2],
            ["insert-plate-vertex", "--store"]
        );
        let delete = ContourEdit::DeleteVertex { vertex: 1 };
        assert_eq!(
            delete.args(store, "id-1")[..2],
            ["delete-plate-vertex", "--store"]
        );
        let sharp = ContourEdit::SetCorner {
            vertex: 3,
            treatment: None,
        };
        let args = sharp.args(store, "id-1");
        assert_eq!(args.first(), Some(&"set-plate-corner".to_owned()));
        assert_eq!(args.last(), Some(&"sharp".to_owned()));
        let round = ContourEdit::SetCorner {
            vertex: 0,
            treatment: Some(CornerTreatment::Round { radius_mm: 25.0 }),
        };
        assert_eq!(
            round.args(store, "id-1").last(),
            Some(&"round:25.0".to_owned())
        );
        let chamfer = ContourEdit::SetCorner {
            vertex: 0,
            treatment: Some(CornerTreatment::Chamfer {
                d1_mm: 100.0,
                d2_mm: 50.0,
            }),
        };
        assert_eq!(
            chamfer.args(store, "id-1").last(),
            Some(&"chamfer:100.0,50.0".to_owned())
        );
        let symmetric = ContourEdit::SetCorner {
            vertex: 0,
            treatment: Some(CornerTreatment::Chamfer {
                d1_mm: 100.0,
                d2_mm: 100.0,
            }),
        };
        assert_eq!(
            symmetric.args(store, "id-1").last(),
            Some(&"chamfer:100.0".to_owned())
        );
    }

    #[test]
    fn the_editor_opens_prefilled_from_the_present_treatment() {
        let mut contour = square();
        contour.corners = vec![steel::model::command::PlateCorner {
            vertex: 1,
            treatment: CornerTreatment::Chamfer {
                d1_mm: 100.0,
                d2_mm: 100.0,
            },
        }];
        let sharp = corner_editor("plate-a", &contour, 0);
        assert_eq!(sharp.mode, CornerMode::Sharp);
        assert_eq!(
            sharp.element, "plate-a",
            "the editor is bound to the selection"
        );
        let chamfer = corner_editor("plate-a", &contour, 1);
        assert_eq!(chamfer.mode, CornerMode::Chamfer);
        assert_eq!(chamfer.d1, "100.0");
        assert_eq!(chamfer.d2, "", "a symmetric chamfer leaves D2 empty");
        // A vertex the contour does not name is sharp, not an error.
        assert_eq!(contour.corner_of(2), None);
    }

    #[test]
    fn apply_maps_the_fields_to_the_treatment_or_refuses() {
        let chamfer = CornerEditor {
            element: "plate-a".into(),
            vertex: 0,
            mode: CornerMode::Chamfer,
            d1: "80".into(),
            d2: String::new(),
            radius: String::new(),
            error: None,
        };
        assert_eq!(
            editor_treatment(&chamfer).unwrap(),
            Some(CornerTreatment::Chamfer {
                d1_mm: 80.0,
                d2_mm: 80.0
            }),
            "an empty D2 cuts the same distance on both edges"
        );
        let round = CornerEditor {
            element: "plate-a".into(),
            vertex: 0,
            mode: CornerMode::Round,
            d1: String::new(),
            d2: String::new(),
            radius: "25.5".into(),
            error: None,
        };
        assert_eq!(
            editor_treatment(&round).unwrap(),
            Some(CornerTreatment::Round { radius_mm: 25.5 })
        );
        let sharp = CornerEditor {
            element: "plate-a".into(),
            vertex: 0,
            mode: CornerMode::Sharp,
            d1: String::new(),
            d2: String::new(),
            radius: String::new(),
            error: None,
        };
        assert_eq!(editor_treatment(&sharp).unwrap(), None);
        let bad_number = CornerEditor {
            d1: "wide".into(),
            ..chamfer.clone()
        };
        assert!(
            editor_treatment(&bad_number)
                .unwrap_err()
                .contains("not a number")
        );
        let bad_value = CornerEditor {
            d1: "0".into(),
            ..chamfer.clone()
        };
        assert!(
            editor_treatment(&bad_value)
                .unwrap_err()
                .contains("positive")
        );
    }
}
