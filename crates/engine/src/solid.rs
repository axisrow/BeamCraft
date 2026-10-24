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
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_model::digest::{ContentDigest, Sha256};

/// An evaluated, certified body mesh in element-local millimetres.
#[derive(Debug, Clone, PartialEq)]
pub struct SolidMesh {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
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
            blob: result.blob().to_vec(),
            digest: result.mesh_digest(),
            volume_mm3: result.volume_mm3(),
        }
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
