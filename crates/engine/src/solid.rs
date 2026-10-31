// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The sealed mesh of any evaluated body, whatever the kernel operation that produced it.
//!
//! A box (plate or member) is a [`PlateResult`]; a plate with through holes is the certified
//! result of the G09 hole operation. Both are BCMESH01 blobs with a digest, vertices, triangles
//! and an analytic volume, and everything downstream (derived USD mesh, view snapshot, quantities)
//! needs only that.

use crate::scene::SceneMesh;
use beamcraft_geom::{Frame3, Point3, RigidTransform, Vec3};
use beamcraft_kernel::PlateResult;
use beamcraft_kernel::ops::OpResult;
use beamcraft_kernel::profile::Extrusion;
use beamcraft_kernel::section::SectionResult;
use beamcraft_kernel::sweep::SweepResult;
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_model::digest::{ContentDigest, Sha256};

/// An evaluated, certified body mesh in element-local millimetres.
#[derive(Debug, Clone, PartialEq)]
pub struct SolidMesh {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    face_table: Vec<String>,
    blob: Vec<u8>,
    digest: ContentDigest,
    volume_mm3: f64,
}

impl SolidMesh {
    /// The mesh of a sealed box.
    pub fn from_box(result: &PlateResult) -> Self {
        let mesh = result.mesh();
        Self {
            vertices: mesh.vertices().iter().map(|p| [p.x, p.y, p.z]).collect(),
            triangles: mesh.triangles().to_vec(),
            face_table: mesh
                .face_table()
                .iter()
                .map(|id| (*id).to_owned())
                .collect(),
            blob: mesh.blob().to_vec(),
            digest: mesh.mesh_digest(),
            volume_mm3: result.volume_mm3(),
        }
    }

    /// The mesh of a certified hole operation. The volume is the midpoint of the enclosure of
    /// the exact signed-volume sum, whose width is rounding only.
    pub fn from_operation(result: &OpResult) -> Self {
        let mesh = result.body().mesh();
        Self {
            vertices: mesh.vertices().to_vec(),
            triangles: mesh.triangles().to_vec(),
            face_table: result.body().face_table().to_vec(),
            blob: result.blob().to_vec(),
            digest: result.mesh_digest(),
            volume_mm3: result.volume_mm3().midpoint(),
        }
    }

    /// The mesh of a sealed G05 extrusion of a plate contour. The volume is the analytic
    /// `net_area * thickness` of the polygonal solid. `None` when the blob is too long to hash.
    pub fn from_extrusion(extrusion: &Extrusion) -> Option<Self> {
        let mesh = extrusion.mesh();
        Some(Self {
            vertices: mesh.vertices().iter().map(|p| [p.x, p.y, p.z]).collect(),
            triangles: mesh.triangles().to_vec(),
            face_table: mesh.face_table().to_vec(),
            blob: mesh.blob().to_vec(),
            digest: Sha256::digest(mesh.blob()).ok()?,
            volume_mm3: extrusion.volume_mm3(),
        })
    }

    /// The mesh of a straight member of an A07.1 section; the volume is the analytic
    /// `net_area * axis_length` of the extruded outline.
    pub fn from_section(result: &SectionResult) -> Self {
        let mesh = result.body().mesh();
        Self {
            vertices: mesh.vertices().to_vec(),
            triangles: mesh.triangles().to_vec(),
            face_table: result.body().face_table().to_vec(),
            blob: result.blob().to_vec(),
            digest: result.mesh_digest(),
            volume_mm3: result.volume_mm3(),
        }
    }

    /// The mesh of a G12 sweep (a P12 polybeam); the volume is the midpoint of the enclosure of
    /// the certified signed-volume sum, which the seal checked against the analytic truncated
    /// prism value.
    pub fn from_sweep(result: &SweepResult) -> Self {
        let mesh = result.body().mesh();
        Self {
            vertices: mesh.vertices().to_vec(),
            triangles: mesh.triangles().to_vec(),
            face_table: result.body().face_table().to_vec(),
            blob: result.blob().to_vec(),
            digest: result.mesh_digest(),
            volume_mm3: result.volume_mm3().midpoint(),
        }
    }

    /// The mesh of an engine-composed solid (A15): an admitted, placed triangle set the kernel
    /// verified segment by segment, with the summed analytic volume. It has no kernel face
    /// table; a weld solid never reaches the scene, which is the only consumer of one. `None`
    /// when the merged mesh fails admission or the blob is too long to hash.
    pub fn composed(mesh: &TriMesh, volume_mm3: f64) -> Option<Self> {
        let blob = mesh.to_blob();
        Some(Self {
            vertices: mesh.vertices().to_vec(),
            triangles: mesh.triangles().to_vec(),
            face_table: Vec::new(),
            blob: blob.clone(),
            digest: Sha256::digest(&blob).ok()?,
            volume_mm3,
        })
    }

    /// The mesh of a G13 ruled loft (an A11 lofted plate); the volume is the midpoint of the
    /// enclosure of the certified signed-volume sum.
    pub fn from_loft(result: &beamcraft_kernel::ruled::LoftResult) -> Self {
        Self {
            vertices: result.body().mesh().vertices().to_vec(),
            triangles: result.body().mesh().triangles().to_vec(),
            face_table: result.body().face_table().to_vec(),
            blob: result.blob().to_vec(),
            digest: result.mesh_digest(),
            volume_mm3: result.volume_mm3().midpoint(),
        }
    }

    /// The mesh of an admitted triangle set with the kernel face table of the body it
    /// certifies (A12.1): a plain plate body translated along the element-local Z. The volume
    /// is the caller's analytic value, which a translation preserves. `None` when the blob is
    /// too long to hash.
    pub fn from_admitted(mesh: &TriMesh, face_table: Vec<String>, volume_mm3: f64) -> Option<Self> {
        let blob = mesh.to_blob();
        Some(Self {
            vertices: mesh.vertices().to_vec(),
            triangles: mesh.triangles().to_vec(),
            face_table,
            blob: blob.clone(),
            digest: Sha256::digest(&blob).ok()?,
            volume_mm3,
        })
    }

    /// The mesh rotated by the rigid placement quaternion `(x, y, z, w)` about the element-local
    /// origin: the composed section frame of a placed member. The member frame is rolled first
    /// and the placement quaternion maps it, so the world map is `R_placement . (axis x up/roll)`.
    /// Triangles, winding, the face table and the analytic volume are invariant under a rotation;
    /// the vertices, the blob and the digest change. `None` when the rotated mesh fails the
    /// kernel's mesh admission: a quaternion admitted by `Placement::validate` preserves every
    /// norm and coordinate magnitude, so this does not fire for one.
    pub fn rotated_by(self, orientation_xyzw: [f64; 4]) -> Option<Self> {
        let axis = |v: [f64; 3]| {
            let r = rotate_vector(v, orientation_xyzw);
            Vec3::new(r[0], r[1], r[2]).ok()
        };
        let frame = Frame3::try_new(
            axis([1.0, 0.0, 0.0])?,
            axis([0.0, 1.0, 0.0])?,
            axis([0.0, 0.0, 1.0])?,
        )
        .ok()?;
        let transform = RigidTransform::from_body_frame(frame, Point3::ORIGIN).ok()?;
        let mesh = TriMesh::from_blob(&self.blob)
            .ok()?
            .placed(&transform)
            .ok()?;
        let blob = mesh.to_blob();
        Some(Self {
            vertices: mesh.vertices().to_vec(),
            triangles: self.triangles,
            face_table: self.face_table,
            digest: Sha256::digest(&blob).ok()?,
            blob,
            volume_mm3: self.volume_mm3,
        })
    }

    /// Vertex positions.
    pub fn vertices(&self) -> &[[f64; 3]] {
        &self.vertices
    }

    /// Triangles as vertex indices, counter-clockwise from outside.
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// The exact BCMESH01 blob.
    pub fn blob(&self) -> &[u8] {
        &self.blob
    }

    /// SHA-256 of the blob.
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }

    /// Volume in cubic millimetres.
    pub const fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }

    /// The mesh as the snapshot builder takes it.
    pub fn scene_mesh(&self) -> SceneMesh {
        SceneMesh::new(
            self.blob.clone(),
            self.digest,
            self.vertices
                .iter()
                .map(|[x, y, z]| Point3::new(*x, *y, *z))
                .collect(),
        )
        .with_face_table(self.face_table.clone())
    }
}

/// Rotates a vector by the rigid quaternion `(x, y, z, w)`; the quaternion is normalized first,
/// so any admitted (nonzero, finite) quaternion maps finite input to finite output.
pub(crate) fn rotate_vector(v: [f64; 3], q: [f64; 4]) -> [f64; 3] {
    let [x, y, z, w] = q;
    let [vx, vy, vz] = v;
    let norm = (((x * x + y * y) + z * z) + (w * w)).sqrt();
    let (x, y, z, w) = (x / norm, y / norm, z / norm, w / norm);
    [
        (1.0 - 2.0 * (y * y + z * z)) * vx
            + 2.0 * (x * y - z * w) * vy
            + 2.0 * (x * z + y * w) * vz,
        2.0 * (x * y + z * w) * vx
            + (1.0 - 2.0 * (x * x + z * z)) * vy
            + 2.0 * (y * z - x * w) * vz,
        2.0 * (x * z - y * w) * vx
            + 2.0 * (y * z + x * w) * vy
            + (1.0 - 2.0 * (x * x + y * y)) * vz,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_kernel::ops::{Body, HoleCutter, through_hole};
    use beamcraft_kernel::profile::{Cavity, Point2, extrude};

    #[test]
    fn contour_and_operation_roles_survive_scene_conversion_and_rotation() {
        let outer = [
            Point2 { x: 0.0, y: 0.0 },
            Point2 { x: 200.0, y: 0.0 },
            Point2 { x: 0.0, y: 200.0 },
        ];
        let extrusion = extrude(&outer, &[], 10.0).unwrap();
        let solid = SolidMesh::from_extrusion(&extrusion).unwrap();
        assert_eq!(
            solid.scene_mesh().face_table.as_ref(),
            extrusion.mesh().face_table()
        );
        assert!(
            solid
                .scene_mesh()
                .face_table
                .iter()
                .any(|id| id == "extrusion:f.o0")
        );
        let cavity = Cavity::polygon(vec![
            Point2 { x: 20.0, y: 20.0 },
            Point2 { x: 20.0, y: 40.0 },
            Point2 { x: 40.0, y: 20.0 },
        ]);
        let result = through_hole(
            &Body::from_extrusion(&extrusion).unwrap(),
            &HoleCutter::new(cavity, -10.0, 20.0, 7),
        )
        .unwrap();
        let solid = SolidMesh::from_operation(&result);
        let expected = result.body().face_table();
        assert_eq!(solid.scene_mesh().face_table.as_ref(), expected);
        assert!(expected.iter().any(|id| id == "cut:f.h7.s0"));
        let half = core::f64::consts::FRAC_1_SQRT_2;
        let rotated = solid.rotated_by([0.0, 0.0, half, half]).unwrap();
        assert_eq!(rotated.scene_mesh().face_table.as_ref(), expected);
    }
}
