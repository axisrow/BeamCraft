// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Canonical evaluated geometry for numbering, independent of mesh indices and placement.

use super::NumberingError;
use crate::solid::SolidMesh;
use beamcraft_geom::Frame3;
use beamcraft_model::digest::{ContentDigest, Sha256};

/// The surface of an evaluated part in its own frame. Triangle and vertex numbering, face
/// labels and rigid translation do not enter the signature. The adapter supplies the part's
/// frame (the section frame for a member); the scene placement never enters this type.
#[derive(Debug, Clone, PartialEq)]
pub struct PartGeometry {
    triangles: Vec<[[f64; 3]; 3]>,
    digest: ContentDigest,
}

fn point_order(a: &[f64; 3], b: &[f64; 3]) -> core::cmp::Ordering {
    a.iter()
        .zip(b)
        .map(|(a, b)| a.total_cmp(b))
        .find(|o| !o.is_eq())
        .unwrap_or(core::cmp::Ordering::Equal)
}

fn triangle_order(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> core::cmp::Ordering {
    a.iter()
        .zip(b)
        .map(|(a, b)| point_order(a, b))
        .find(|o| !o.is_eq())
        .unwrap_or(core::cmp::Ordering::Equal)
}

impl PartGeometry {
    /// Evaluated plate geometry in the element's local frame, without scene placement.
    pub fn plate(solid: &SolidMesh) -> Result<Self, NumberingError> {
        Self::from_solid(solid, Frame3::IDENTITY)
    }

    /// Normalizes a certified local solid into the supplied orthonormal part frame, subtracts
    /// its bounding-box minimum and hashes its sorted surface triangles. Tessellation remains
    /// part of the signature; this is an evaluated-mesh identity, not a general solid isometry
    /// solver. A different digest falls back to the configured millimetre tolerance.
    pub fn from_solid(solid: &SolidMesh, frame: Frame3) -> Result<Self, NumberingError> {
        let fail = || NumberingError("numbering geometry is invalid or too large");
        let origin = solid.vertices().first().ok_or_else(fail)?;
        let axes = [
            frame.x().components(),
            frame.y().components(),
            frame.z().components(),
        ];
        let mut vertices = Vec::with_capacity(solid.vertices().len());
        let mut low = [f64::INFINITY; 3];
        for point in solid.vertices() {
            let delta = [
                point[0] - origin[0],
                point[1] - origin[1],
                point[2] - origin[2],
            ];
            let point =
                axes.map(|axis| (delta[0] * axis[0] + delta[1] * axis[1]) + delta[2] * axis[2]);
            for (low, value) in low.iter_mut().zip(point) {
                if !value.is_finite() {
                    return Err(fail());
                }
                *low = low.min(value);
            }
            vertices.push(point);
        }
        for point in &mut vertices {
            for (value, low) in point.iter_mut().zip(low) {
                // Canonical positive zero; no tolerance buckets, so nearby coordinates on
                // opposite sides of a rounding boundary can still compare within tolerance.
                *value = (*value - low) + 0.0;
            }
        }
        let mut triangles = Vec::with_capacity(solid.triangles().len());
        for indices in solid.triangles() {
            let mut points = [
                *vertices.get(indices[0] as usize).ok_or_else(fail)?,
                *vertices.get(indices[1] as usize).ok_or_else(fail)?,
                *vertices.get(indices[2] as usize).ok_or_else(fail)?,
            ];
            points.sort_by(point_order);
            triangles.push(points);
        }
        triangles.sort_by(triangle_order);
        let mut hash = Sha256::new();
        hash.update(b"BCNUM-GEOMETRY1\n").map_err(|_| fail())?;
        for value in triangles.iter().flatten().flatten() {
            hash.update(&value.to_bits().to_le_bytes())
                .map_err(|_| fail())?;
        }
        Ok(Self {
            triangles,
            digest: hash.finalize(),
        })
    }

    /// The full digest of the normalized evaluated geometry.
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }

    pub(super) fn equivalent(&self, other: &Self, tolerance_mm: f64) -> bool {
        self.digest == other.digest
            || (self.triangles.len() == other.triangles.len()
                && self
                    .triangles
                    .iter()
                    .flatten()
                    .flatten()
                    .zip(other.triangles.iter().flatten().flatten())
                    .all(|(a, b)| super::close(*a, *b, tolerance_mm)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_kernel::trimesh::TriMesh;
    use beamcraft_kernel::{RectangularPlate, rectangular_plate};

    #[test]
    fn geometry_hash_ignores_mesh_numbering_and_translation_but_binds_shape() {
        let box_body = rectangular_plate(RectangularPlate {
            width_mm: 100.0,
            length_mm: 200.0,
            thickness_mm: 10.0,
        })
        .unwrap();
        let solid = SolidMesh::from_box(&box_body);
        let expected = PartGeometry::plate(&solid).unwrap();
        let vertices: Vec<_> = solid
            .vertices()
            .iter()
            .rev()
            .map(|[x, y, z]| [x + 1200.0, y - 800.0, z + 300.0])
            .collect();
        let last = (vertices.len() - 1) as u32;
        let triangles: Vec<_> = solid
            .triangles()
            .iter()
            .rev()
            .map(|[a, b, c]| [last - c, last - a, last - b])
            .collect();
        let mesh = TriMesh::from_buffers(&vertices, &triangles).unwrap();
        let reordered = SolidMesh::composed(&mesh, solid.volume_mm3()).unwrap();
        let actual = PartGeometry::plate(&reordered).unwrap();
        assert_eq!(expected.digest(), actual.digest());
        assert!(expected.equivalent(&actual, 0.0));
        let changed = rectangular_plate(RectangularPlate {
            width_mm: 100.0,
            length_mm: 200.3,
            thickness_mm: 10.0,
        })
        .unwrap();
        let changed = PartGeometry::plate(&SolidMesh::from_box(&changed)).unwrap();
        assert_ne!(expected.digest(), changed.digest());
        assert!(!expected.equivalent(&changed, 0.001));
        assert!(expected.equivalent(&changed, 0.5));
    }
}
