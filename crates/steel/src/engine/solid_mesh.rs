// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The meshes of the steel bodies: a member of a steel section, a ruled loft and a sweep, as
//! [`SolidMesh`].

use crate::kernel::ruled::LoftResult;
use crate::kernel::section::SectionResult;
use crate::kernel::sweep::SweepResult;
use beamcraft_engine::solid::SolidMesh;

/// The mesh of a straight member of an A07.1 section; the volume is the analytic
/// `net_area * axis_length` of the extruded outline.
pub fn from_section(result: &SectionResult) -> SolidMesh {
    let mesh = result.body().mesh();
    SolidMesh::from_parts(
        mesh.vertices().to_vec(),
        mesh.triangles().to_vec(),
        result.body().face_table().to_vec(),
        result.blob().to_vec(),
        result.mesh_digest(),
        result.volume_mm3(),
    )
}

/// The mesh of a G13 ruled loft (an A11 lofted plate); the volume is the midpoint of the
/// enclosure of the certified signed-volume sum.
pub fn from_loft(result: &LoftResult) -> SolidMesh {
    SolidMesh::from_parts(
        result.body().mesh().vertices().to_vec(),
        result.body().mesh().triangles().to_vec(),
        result.body().face_table().to_vec(),
        result.blob().to_vec(),
        result.mesh_digest(),
        result.volume_mm3().midpoint(),
    )
}

/// The mesh of a G12 sweep (a P12 polybeam); the volume is the midpoint of the enclosure of the
/// certified signed-volume sum, which the seal checked against the analytic truncated prism
/// value.
pub fn from_sweep(result: &SweepResult) -> SolidMesh {
    let mesh = result.body().mesh();
    SolidMesh::from_parts(
        mesh.vertices().to_vec(),
        mesh.triangles().to_vec(),
        result.body().face_table().to_vec(),
        result.blob().to_vec(),
        result.mesh_digest(),
        result.volume_mm3().midpoint(),
    )
}
