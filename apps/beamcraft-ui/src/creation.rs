// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Bounded interactive contour drafts. Only the engine decides geometric admission.

use crate::tools::FormError;
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_model::limits::{CONTOUR_POINT_CAP, COORDINATE_CAP_MM};

/// Unpublished world points of one plate contour, in the picked order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContourDraft {
    points: Vec<[f64; 3]>,
}

impl ContourDraft {
    /// Hit the first visible draft marker within ten screen points, independently of model snaps.
    pub fn closes_near(
        &self,
        rect: egui::Rect,
        view: &crate::camera::View,
        origin: [f64; 3],
        pointer: egui::Pos2,
    ) -> bool {
        if self.points.len() < 3 || !rect.contains(pointer) {
            return false;
        }
        let Some(point) = self.points.first() else {
            return false;
        };
        let local = std::array::from_fn(|i| {
            (point.get(i).copied().unwrap_or(0.0) - origin.get(i).copied().unwrap_or(0.0)) as f32
        });
        let [x, y, z] = view.project(local);
        if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&z) {
            return false;
        }
        let marker = egui::pos2(
            rect.left() + (x + 1.0) * rect.width() / 2.0,
            rect.top() + (1.0 - y) * rect.height() / 2.0,
        );
        rect.contains(marker) && marker.distance(pointer) <= 10.0
    }
    /// World vertices for the draft overlay.
    pub fn points(&self) -> &[[f64; 3]] {
        &self.points
    }

    /// Add one finite point without allocating beyond the engine's contour budget.
    pub fn push(&mut self, point: [f64; 3]) -> Result<(), FormError> {
        if self.points.len() >= CONTOUR_POINT_CAP {
            return Err(FormError {
                field: "Contour",
                message: "The contour has reached its vertex limit.".into(),
            });
        }
        if point
            .iter()
            .any(|p| !p.is_finite() || p.abs() > COORDINATE_CAP_MM)
        {
            return Err(FormError {
                field: "Point",
                message: "Choose a finite point inside the model limits.".into(),
            });
        }
        self.points.push(point);
        Ok(())
    }

    /// Undo the last draft vertex without touching model history.
    pub fn back(&mut self) {
        self.points.pop();
    }

    /// Explicitly change the picked contour's direction without changing its vertex positions.
    pub fn reverse(&mut self) {
        self.points.reverse();
    }

    /// Discard an unfinished contour; this runs no command.
    pub fn clear(&mut self) {
        self.points.clear();
    }

    /// Resolve a spatial contour through the engine without flattening it to the session plane.
    pub fn spatial_contour(
        &self,
        preferred: Option<WorkPlane>,
    ) -> Result<(String, WorkPlane), FormError> {
        let plane =
            beamcraft_engine::coordinate::contour_frame(&self.points, preferred).map_err(|e| {
                FormError {
                    field: "Contour",
                    message: e.message.into(),
                }
            })?;
        Ok((self.contour(Some(plane))?, plane))
    }

    /// Convert the draft through the engine frame for the existing local-contour CLI.
    /// Preserve winding, topology and coordinates for engine validation; repair nothing.
    pub fn contour(&self, plane: Option<WorkPlane>) -> Result<String, FormError> {
        if self.points.len() < 3 {
            return Err(FormError {
                field: "Contour",
                message: "Pick at least three vertices.".into(),
            });
        }
        self.points
            .iter()
            .map(|point| {
                let [x, y, z] = match plane {
                    Some(p) => p.to_local(*point).map_err(|e| FormError {
                        field: "Work plane",
                        message: e.message.into(),
                    })?,
                    None => *point,
                };
                if z.abs() > 1e-6 {
                    return Err(FormError {
                        field: "Point",
                        message: "All vertices must lie on the active work plane.".into(),
                    });
                }
                Ok(format!("{x},{y}"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|points| points.join(";"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn first_marker_closes_in_pixels_at_different_scales_without_vertex_snaps() {
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(800.0, 800.0));
        for extent in [100.0, 100_000.0] {
            let view = crate::camera::Camera::new([0.0; 3], extent, 1.0)
                .unwrap()
                .view(1.0)
                .unwrap();
            for origin in [[0.0; 3], [100_000_000.0, 0.0, 0.0]] {
                let mut draft = ContourDraft::default();
                draft.push(origin).unwrap();
                draft
                    .push([origin[0] + 10.0, origin[1], origin[2]])
                    .unwrap();
                assert!(!draft.closes_near(rect, &view, origin, rect.center()));
                draft
                    .push([origin[0], origin[1] + 10.0, origin[2]])
                    .unwrap();
                assert!(draft.closes_near(
                    rect,
                    &view,
                    origin,
                    rect.center() + egui::vec2(8.0, 0.0)
                ));
                assert!(!draft.closes_near(
                    rect,
                    &view,
                    origin,
                    rect.center() + egui::vec2(14.0, 0.0)
                ));
                assert_eq!(draft.points().len(), 3);
            }
        }
    }
}
