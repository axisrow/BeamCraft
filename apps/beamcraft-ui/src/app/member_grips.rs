// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The start and end handles of the selected straight member. Dragging a handle slides that end
//! strictly along the member's axis; clicking one selects the point, and the Move command then
//! places that point instead of the whole member. Both commit one `update-element --axis-point`,
//! the command a user types, so one gesture is one revision and one undo unit.

use super::contour_grips::project;
use super::{App, workbench};
use crate::edit::{AxisEnd, MemberAxis, Ray, Target, ndc_of, ray_through};
use crate::theme::{BRAND, BRAND_LIGHT};
use beamcraft_model::limits::COORDINATE_CAP_MM;
use egui::{Pos2, Rect, Response, Stroke, Vec2};

/// A press within this many points of a handle takes the handle.
const GRIP_RADIUS: f32 = 8.0;

/// The shortest member a drag may leave, millimetres: a handle stops before it reaches the other.
const MIN_LENGTH_MM: f64 = 1.0;

/// A handle being dragged and the world point the release commits.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    end: AxisEnd,
    to: [f64; 3],
}

/// The state of the member handles.
#[derive(Debug, Default)]
pub(crate) struct Grips {
    drag: Option<Drag>,
}

impl Grips {
    /// Whether a handle holds the pointer, so the camera takes no primary drag.
    pub(crate) fn holds_pointer(&self) -> bool {
        self.drag.is_some()
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn minus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The point of the member's axis line nearest the pointer's ray: where `end` lands. It stays
/// on the far side of the other end by at least [`MIN_LENGTH_MM`], so a drag never flips the
/// member. `None` when the view looks along the axis or the point is out of range.
fn along_axis(axis: &MemberAxis, end: AxisEnd, ray: &Ray) -> Option<[f64; 3]> {
    let start = axis.world(AxisEnd::Start);
    let span = minus(axis.world(AxisEnd::End), start);
    let length = dot(span, span).sqrt();
    if !length.is_finite() || length <= 0.0 {
        return None;
    }
    let direction = span.map(|v| v / length);
    let to_line = minus(start, ray.origin);
    let (a, b, c) = (
        dot(ray.direction, ray.direction),
        dot(ray.direction, direction),
        1.0,
    );
    let denominator = a * c - b * b;
    if !denominator.is_finite() || denominator.abs() < 1e-9 * a {
        return None;
    }
    let (d, e) = (dot(ray.direction, to_line), dot(direction, to_line));
    let along = (b * d - a * e) / denominator;
    let along = match end {
        AxisEnd::Start => along.min(length - MIN_LENGTH_MM),
        AxisEnd::End => along.max(MIN_LENGTH_MM),
    };
    let point = [
        start[0] + along * direction[0],
        start[1] + along * direction[1],
        start[2] + along * direction[2],
    ];
    point
        .iter()
        .all(|v| v.is_finite() && v.abs() <= COORDINATE_CAP_MM)
        .then_some(point)
}

/// The handle under `pos`, the nearer one when both are in reach.
fn grip_at(ends: &[(AxisEnd, Pos2)], pos: Pos2) -> Option<AxisEnd> {
    ends.iter()
        .filter(|(_, p)| p.distance(pos) <= GRIP_RADIUS)
        .min_by(|a, b| a.1.distance(pos).total_cmp(&b.1.distance(pos)))
        .map(|(end, _)| *end)
}

/// The two handles in screen points; one behind the camera has none.
fn ends_screen(
    axis: &MemberAxis,
    view: &crate::camera::View,
    rect: Rect,
    origin: [f64; 3],
) -> Vec<(AxisEnd, Pos2)> {
    [AxisEnd::Start, AxisEnd::End]
        .into_iter()
        .filter_map(|end| project(&axis.world(end), origin, view, rect).map(|p| (end, p)))
        .collect()
}

impl App {
    /// The selected member's axis while its handles are live: on the Select and Move benches,
    /// with no creation tool or view pick owning the pointer, and for the element the
    /// properties were read from.
    fn grip_axis(&self) -> Option<MemberAxis> {
        let live = !self.beam_mode
            && !self.plate_mode
            && self.views.picking.is_none()
            && matches!(
                self.bench,
                workbench::Bench::Select | workbench::Bench::Move
            );
        let details = self.props.details().filter(|_| live)?;
        (self.editor.selected()?.id == details.id)
            .then(|| details.member_axis())
            .flatten()
    }

    /// Draws the handles, the selected point and the member a live drag would leave.
    pub(crate) fn paint_member_grips(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        view: &crate::camera::View,
        origin: [f64; 3],
    ) {
        let Some(axis) = self.grip_axis() else {
            return;
        };
        let drag = self.member_grips.drag;
        if let Some(drag) = drag
            && let (Some(a), Some(b)) = (
                project(&axis.world(drag.end.other()), origin, view, rect),
                project(&drag.to, origin, view, rect),
            )
        {
            painter.line_segment([a, b], Stroke::new(2.0, BRAND_LIGHT));
        }
        let selected = self.editor.axis_point().map(|(_, end)| end);
        for (end, pos) in ends_screen(&axis, view, rect, origin) {
            let pos = match drag {
                Some(drag) if drag.end == end => {
                    project(&drag.to, origin, view, rect).unwrap_or(pos)
                }
                _ => pos,
            };
            let active = selected == Some(end) || drag.is_some_and(|d| d.end == end);
            let size = Vec2::splat(if active { 12.0 } else { 8.0 });
            // White inside a dark outline: the handle must read against the teal selection.
            let square = Rect::from_center_size(pos, size);
            let colour = if active {
                BRAND_LIGHT
            } else {
                egui::Color32::WHITE
            };
            painter.rect_filled(square, 1.0, colour);
            painter.rect_stroke(
                square,
                1.0,
                Stroke::new(1.5, BRAND),
                egui::StrokeKind::Outside,
            );
        }
    }

    /// Feeds one frame of the pane's pointer to the handles. True when they used the input, so
    /// it neither selects nor moves the camera.
    pub(crate) fn member_grip_input(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        view: &crate::camera::View,
    ) -> bool {
        let Some(axis) = self.grip_axis() else {
            self.member_grips.drag = None;
            return false;
        };
        let origin = self
            .loaded
            .as_ref()
            .map_or([0.0; 3], |l| l.summary.origin_mm);
        let rect = response.rect;
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));

        if let Some(drag) = self.member_grips.drag {
            if esc {
                self.member_grips.drag = None;
                return true;
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                self.member_grips.drag = None;
                self.commit_axis_drag(&axis, drag);
                return true;
            }
            if let Some(pos) = response.interact_pointer_pos()
                && let Some(ndc) = ndc_of(
                    [pos.x, pos.y],
                    [rect.min.x, rect.min.y],
                    [rect.width(), rect.height()],
                )
                && let Some(ray) = ray_through(view, ndc, origin)
                && let Some(to) = along_axis(&axis, drag.end, &ray)
                && let Some(live) = self.member_grips.drag.as_mut()
            {
                live.to = to;
            }
            return true;
        }

        if esc {
            self.editor.clear_axis_point();
        }
        let ends = ends_screen(&axis, view, rect, origin);
        if response.drag_started_by(egui::PointerButton::Primary)
            && !self.other_command_running()
            && let Some(press) = ui.input(|i| i.pointer.press_origin())
            && let Some(end) = grip_at(&ends, press)
        {
            self.member_grips.drag = Some(Drag {
                end,
                to: axis.world(end),
            });
            return true;
        }
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            if let Some(end) = grip_at(&ends, pos) {
                self.editor.select_axis_point(axis, end);
                self.edit_note = Some(Ok(format!(
                    "{} point selected: the Move command places it.",
                    end.label()
                )));
                return true;
            }
            self.editor.clear_axis_point();
        }
        false
    }

    /// Runs the released drag as one edit.
    fn commit_axis_drag(&mut self, axis: &MemberAxis, drag: Drag) {
        if drag.to == axis.world(drag.end) {
            return;
        }
        if self.other_command_running() {
            self.edit_note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let target = Target {
            store: &self.source.store,
            workspace: self.source.workspace.as_deref(),
        };
        match self.editor.begin_axis_move(axis, drag.end, drag.to, target) {
            Ok(Some(edit)) => self.spawn_edit(edit),
            Ok(None) => {}
            Err(error) => self.edit_note = Some(Err(error.to_string())),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn beam() -> MemberAxis {
        MemberAxis {
            points: [[0.0, 0.0, 0.0], [4000.0, 0.0, 0.0]],
            translation_mm: [100.0, 200.0, 300.0],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn ray_down_at(x: f64, y: f64) -> Ray {
        Ray {
            origin: [x, y, 5000.0],
            direction: [0.0, 0.0, -1.0],
        }
    }

    #[test]
    fn a_handle_lands_on_the_axis_however_far_the_pointer_is_off_it() {
        let to = along_axis(&beam(), AxisEnd::End, &ray_down_at(5100.0, 9000.0)).unwrap();
        assert!((to[0] - 5100.0).abs() < 1e-6, "{to:?}");
        assert!((to[1] - 200.0).abs() < 1e-6, "{to:?}");
        assert!((to[2] - 300.0).abs() < 1e-6, "{to:?}");
    }

    #[test]
    fn a_handle_stops_before_the_other_end() {
        let end = along_axis(&beam(), AxisEnd::End, &ray_down_at(-9000.0, 0.0)).unwrap();
        assert!((end[0] - (100.0 + MIN_LENGTH_MM)).abs() < 1e-6, "{end:?}");
        let start = along_axis(&beam(), AxisEnd::Start, &ray_down_at(9000.0, 0.0)).unwrap();
        assert!(
            (start[0] - (4100.0 - MIN_LENGTH_MM)).abs() < 1e-6,
            "{start:?}"
        );
    }

    #[test]
    fn looking_along_the_axis_names_no_point() {
        let ray = Ray {
            origin: [-5000.0, 200.0, 300.0],
            direction: [1.0, 0.0, 0.0],
        };
        assert_eq!(along_axis(&beam(), AxisEnd::End, &ray), None);
    }

    #[test]
    fn the_nearer_handle_within_reach_is_taken() {
        let ends = [
            (AxisEnd::Start, Pos2::new(10.0, 10.0)),
            (AxisEnd::End, Pos2::new(20.0, 10.0)),
        ];
        assert_eq!(grip_at(&ends, Pos2::new(18.0, 10.0)), Some(AxisEnd::End));
        assert_eq!(grip_at(&ends, Pos2::new(100.0, 10.0)), None);
    }
}
