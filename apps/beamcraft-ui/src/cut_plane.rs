// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V15 cut plane: the window's non-destructive display cut and the uniform the renderer
//! binds. All plane arithmetic belongs to the engine; this module only formats and shows.
use crate::tools::FormError;
use beamcraft_engine::view_clipping::{ClipPlane, Clipping, MAX_EFFECTIVE, WorkArea};

/// The clip uniform layout: a plane count, then up to [`MAX_EFFECTIVE`] half-spaces as
/// `(normal.xyz, -normal·point)` in display f32. Zero planes cut nothing.
#[derive(Debug, Clone, Copy)]
pub struct ClipGpu {
    /// Half-space count the shader iterates; the rest of the array is ignored.
    pub count: u32,
    planes: [[f32; 4]; MAX_EFFECTIVE],
}
impl ClipGpu {
    /// Little-endian uniform bytes, `16 + MAX_EFFECTIVE * 16` wide.
    pub const BYTES: usize = 16 + MAX_EFFECTIVE * 16;
    /// The empty cut: every fragment passes.
    pub fn none() -> Self {
        Self {
            count: 0,
            planes: [[0.0; 4]; MAX_EFFECTIVE],
        }
    }
    /// Expands a validated cut into display planes. Plane points are model millimetres; the
    /// shader's world is the scene re-centred on `origin_mm`, so the offset moves with it. A
    /// record over the budget clips at the engine cap instead of failing a frame, and an
    /// invalid one cuts nothing.
    pub fn of(clip: &Clipping, origin_mm: [f64; 3]) -> Self {
        let planes = match clip.effective() {
            Ok(planes) => planes,
            Err(_) => return Self::none(),
        };
        let mut gpu = Self::none();
        for (plane, slot) in planes.iter().zip(gpu.planes.iter_mut()) {
            let [nx, ny, nz] = plane.normal.map(|v| v as f32);
            let mut point = [0.0f32; 3];
            for ((at, p), o) in point.iter_mut().zip(plane.point_mm).zip(origin_mm) {
                *at = (p - o) as f32;
            }
            let [px, py, pz] = point;
            *slot = [nx, ny, nz, -(nx * px + ny * py + nz * pz)];
            gpu.count += 1;
        }
        gpu
    }
    /// The uniform bytes for [`crate::render::Renderer::draw`].
    pub fn bytes(self) -> [u8; Self::BYTES] {
        let mut out = [0u8; Self::BYTES];
        out[..4].copy_from_slice(&self.count.to_le_bytes());
        // Each plane is four f32 slots; the slots run on from the 16-byte header.
        let slots = out[16..].as_chunks_mut::<4>().0;
        for (plane, group) in self.planes.iter().zip(slots.as_chunks_mut::<4>().0) {
            for (slot, value) in group.iter_mut().zip(plane.map(f32::to_le_bytes)) {
                *slot = value;
            }
        }
        out
    }
}
/// Ways to state the one user clip plane of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Horizontal: everything at and above `z = offset` stays visible.
    #[default]
    XY,
    /// Facing +Y: everything at and beyond `y = offset` stays visible.
    XZ,
    /// Facing +X: everything at and beyond `x = offset` stays visible.
    YZ,
    /// Explicit point and normal, the visible half-space faces the normal.
    Point,
}
/// An explicit action from the cut panel; editing text changes no applied cut.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Install this admitted cut.
    Apply(Clipping),
    /// Remove the cut: the whole model is visible again.
    Clear,
}
/// Uncommitted cut parameters.
#[derive(Debug, Clone, Default)]
pub struct Form {
    /// How the single user plane is stated.
    pub mode: Mode,
    /// Plane offset along the model axis, millimetres (`Mode::XY`, `XZ`, `YZ`).
    pub offset: String,
    /// Plane point "X,Y,Z" (`Mode::Point`).
    pub point: String,
    /// Visible-side normal "X,Y,Z" (`Mode::Point`).
    pub normal: String,
    /// Whether the work-area box is applied.
    pub area: bool,
    /// Work-area minimum corner "X,Y,Z".
    pub min: String,
    /// Work-area maximum corner "X,Y,Z".
    pub max: String,
}
fn error(message: impl ToString) -> FormError {
    FormError {
        field: "Cut plane",
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
    /// The single user plane of the form, by mode.
    pub fn plane(&self) -> Result<ClipPlane, FormError> {
        match self.mode {
            Mode::XY | Mode::XZ | Mode::YZ => {
                if self.offset.len() > 32 {
                    return Err(error("offset text exceeds limit"));
                }
                let offset = self
                    .offset
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| error("enter a finite offset"))?;
                let (point, normal) = match self.mode {
                    Mode::XY => ([0.0, 0.0, offset], [0.0, 0.0, 1.0]),
                    Mode::XZ => ([0.0, offset, 0.0], [0.0, 1.0, 0.0]),
                    _ => ([offset, 0.0, 0.0], [1.0, 0.0, 0.0]),
                };
                ClipPlane::new(point, normal).map_err(error)
            }
            Mode::Point => ClipPlane::new(point(&self.point)?, point(&self.normal)?).map_err(error),
        }
    }
    /// The work-area box of the form, when it is switched on.
    pub fn box_area(&self) -> Result<Option<WorkArea>, FormError> {
        if !self.area {
            return Ok(None);
        }
        WorkArea::new(point(&self.min)?, point(&self.max)?)
            .map(Some)
            .map_err(error)
    }
    /// Ask the engine to admit the requested cut; this writes no model or window state.
    pub fn resolve(&self) -> Result<Clipping, FormError> {
        Ok(Clipping {
            planes: vec![self.plane()?],
            area: self.box_area()?,
        })
    }
    /// Show the cut editor and return an explicit action, or a validation error to display.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Result<Option<Action>, FormError> {
        let mut action = None;
        ui.collapsing("Cut plane", |ui| {
            egui::ComboBox::from_id_salt("cut-plane-mode")
                .selected_text(match self.mode {
                    Mode::XY => "XY (cut below)",
                    Mode::XZ => "XZ (cut in front)",
                    Mode::YZ => "YZ (cut left)",
                    Mode::Point => "Point and normal",
                })
                .show_ui(ui, |ui| {
                    for (mode, label) in [
                        (Mode::XY, "XY (cut below)"),
                        (Mode::XZ, "XZ (cut in front)"),
                        (Mode::YZ, "YZ (cut left)"),
                        (Mode::Point, "Point and normal"),
                    ] {
                        ui.selectable_value(&mut self.mode, mode, label);
                    }
                });
            if matches!(self.mode, Mode::XY | Mode::XZ | Mode::YZ) {
                ui.label("Model axis offset (mm)");
                ui.add(egui::TextEdit::singleline(&mut self.offset).char_limit(32));
            } else {
                ui.label("Point X,Y,Z (mm)");
                ui.add(egui::TextEdit::singleline(&mut self.point).char_limit(128));
                ui.label("Normal X,Y,Z");
                ui.add(egui::TextEdit::singleline(&mut self.normal).char_limit(128));
            }
            ui.checkbox(&mut self.area, "Work-area box");
            if self.area {
                ui.label("Min X,Y,Z (mm)");
                ui.add(egui::TextEdit::singleline(&mut self.min).char_limit(128));
                ui.label("Max X,Y,Z (mm)");
                ui.add(egui::TextEdit::singleline(&mut self.max).char_limit(128));
            }
            ui.horizontal(|ui| {
                if ui.button("Apply cut").clicked() {
                    action = Some(self.resolve().map(Action::Apply));
                }
                if ui.button("Remove cut").clicked() {
                    action = Some(Ok(Action::Clear));
                }
            });
        });
        action.transpose()
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn the_form_resolves_axis_and_explicit_cuts() {
        let form = Form {
            mode: Mode::XY,
            offset: "200".into(),
            ..Form::default()
        };
        let clip = form.resolve().unwrap();
        assert_eq!(clip.planes.len(), 1);
        assert!(clip.area.is_none());
        assert_eq!(clip.planes[0].distance([0.0, 0.0, 300.0]), 100.0);
        assert_eq!(clip.planes[0].distance([0.0, 0.0, 100.0]), -100.0);
        let explicit = Form {
            mode: Mode::Point,
            point: "1,2,3".into(),
            normal: "0,2,0".into(),
            ..Form::default()
        };
        let clip = explicit.resolve().unwrap();
        assert_eq!(clip.planes[0].normal, [0.0, 1.0, 0.0]);
        assert_eq!(clip.planes[0].point_mm, [1.0, 2.0, 3.0]);
        for bad in [
            Form {
                mode: Mode::XY,
                offset: "NaN".into(),
                ..Form::default()
            },
            Form {
                mode: Mode::Point,
                point: "0,0".into(),
                ..Form::default()
            },
            Form {
                mode: Mode::Point,
                point: "0,0,0".into(),
                normal: "0,0,0".into(),
                ..Form::default()
            },
        ] {
            assert!(bad.resolve().is_err());
        }
    }
    #[test]
    fn the_work_area_switch_and_the_gpu_form_follow_the_record() {
        let mut form = Form {
            offset: "0".into(),
            area: true,
            min: "-10,-10,-10".into(),
            max: "10,10,10".into(),
            ..Form::default()
        };
        let clip = form.resolve().unwrap();
        let planes = clip.effective().unwrap();
        assert_eq!(planes.len(), 7);
        form.max = "-20,10,10".into();
        assert!(form.resolve().is_err());
        // The GPU form carries the planes, offset into the scene-centred display space; an
        // empty record cuts nothing.
        let origin = [10.0, 0.0, 0.0];
        let gpu = ClipGpu::of(&clip, origin);
        assert_eq!(gpu.count, 7);
        let bytes = gpu.bytes();
        assert_eq!(bytes.len(), ClipGpu::BYTES);
        assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()), 7);
        // The first plane is the form's XY plane at z=0: the offset leaves it untouched.
        let first: [u8; 16] = bytes[16..32].try_into().unwrap();
        let values: Vec<f32> = first
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(values, [0.0, 0.0, 1.0, 0.0]);
        // The second plane is the box's +X face: each plane owns a full 16-byte stride.
        let second: [u8; 16] = bytes[32..48].try_into().unwrap();
        let values: Vec<f32> = second
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(values, [1.0, 0.0, 0.0, 20.0], "point -10 - origin 10");
        assert_eq!(ClipGpu::of(&Clipping::default(), origin).count, 0);
        assert_eq!(ClipGpu::none().bytes()[..4], [0; 4]);
    }
}
