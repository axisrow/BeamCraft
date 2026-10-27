// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V08 form state. All frame construction and coordinate arithmetic belong to the engine.
use crate::tools::FormError;
use beamcraft_engine::work_plane::WorkPlane;

/// Ways to construct a session frame from typed coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Parallel to XY.
    #[default]
    XY,
    /// Parallel to XZ.
    XZ,
    /// Parallel to YZ.
    YZ,
    /// Explicit origin and two orthogonal directions.
    Axes,
    /// Origin, point on +X, and point on positive-Y side.
    ThreePoints,
    /// Vertical through two points.
    Vertical,
}
/// An explicit action from the plane panel; merely editing text changes no active plane.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Install this admitted session frame.
    Apply(WorkPlane),
    /// Restore the legacy global input behavior.
    Clear,
    /// Arm a face pick in the viewport.
    PickFace,
}
/// Uncommitted plane parameters.
#[derive(Debug, Clone)]
pub struct Form {
    /// Construction mode.
    pub mode: Mode,
    /// Origin or first point.
    pub origin: String,
    /// X direction, second point, or vertical-line endpoint.
    pub x: String,
    /// Y direction or third point.
    pub y: String,
    /// Offset along the omitted model axis, millimetres.
    pub offset: String,
}
impl Default for Form {
    fn default() -> Self {
        Self {
            mode: Mode::XY,
            origin: "0,0,0".into(),
            x: "1,0,0".into(),
            y: "0,1,0".into(),
            offset: "0".into(),
        }
    }
}
fn error(message: impl ToString) -> FormError {
    FormError {
        field: "Work plane",
        message: message.to_string(),
    }
}
fn point(text: &str) -> Result<[f64; 3], FormError> {
    if text.len() > 128 {
        return Err(error("coordinate text exceeds limit"));
    }
    let p = text
        .split(',')
        .take(4)
        .map(|p| p.trim().parse::<f64>().map_err(|_| error("enter X,Y,Z")))
        .collect::<Result<Vec<_>, _>>()?;
    p.try_into()
        .map_err(|_| error("enter three coordinates X,Y,Z"))
}
impl Form {
    /// Ask the engine to construct the requested frame; this writes no model or session state.
    pub fn resolve(&self) -> Result<WorkPlane, FormError> {
        let plane = match self.mode {
            Mode::XY | Mode::XZ | Mode::YZ => {
                if self.offset.len() > 32 {
                    return Err(error("offset text exceeds limit"));
                }
                let offset = self
                    .offset
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| error("enter a finite offset"))?;
                let name = match self.mode {
                    Mode::XY => "xy",
                    Mode::XZ => "xz",
                    _ => "yz",
                };
                WorkPlane::parallel(name, offset)
            }
            Mode::Axes => WorkPlane::new(point(&self.origin)?, point(&self.x)?, point(&self.y)?),
            Mode::ThreePoints => {
                WorkPlane::three_points(point(&self.origin)?, point(&self.x)?, point(&self.y)?)
            }
            Mode::Vertical => WorkPlane::vertical(point(&self.origin)?, point(&self.x)?),
        };
        plane.map_err(error)
    }
    /// Fill the editable axes from a face-picked frame.
    pub fn set(&mut self, plane: WorkPlane) {
        let text = |v: [f64; 3]| {
            let [x, y, z] = v;
            format!("{x:?},{y:?},{z:?}")
        };
        self.mode = Mode::Axes;
        self.origin = text(plane.origin());
        self.x = text(plane.x());
        self.y = text(plane.y());
    }
    /// Show the frame editor and return an explicit action, or a validation error to display.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Result<Option<Action>, FormError> {
        let mut action = None;
        ui.collapsing("Work plane", |ui| {
            egui::ComboBox::from_id_salt("work-plane-mode")
                .selected_text(match self.mode {
                    Mode::XY => "XY",
                    Mode::XZ => "XZ",
                    Mode::YZ => "YZ",
                    Mode::Axes => "Origin and axes",
                    Mode::ThreePoints => "Three points",
                    Mode::Vertical => "Vertical through two points",
                })
                .show_ui(ui, |ui| {
                    for (mode, label) in [
                        (Mode::XY, "XY"),
                        (Mode::XZ, "XZ"),
                        (Mode::YZ, "YZ"),
                        (Mode::Axes, "Origin and axes"),
                        (Mode::ThreePoints, "Three points"),
                        (Mode::Vertical, "Vertical through two points"),
                    ] {
                        ui.selectable_value(&mut self.mode, mode, label);
                    }
                });
            let field = |ui: &mut egui::Ui, label, text: &mut String| {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(text).char_limit(128));
            };
            if matches!(self.mode, Mode::XY | Mode::XZ | Mode::YZ) {
                field(ui, "Model axis offset (mm)", &mut self.offset);
            } else {
                field(ui, "Origin X,Y,Z (mm)", &mut self.origin);
                field(
                    ui,
                    if self.mode == Mode::Axes {
                        "X direction"
                    } else {
                        "Second point X,Y,Z (mm)"
                    },
                    &mut self.x,
                );
                if self.mode != Mode::Vertical {
                    field(
                        ui,
                        if self.mode == Mode::Axes {
                            "Y direction"
                        } else {
                            "Third point X,Y,Z (mm)"
                        },
                        &mut self.y,
                    );
                }
            }
            if ui.button("Apply plane").clicked() {
                action = Some(self.resolve().map(Action::Apply));
            }
            if ui.button("Pick face in view").clicked() {
                action = Some(Ok(Action::PickFace));
            }
            if ui.button("Global coordinates").clicked() {
                action = Some(Ok(Action::Clear));
            }
        });
        action.transpose()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn form_construction_and_face_fill_use_engine_frames() {
        let mut form = Form {
            mode: Mode::XZ,
            offset: "200".into(),
            ..Form::default()
        };
        assert_eq!(
            form.resolve().unwrap().to_world([10., 30., 0.]).unwrap(),
            [10., 200., 30.]
        );
        form.set(WorkPlane::face([10., 20., 30.], [0., 1., 0.]).unwrap());
        let plane = form.resolve().unwrap();
        assert_eq!(plane.origin(), [10., 20., 30.]);
        assert_eq!(plane.normal(), [0., 1., 0.]);
        form.x = "0,0,0".into();
        assert!(form.resolve().is_err());
        form.mode = Mode::XY;
        form.offset = "NaN".into();
        assert!(form.resolve().is_err());
    }
}
