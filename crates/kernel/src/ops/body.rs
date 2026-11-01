// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The certified solid every pilot operation takes and returns.
//!
//! A [`Body`] is admitted once through the G06 shell certification (closed, 2-manifold,
//! consistently oriented, outward, non-degenerate, free of self-intersection, one shell) and
//! is immutable afterwards, so an operation never has to re-prove its input. It carries the
//! face table (local feature ids and outward normals) that travels beside the BCMESH01 blob.

use super::diagnostic::{OpCode, OpDefect, OpFailure, OpField};
use crate::PlateResult;
use crate::diagnostic::Stage;
use crate::enclosure::Enclosure;
use crate::measure::measure;
use crate::profile::{Extrusion, FaceRole};
use crate::topology::{Analysis, Limits, analyze_with_tolerance};
use crate::trimesh::{Space, TriMesh};
use beamcraft_geom::Vec3;
use beamcraft_geom::tolerance::Tolerance;

/// Longest local feature id: F06 section 5 allows 64 bytes for `fid1:` + a 36-byte ElementId +
/// `:` + the local id, which leaves 22 bytes.
pub const MAX_LOCAL_ID_BYTES: usize = 22;

/// Largest operation ordinal; keeps `cut:f.h<ordinal>.s<k>` inside [`MAX_LOCAL_ID_BYTES`].
pub const MAX_ORDINAL: u16 = 999;

/// Smallest cosine between a supplied face normal and the face's area vector (about 0.08 degrees).
const NORMAL_AGREEMENT: f64 = 0.999_999;

/// A certified, immutable solid with its face table.
#[derive(Debug, Clone)]
pub struct Body {
    mesh: TriMesh,
    face_ids: Vec<String>,
    normals: Vec<Option<[f64; 3]>>,
    tolerance: Tolerance,
    analysis: Analysis,
    volume_mm3: Enclosure,
}

pub(crate) fn valid_local_id(id: &str) -> bool {
    let mut parts = id.split(':');
    let (Some(kind), Some(token), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    id.len() <= MAX_LOCAL_ID_BYTES
        && !kind.is_empty()
        && !token.is_empty()
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b':' | b'.' | b'-')
        })
}

fn table_error(defect: OpDefect, code: OpCode) -> OpFailure {
    OpFailure::new(code, OpField::FaceTable, defect, Stage::Validation)
}

impl Body {
    /// Admits a mesh with its face table.
    ///
    /// `face_ids` must hold one distinct local id (`kind:token`, characters `a-z0-9:.-`, at
    /// most [`MAX_LOCAL_ID_BYTES`] bytes) per entry of the mesh's face table, each used by at
    /// least one triangle; `normals` one optional finite outward unit normal per face (`None`
    /// for a curved face). `tolerance` is the body's own F06 scale when known (a plate or an
    /// extrusion supplies it); `None` derives the generic mesh rule of [`TriMesh::tolerance`].
    ///
    /// The mesh is certified with the default G06 limits; anything but a certified single
    /// shell is refused with the shell's own code. Admission has no cancellation point:
    /// certify first with [`crate::topology::analyze_with`] if a host needs one.
    pub fn from_mesh(
        mesh: TriMesh,
        face_ids: Vec<String>,
        normals: Vec<Option<[f64; 3]>>,
        tolerance: Option<Tolerance>,
    ) -> Result<Self, OpFailure> {
        let faces = usize::try_from(mesh.face_count())
            .map_err(|_| table_error(OpDefect::FaceTableSize, OpCode::BlobInvalid))?;
        // Operations seal element-local blobs without converting coordinates, so a world-space
        // mesh would be silently relabelled as local.
        if mesh.space() != Space::Local {
            return Err(OpFailure::new(
                OpCode::BlobInvalid,
                OpField::Body,
                OpDefect::BodySpace,
                Stage::Validation,
            ));
        }
        if face_ids.len() != faces || normals.len() != faces {
            return Err(table_error(OpDefect::FaceTableSize, OpCode::BlobInvalid));
        }
        if face_ids.iter().any(|id| !valid_local_id(id)) {
            return Err(table_error(OpDefect::FaceTableEntry, OpCode::BlobInvalid));
        }
        let mut sorted: Vec<&str> = face_ids.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        if sorted.windows(2).any(|w| w.first() == w.get(1)) {
            return Err(table_error(OpDefect::FaceTableSize, OpCode::BlobInvalid));
        }
        let finite_unit = |n: &[f64; 3]| {
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            n.iter().all(|v| v.is_finite()) && (length - 1.0).abs() <= 1e-9
        };
        if normals.iter().flatten().any(|n| !finite_unit(n)) {
            return Err(table_error(OpDefect::FaceTableEntry, OpCode::BlobInvalid));
        }
        let mut used = vec![false; faces];
        for &f in mesh.faces() {
            if let Some(slot) = used.get_mut(usize::from(f)) {
                *slot = true;
            }
        }
        if used.iter().any(|u| !u) {
            return Err(table_error(OpDefect::FaceTableSize, OpCode::BlobInvalid));
        }
        let tolerance = match tolerance {
            Some(t) => t,
            None => mesh
                .tolerance()
                .map_err(|e| OpFailure::from_mesh(e, OpField::Body))?,
        };
        let analysis = analyze_with_tolerance(&mesh, tolerance, Limits::DEFAULT, |_| false)
            .map_err(|e| OpFailure::from_mesh(e, OpField::Body))?;
        if let Some(code) = analysis.status().code() {
            return Err(OpFailure::new(
                OpCode::from(code),
                OpField::Body,
                OpDefect::None,
                Stage::Checks,
            ));
        }
        if analysis.shells().len() != 1 {
            return Err(OpFailure::unsupported(
                OpField::Body,
                OpDefect::BodyShells,
                Stage::Checks,
            ));
        }
        check_normals(&mesh, &normals)?;
        let measurement =
            measure(&mesh, &analysis).map_err(|e| OpFailure::from_mesh(e, OpField::Body))?;
        let volume_mm3 = measurement.volume_mm3().ok_or_else(|| {
            OpFailure::numeric(OpField::Body, OpDefect::NumericRange, Stage::Quantities)
        })?;
        Ok(Self {
            mesh,
            face_ids,
            normals,
            tolerance,
            analysis,
            volume_mm3,
        })
    }

    /// Admits a sealed G02 plate with its six local ids and outward normals.
    pub fn from_plate(plate: &PlateResult) -> Result<Self, OpFailure> {
        let mesh = TriMesh::from_blob(plate.mesh().blob())
            .map_err(|e| OpFailure::from_mesh(e, OpField::Body))?;
        let ids = plate.mesh().face_table().iter().map(|s| (*s).to_owned());
        let normals = plate.mesh().face_normals().iter().map(|n| Some(unit(*n)));
        Self::from_mesh(
            mesh,
            ids.collect(),
            normals.collect(),
            Some(plate.tolerance()),
        )
    }

    /// Admits a sealed G05 extrusion with its provisional `extrusion:` ids and normals.
    pub fn from_extrusion(extrusion: &Extrusion) -> Result<Self, OpFailure> {
        let mesh = TriMesh::from_blob(extrusion.mesh().blob())
            .map_err(|e| OpFailure::from_mesh(e, OpField::Body))?;
        // The roles guarantee a curved wall has no normal; anything else must have one.
        let consistent = extrusion
            .mesh()
            .face_roles()
            .iter()
            .zip(extrusion.mesh().face_normals())
            .all(|(role, n)| {
                n.is_none() == matches!(role, FaceRole::CavityWall { edge: None, .. })
            });
        if !consistent {
            return Err(table_error(OpDefect::FaceTableEntry, OpCode::BlobInvalid));
        }
        Self::from_mesh(
            mesh,
            extrusion.mesh().face_table().to_vec(),
            extrusion
                .mesh()
                .face_normals()
                .iter()
                .map(|n| n.map(unit))
                .collect(),
            Some(extrusion.tolerance()),
        )
    }

    pub(crate) fn from_sealed(
        mesh: TriMesh,
        face_ids: Vec<String>,
        normals: Vec<Option<[f64; 3]>>,
        tolerance: Tolerance,
        analysis: Analysis,
        volume_mm3: Enclosure,
    ) -> Self {
        Self {
            mesh,
            face_ids,
            normals,
            tolerance,
            analysis,
            volume_mm3,
        }
    }

    /// The certified mesh (element-local space).
    pub const fn mesh(&self) -> &TriMesh {
        &self.mesh
    }

    /// Local face ids in face-table order, one per face of the mesh.
    pub fn face_table(&self) -> &[String] {
        &self.face_ids
    }

    /// Outward unit normal per face, `None` for a curved face.
    pub fn face_normals(&self) -> &[Option<[f64; 3]>] {
        &self.normals
    }

    /// The body's F06 local-size policy, reused by every result derived from it.
    pub const fn tolerance(&self) -> Tolerance {
        self.tolerance
    }

    /// The G06 certificate of the mesh.
    pub const fn analysis(&self) -> &Analysis {
        &self.analysis
    }

    /// Enclosure of the mesh volume in mm3 (exact signed-volume sum, enclosed once).
    pub const fn volume_mm3(&self) -> Enclosure {
        self.volume_mm3
    }

    /// The canonical BCMESH01 blob of the mesh.
    pub fn to_blob(&self) -> Vec<u8> {
        self.mesh.to_blob()
    }
}

pub(crate) fn unit(n: Vec3) -> [f64; 3] {
    [n.x(), n.y(), n.z()]
}

// A planar face's supplied normal must agree with the oriented triangles it labels: the area
// vector of the face (sum of the triangles' cross products, outward by the certified winding)
// has to point along it. A curved face has no normal and nothing to check.
fn check_normals(mesh: &TriMesh, normals: &[Option<[f64; 3]>]) -> Result<(), OpFailure> {
    let bad = || table_error(OpDefect::FaceTableEntry, OpCode::BlobInvalid);
    let mut sums = vec![[0.0f64; 3]; normals.len()];
    for (index, face) in mesh.faces().iter().enumerate() {
        let [a, b, c] = mesh.triangle_points(index).ok_or_else(bad)?;
        let (u, w) = (
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
            [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
        );
        let cross = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        let sum = sums.get_mut(usize::from(*face)).ok_or_else(bad)?;
        for (s, v) in sum.iter_mut().zip(cross) {
            *s += v;
        }
    }
    for (normal, sum) in normals.iter().zip(&sums) {
        let Some(n) = normal else { continue };
        let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
        let along = sum[0] * n[0] + sum[1] * n[1] + sum[2] * n[2];
        if !(length > 0.0 && along >= NORMAL_AGREEMENT * length) {
            return Err(bad());
        }
    }
    Ok(())
}
