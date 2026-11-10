// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Bounded model-library thumbnails of actual engine geometry, not illustrative sample models.

use crate::Error;
use crate::camera::Camera;
use crate::scene::{INSTANCE_BYTES, Scene, VERTEX_BYTES};
use crate::theme::Palette;
use beamcraft_engine::{Representation, ViewSnapshot};
use egui::{Color32, Painter, Pos2, Rect, pos2};

/// Maximum geometry drawn in a library thumbnail; larger models get an explicit preview refusal.
pub const MAX_TRIANGLES: usize = 8_192;

#[derive(Debug)]
struct Face {
    points: [Pos2; 3],
    depth: f32,
    light: f32,
    /// The world-space face normal (scene-centred), for the cull test and its test.
    #[allow(dead_code)] // read by the unit test, kept beside the cull that fills it
    normal: [f32; 3],
    /// The face midpoint, scene-centred.
    #[allow(dead_code)]
    mid: [f32; 3],
}

/// A display-only projection of an engine snapshot, built once on a worker thread.
#[derive(Debug)]
pub struct ModelPreview {
    faces: Vec<Face>,
    /// Actual number of elements in the inspected model.
    pub elements: usize,
}

fn floats<const N: usize>(bytes: &[u8]) -> Result<[f32; N], Error> {
    if bytes.len() != N.saturating_mul(4) {
        return Err(Error::Input("invalid preview display buffer"));
    }
    let mut out = [0.0; N];
    for (value, chunk) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *value = f32::from_le_bytes(*chunk);
    }
    Ok(out)
}

fn cross([a, b, c]: [f32; 3], [x, y, z]: [f32; 3]) -> [f32; 3] {
    [b * z - c * y, c * x - a * z, a * y - b * x]
}

fn rotate(point: [f32; 3], [x, y, z, w]: [f32; 4]) -> [f32; 3] {
    let q = [x, y, z];
    let uv = cross(q, point);
    let uuv = cross(q, uv);
    let mut out = [0.0; 3];
    for (((out, p), u), v) in out.iter_mut().zip(point).zip(uv).zip(uuv) {
        *out = p + 2.0 * (w * u + v);
    }
    out
}

impl ModelPreview {
    /// Projects the existing validated display buffers with the viewport's own camera.
    pub fn from_snapshot(snapshot: &ViewSnapshot) -> Result<Self, Error> {
        let scene = Scene::from_snapshot(snapshot, Representation::Full)?;
        if scene.triangles > MAX_TRIANGLES as u64 {
            return Err(Error::Input(
                "model exceeds the library preview triangle budget",
            ));
        }
        let (target, extent) = scene.framing();
        let view = Camera::new(target, extent, 268.0 / 184.0)?.view(268.0 / 184.0)?;
        let mut faces = Vec::new();
        for mesh in &scene.meshes {
            for instance in mesh.instances.as_chunks::<INSTANCE_BYTES>().0 {
                // Placement and quaternion only; the overlay tint is display state the
                // library preview does not show.
                let [tx, ty, tz, qx, qy, qz, qw] = floats::<7>(&instance[..28])?;
                let quaternion = [qx, qy, qz, qw];
                for triangle in mesh.vertices.as_chunks::<{ VERTEX_BYTES * 3 }>().0 {
                    if faces.len() >= MAX_TRIANGLES {
                        return Err(Error::Input(
                            "model exceeds the library preview triangle budget",
                        ));
                    }
                    let mut points = [Pos2::ZERO; 3];
                    let mut depth = 0.0;
                    let mut light = 0.5;
                    let mut normal = [0.0f32; 3];
                    let mut mid = [0.0f32; 3];
                    for (point, vertex) in points
                        .iter_mut()
                        .zip(triangle.as_chunks::<VERTEX_BYTES>().0)
                    {
                        let [x, y, z, nx, ny, nz] = floats::<6>(vertex)?;
                        let [x, y, z] = rotate([x, y, z], quaternion);
                        let [sx, sy, sz] = view.project([x + tx, y + ty, z + tz]);
                        *point = pos2(sx * 0.5 + 0.5, 0.5 - sy * 0.5);
                        depth += sz / 3.0;
                        let [rnx, rny, rnz] = rotate([nx, ny, nz], quaternion);
                        normal = [normal[0] + rnx, normal[1] + rny, normal[2] + rnz];
                        mid = [mid[0] + x + tx, mid[1] + y + ty, mid[2] + z + tz];
                        light = (0.55 + 0.45 * (rnx * 0.3 - rny * 0.4 + rnz * 0.85).max(0.0))
                            .clamp(0.4, 1.0);
                    }
                    // Back-face cull (issue 569 item 15): a face turned away from the eye never
                    // paints, so a plain box's hidden faces cannot streak over the visible ones
                    // in the painter's sort. The flat painter stays; convex parts read clean.
                    let eye_to_face = [
                        mid[0] / 3.0 - view.eye[0],
                        mid[1] / 3.0 - view.eye[1],
                        mid[2] / 3.0 - view.eye[2],
                    ];
                    if normal[0] * eye_to_face[0]
                        + normal[1] * eye_to_face[1]
                        + normal[2] * eye_to_face[2]
                        >= 0.0
                    {
                        continue;
                    }
                    faces.push(Face {
                        points,
                        depth,
                        light,
                        normal,
                        mid,
                    });
                }
            }
        }
        faces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        Ok(Self {
            faces,
            elements: scene.elements,
        })
    }

    /// Paints only this model's actual projected faces into a library preview or row thumbnail.
    pub fn paint(&self, painter: &Painter, rect: Rect, palette: Palette) {
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 3, palette.head);
        let mut mesh = egui::Mesh::default();
        for face in &self.faces {
            let points = face.points.map(|point| {
                pos2(
                    rect.left() + point.x * rect.width(),
                    rect.top() + point.y * rect.height(),
                )
            });
            let base = if palette.dark {
                [139.0, 170.0, 190.0]
            } else {
                [129.0, 154.0, 169.0]
            };
            let [r, g, b] = base.map(|value| (value * face.light) as u8);
            // No per-face feathering: it produces long miters on thin steel triangles.
            let first = mesh.vertices.len() as u32;
            for point in points {
                mesh.colored_vertex(point, Color32::from_rgb(r, g, b));
            }
            mesh.add_triangle(first, first + 1, first + 2);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use beamcraft_engine::{Limits, ViewPin, synthetic::plate_scene};
    use beamcraft_model::digest::Sha256;

    #[test]
    fn preview_uses_real_snapshot_elements_and_bounds_its_geometry() {
        let pin = ViewPin::new(Sha256::digest(b"preview").unwrap(), 0);
        let snapshot =
            ViewSnapshot::build(pin, plate_scene(3, false).unwrap(), &Limits::default()).unwrap();
        let preview = ModelPreview::from_snapshot(&snapshot).unwrap();
        assert_eq!(preview.elements, 3);
        // Back faces are culled (issue 569 item 15): some of the 36 triangles survive — the
        // ones facing the fitted camera — and every survivor really faces it.
        assert!(preview.faces.len() < 36, "back faces are culled");
        assert!(!preview.faces.is_empty());
        let (target, extent) = crate::scene::Scene::from_snapshot(&snapshot, Representation::Full)
            .unwrap()
            .framing();
        let view = Camera::new(target, extent, 268.0 / 184.0)
            .unwrap()
            .view(268.0 / 184.0)
            .unwrap();
        for face in &preview.faces {
            assert!(
                face.normal[0] * (view.eye[0] - face.mid[0] / 3.0)
                    + face.normal[1] * (view.eye[1] - face.mid[1] / 3.0)
                    + face.normal[2] * (view.eye[2] - face.mid[2] / 3.0)
                    > 0.0,
                "every kept face points toward the eye"
            );
        }
        assert!(
            preview
                .faces
                .iter()
                .flat_map(|face| face.points)
                .all(|point| point.is_finite())
        );
        let context = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(268.0, 184.0));
        let output = context.run_ui(Default::default(), |ui| {
            preview.paint(ui.painter(), rect, Palette::current(ui));
        });
        let vertices: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Mesh(mesh) = &shape.shape {
                    Some(mesh.vertices.iter().map(|vertex| vertex.pos))
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        output.drop_without_applying_deltas();
        // Only the kept faces reach the mesh: one triangle per face, its three vertices.
        assert_eq!(vertices.len(), preview.faces.len() * 3);
        assert!(vertices.iter().all(|position| rect.contains(*position)));
    }

    #[test]
    fn preview_refuses_over_budget_instead_of_showing_incomplete_geometry() {
        let pin = ViewPin::new(Sha256::digest(b"preview-budget").unwrap(), 0);
        let snapshot = ViewSnapshot::build(
            pin,
            plate_scene((MAX_TRIANGLES / 12 + 1).try_into().unwrap(), false).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        assert!(matches!(
            ModelPreview::from_snapshot(&snapshot),
            Err(Error::Input(
                "model exceeds the library preview triangle budget"
            ))
        ));
    }

    #[test]
    fn preview_preserves_rotated_placements() {
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let [x, y, z] = rotate([1.0, 0.0, 0.0], [0.0, 0.0, half, half]);
        assert!(x.abs() < 1e-6 && (y - 1.0).abs() < 1e-6 && z.abs() < 1e-6);
    }
}
