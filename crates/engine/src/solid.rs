// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The sealed mesh of any evaluated body, whatever the kernel operation that produced it.
//!
//! A box (plate or member) is a [`PlateResult`]; a contour plate is a sealed G05 extrusion; a
//! plate with through holes is the certified result of the G09 hole operation. All are BCMESH01
//! blobs with a digest, vertices, triangles and an analytic volume, and everything downstream
//! (derived USD mesh, view snapshot, quantities) needs only that.

use crate::scene::SceneMesh;
use beamcraft_geom::Point3;
use beamcraft_kernel::PlateResult;
use beamcraft_kernel::ops::OpResult;
use beamcraft_kernel::profile::Extrusion;
use beamcraft_kernel::section::SectionResult;
use beamcraft_model::digest::ContentDigest;

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

    /// The mesh of a sealed G05 extrusion (a contour plate): the extrusion's own exact blob and
    /// digest, its vertices and triangles, and the analytic `net_area * thickness` volume. The
    /// caller certifies the same shell with G06 (`Body::from_extrusion`) before storing it.
    pub fn from_extrusion(extrusion: &Extrusion) -> Self {
        let mesh = extrusion.mesh();
        Self {
            vertices: mesh.vertices().iter().map(|p| [p.x, p.y, p.z]).collect(),
            triangles: mesh.triangles().to_vec(),
            blob: mesh.blob().to_vec(),
            digest: mesh.mesh_digest(),
            volume_mm3: extrusion.volume_mm3(),
        }
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
