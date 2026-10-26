// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Fixed F06 section 6 plate layout, with bounded self-checks and encoding.

use crate::diagnostic::{Code, Failure, Field, Stage};

use beamcraft_geom::{Point3, Vec3};
use beamcraft_model::digest::{ContentDigest, Sha256};

/// Canonical counter-clockwise triangles, two per face in F06 table order.
pub const PLATE_TRIANGLES: [[u32; 3]; 12] = [
    [0, 4, 6],
    [0, 6, 2],
    [1, 3, 7],
    [1, 7, 5],
    [0, 1, 5],
    [0, 5, 4],
    [2, 6, 7],
    [2, 7, 3],
    [0, 2, 3],
    [0, 3, 1],
    [4, 5, 7],
    [4, 7, 6],
];
/// Snapshot-local mapping from each triangle to its semantic face table entry.
pub const PLATE_TRIANGLE_FACES: [u16; 12] = [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5];

/// Sealed standard plate mesh; counts and memory use are independent of input.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateMesh {
    pub(crate) vertices: [Point3; 8],
    pub(crate) face_ids: &'static [&'static str; 6],
    pub(crate) normals: [Vec3; 6],
    pub(crate) blob: [u8; 392],
    pub(crate) digest: ContentDigest,
}

impl PlateMesh {
    /// Eight finite element-local positions in role-bit order.
    pub const fn vertices(&self) -> &[Point3; 8] {
        &self.vertices
    }
    /// Twelve triangles; indices are snapshot-local, never feature addresses.
    pub const fn triangles(&self) -> &[[u32; 3]; 12] {
        &PLATE_TRIANGLES
    }
    /// Per-triangle face indices into [`Self::face_table`].
    pub const fn triangle_faces(&self) -> &[u16; 12] {
        &PLATE_TRIANGLE_FACES
    }
    /// Six stable local face ids in canonical order.
    pub const fn face_table(&self) -> &[&'static str; 6] {
        self.face_ids
    }
    /// Six outward unit normals in face-table order, separate from the blob.
    pub const fn face_normals(&self) -> &[Vec3; 6] {
        &self.normals
    }
    /// Explicit USD-compatible subdivision scheme; never rely on a default.
    pub const fn subdivision_scheme(&self) -> &'static str {
        "none"
    }
    /// Front faces have counter-clockwise winding seen from outside.
    pub const fn orientation(&self) -> &'static str {
        "rightHanded"
    }
    /// Exact 392-byte BCMESH01 element-local buffer, little-endian, layout 1.
    pub const fn blob(&self) -> &[u8; 392] {
        &self.blob
    }
    /// SHA-256 of the exact blob, distinct from a future shape key.
    pub const fn mesh_digest(&self) -> ContentDigest {
        self.digest
    }
}

pub(crate) fn self_check(
    vertices: &[Point3; 8],
    triangles: &[[u32; 3]; 12],
    face_normals: &[Vec3; 6],
) -> Result<(), Failure> {
    let fail = || Failure::one(Code::SelfCheckFailed, Field::Mesh, Stage::Checks);
    let mut volume_sum = 0.0;
    let mut unique_edges = 0usize;
    let mut used = [false; 8];
    for (triangle, expected_normal) in triangles
        .iter()
        .zip(face_normals.iter().flat_map(|n| [*n, *n]))
    {
        let [ia, ib, ic] = *triangle;
        if ia == ib || ib == ic || ia == ic {
            return Err(fail());
        }
        let point = |i: u32| -> Result<Point3, Failure> {
            let i = usize::try_from(i).map_err(|_| fail())?;
            vertices
                .get(i)
                .copied()
                .ok_or_else(fail)?
                .validated()
                .map_err(|_| fail())
        };
        let (a, b, c) = (point(ia)?, point(ib)?, point(ic)?);
        for i in triangle {
            let slot = used
                .get_mut(usize::try_from(*i).map_err(|_| fail())?)
                .ok_or_else(fail)?;
            *slot = true;
        }
        let cross = a
            .vector_to(b)
            .and_then(|ab| ab.cross(a.vector_to(c)?))
            .map_err(|_| fail())?;
        if cross.dot(expected_normal).map_err(|_| fail())? <= 0.0 {
            return Err(fail());
        }
        // Relative to the first vertex: the closed-surface volume does not depend on the origin,
        // and absolute coordinates far from zero would cancel catastrophically.
        let origin = vertices.first().copied().ok_or_else(fail)?;
        let av = origin.vector_to(a).map_err(|_| fail())?;
        let bv = origin.vector_to(b).map_err(|_| fail())?;
        let cv = origin.vector_to(c).map_err(|_| fail())?;
        volume_sum += av
            .dot(bv.cross(cv).map_err(|_| fail())?)
            .map_err(|_| fail())?;
    }
    // 36 directed occurrences; at most 36*36 comparisons, no recursion/allocation.
    for (a, b) in edges(triangles) {
        let mut forward = 0;
        let mut backward = 0;
        for (c, d) in edges(triangles) {
            forward += usize::from(a == c && b == d);
            backward += usize::from(a == d && b == c);
        }
        if forward != 1 || backward != 1 {
            return Err(fail());
        }
        if a < b {
            unique_edges += 1;
        }
    }
    if !used.into_iter().all(|u| u)
        || unique_edges != 18
        || 8 + 12 != unique_edges + 2
        || !volume_sum.is_finite()
        || (1.0 / 6.0) * volume_sum <= 0.0
    {
        return Err(fail());
    }
    Ok(())
}

fn edges(triangles: &[[u32; 3]; 12]) -> impl Iterator<Item = (u32, u32)> + '_ {
    triangles
        .iter()
        .flat_map(|[a, b, c]| [(*a, *b), (*b, *c), (*c, *a)])
}

pub(crate) fn encode(vertices: &[Point3; 8]) -> Result<([u8; 392], ContentDigest), Failure> {
    let fail = || Failure::one(Code::SelfCheckFailed, Field::Mesh, Stage::Build);
    let mut blob = [0u8; 392];
    let header = *b"BCMESH01";
    let bytes = header
        .into_iter()
        .chain(
            [1u32, 8, 12, 6, 0, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes),
        )
        .chain(vertices.iter().flat_map(|p| [p.x, p.y, p.z]).flat_map(|v| {
            let v = if v == 0.0 { 0.0 } else { v };
            v.to_le_bytes()
        }))
        .chain(
            PLATE_TRIANGLES
                .into_iter()
                .flatten()
                .flat_map(u32::to_le_bytes),
        )
        .chain(PLATE_TRIANGLE_FACES.into_iter().flat_map(u16::to_le_bytes));
    let mut written = 0usize;
    for byte in bytes {
        *blob.get_mut(written).ok_or_else(fail)? = byte;
        written += 1;
    }
    if written != blob.len() {
        return Err(fail());
    }
    let digest = Sha256::digest(&blob).map_err(|_| fail())?;
    Ok((blob, digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RectangularPlate, rectangular_plate};

    #[test]
    fn corrupted_builder_candidates_never_seal() {
        let mesh = rectangular_plate(RectangularPlate {
            width_mm: 10.0,
            length_mm: 20.0,
            thickness_mm: 1.0,
        })
        .unwrap();
        let vertices = mesh.mesh().vertices();
        let normals = *mesh.mesh().face_normals();
        for mutation in 0..4 {
            let mut triangles = PLATE_TRIANGLES;
            match mutation {
                0 => triangles[0].swap(1, 2),     // F06-NEG-20 inward face.
                1 => triangles[0] = triangles[1], // F06-NEG-18 missing triangle/closure.
                2 => triangles[0][0] = 8,
                _ => triangles[0][0] = triangles[0][1],
            }
            assert_eq!(
                self_check(vertices, &triangles, &normals)
                    .unwrap_err()
                    .diagnostics()
                    .next()
                    .unwrap()
                    .code,
                Code::SelfCheckFailed
            );
        }
        let mut bad = *vertices;
        bad[7].z = 0.0;
        assert!(self_check(&bad, &PLATE_TRIANGLES, &normals).is_err());
    }
}
