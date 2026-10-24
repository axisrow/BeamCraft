// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Validated triangle meshes of arbitrary origin: BCMESH01 blobs and buffers.
//!
//! This is the structural gate (F06 sections 6 and 10) in front of the shell
//! checks. A [`TriMesh`] is finite, within the hard caps, with every index in
//! range; it is not thereby closed, oriented or an engineering result.

use crate::diagnostic::Stage;
use crate::verdict::{Fault, MeshCode, MeshError};
use beamcraft_geom::tolerance::Tolerance;
use beamcraft_geom::{Point3, RigidTransform};

/// Vertices per body (F06 section 6).
pub const MAX_VERTICES: usize = 262_144;
/// Triangles per body.
pub const MAX_TRIANGLES: usize = 524_288;
/// Face-table entries per body.
pub const MAX_FACE_TABLE: usize = 65_535;
/// Shells per body.
pub const MAX_SHELLS: usize = 16;
/// Mesh blob size: `32 + 24*262144 + pad8(12*524288) + pad8(2*524288)`.
pub const MAX_BLOB_BYTES: usize = 13_631_520;
/// Largest absolute coordinate, the F06 section 9 world-coordinate bound.
pub const MAX_COORDINATE_MM: f64 = 4_000_000_000.0;

const HEADER_BYTES: usize = 32;
const MAGIC: &[u8; 8] = b"BCMESH01";

/// Coordinate space recorded in a blob header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    /// Element-local space (header value 0).
    Local,
    /// World space (header value 1).
    World,
}

/// A structurally valid triangle mesh with at most [`MAX_VERTICES`] vertices.
///
/// Winding is counter-clockwise seen from outside (right-handed). Negative
/// zero is canonicalized on construction from buffers and rejected in blobs.
#[derive(Debug, Clone, PartialEq)]
pub struct TriMesh {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    faces: Vec<u16>,
    face_count: u32,
    space: Space,
}

const fn err(code: MeshCode, fault: Fault) -> MeshError {
    MeshError::new(code, fault, Stage::Validation)
}

fn pad8(n: u64) -> u64 {
    n.div_ceil(8) * 8
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn le_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn le_f64(bytes: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_le_bytes(
        bytes.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

impl TriMesh {
    /// Validates vertex and index buffers (face table: one entry, all triangles on face 0).
    ///
    /// Order: counts against caps, finiteness, coordinate caps, index range.
    /// Empty buffers are rejected. Negative zero is normalized.
    pub fn from_buffers(vertices: &[[f64; 3]], triangles: &[[u32; 3]]) -> Result<Self, MeshError> {
        if vertices.len() > MAX_VERTICES {
            return Err(err(MeshCode::BufferLimit, Fault::Vertices));
        }
        if triangles.len() > MAX_TRIANGLES {
            return Err(err(MeshCode::BufferLimit, Fault::Triangles));
        }
        if vertices.is_empty() {
            return Err(err(MeshCode::BlobInvalid, Fault::Vertices));
        }
        if triangles.is_empty() {
            return Err(err(MeshCode::BlobInvalid, Fault::Triangles));
        }
        if vertices.iter().flatten().any(|v| !v.is_finite()) {
            return Err(err(MeshCode::NonFinite, Fault::Coordinates));
        }
        if vertices
            .iter()
            .flatten()
            .any(|v| v.abs() > MAX_COORDINATE_MM)
        {
            return Err(err(MeshCode::AboveLimit, Fault::Coordinates));
        }
        let count = u32::try_from(vertices.len())
            .map_err(|_| err(MeshCode::BufferLimit, Fault::Vertices))?;
        if triangles.iter().flatten().any(|i| *i >= count) {
            return Err(err(MeshCode::BlobInvalid, Fault::Indices));
        }
        Ok(Self {
            vertices: vertices
                .iter()
                .map(|p| p.map(|v| if v == 0.0 { 0.0 } else { v }))
                .collect(),
            triangles: triangles.to_vec(),
            faces: vec![0; triangles.len()],
            face_count: 1,
            space: Space::Local,
        })
    }

    /// Validates a BCMESH01 blob (F06 section 6) before reading any section.
    ///
    /// The supplied length is capped first; counts are compared with their caps
    /// before any size is computed (checked 64-bit arithmetic), and the supplied
    /// length must equal the computed length. Wrong magic, layout, reserved
    /// field, zero counts, nonzero padding, non-finite or negative-zero
    /// coordinates, out-of-range indices and face indices outside the table are
    /// `GEO_BLOB_INVALID`; counts above caps are `GEO_BUFFER_LIMIT` or
    /// `GEO_TOPOLOGY_LIMIT`.
    pub fn from_blob(bytes: &[u8]) -> Result<Self, MeshError> {
        if bytes.len() > MAX_BLOB_BYTES {
            return Err(err(MeshCode::BufferLimit, Fault::Bytes));
        }
        let invalid = |fault| err(MeshCode::BlobInvalid, fault);
        if bytes.len() < HEADER_BYTES || bytes.get(..8) != Some(MAGIC.as_slice()) {
            return Err(invalid(Fault::Header));
        }
        let field = |at| le_u32(bytes, at).ok_or_else(|| invalid(Fault::Header));
        let (layout, v, t, k, space, reserved) = (
            field(8)?,
            field(12)?,
            field(16)?,
            field(20)?,
            field(24)?,
            field(28)?,
        );
        if layout != 1 || reserved != 0 || space > 1 {
            return Err(invalid(Fault::Header));
        }
        if v == 0 {
            return Err(invalid(Fault::Vertices));
        }
        if t == 0 {
            return Err(invalid(Fault::Triangles));
        }
        if k == 0 {
            return Err(invalid(Fault::FaceTable));
        }
        let (vn, tn, kn) = (u64::from(v), u64::from(t), u64::from(k));
        if vn > MAX_VERTICES as u64 {
            return Err(err(MeshCode::BufferLimit, Fault::Vertices));
        }
        if tn > MAX_TRIANGLES as u64 {
            return Err(err(MeshCode::BufferLimit, Fault::Triangles));
        }
        if kn > MAX_FACE_TABLE as u64 {
            return Err(err(MeshCode::TopologyLimit, Fault::FaceTable));
        }
        let index_bytes = pad8(12 * tn);
        let total = 32 + 24 * vn + index_bytes + pad8(2 * tn);
        if u64::try_from(bytes.len()) != Ok(total) {
            return Err(invalid(Fault::Bytes));
        }
        let (vn, tn) = (
            usize::try_from(vn).map_err(|_| invalid(Fault::Header))?,
            usize::try_from(tn).map_err(|_| invalid(Fault::Header))?,
        );
        let mut vertices = Vec::with_capacity(vn);
        for i in 0..vn {
            let base = HEADER_BYTES + 24 * i;
            let mut p = [0.0; 3];
            for (axis, slot) in p.iter_mut().enumerate() {
                let value = le_f64(bytes, base + 8 * axis).ok_or_else(|| invalid(Fault::Bytes))?;
                if !value.is_finite() || (value == 0.0 && value.is_sign_negative()) {
                    return Err(invalid(Fault::Coordinates));
                }
                if value.abs() > MAX_COORDINATE_MM {
                    return Err(err(MeshCode::AboveLimit, Fault::Coordinates));
                }
                *slot = value;
            }
            vertices.push(p);
        }
        let index_base = HEADER_BYTES + 24 * vn;
        let mut triangles = Vec::with_capacity(tn);
        for t_index in 0..tn {
            let base = index_base + 12 * t_index;
            let mut tri = [0u32; 3];
            for (corner, slot) in tri.iter_mut().enumerate() {
                let i = le_u32(bytes, base + 4 * corner).ok_or_else(|| invalid(Fault::Bytes))?;
                if i >= v {
                    return Err(invalid(Fault::Indices));
                }
                *slot = i;
            }
            triangles.push(tri);
        }
        let face_base =
            index_base + usize::try_from(index_bytes).map_err(|_| invalid(Fault::Header))?;
        let mut faces = Vec::with_capacity(tn);
        for t_index in 0..tn {
            let f = le_u16(bytes, face_base + 2 * t_index).ok_or_else(|| invalid(Fault::Bytes))?;
            if u32::from(f) >= k {
                return Err(invalid(Fault::Faces));
            }
            faces.push(f);
        }
        let padding_nonzero = |from: usize, to: usize| {
            bytes
                .get(from..to)
                .is_none_or(|pad| pad.iter().any(|b| *b != 0))
        };
        if padding_nonzero(index_base + 12 * tn, face_base)
            || padding_nonzero(face_base + 2 * tn, bytes.len())
        {
            return Err(invalid(Fault::Faces));
        }
        Ok(Self {
            vertices,
            triangles,
            faces,
            face_count: k,
            space: if space == 0 {
                Space::Local
            } else {
                Space::World
            },
        })
    }

    /// Encodes the canonical BCMESH01 blob; `from_blob(to_blob())` reproduces `self`.
    pub fn to_blob(&self) -> Vec<u8> {
        let (v, t) = (self.vertices.len(), self.triangles.len());
        let mut out = Vec::with_capacity(HEADER_BYTES + 24 * v + 16 * t + 16);
        out.extend_from_slice(MAGIC);
        let counts = [
            1,
            u32::try_from(v).unwrap_or(u32::MAX),
            u32::try_from(t).unwrap_or(u32::MAX),
            self.face_count,
            u32::from(self.space == Space::World),
            0,
        ];
        for c in counts {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for x in self.vertices.iter().flatten() {
            out.extend_from_slice(&(if *x == 0.0 { 0.0 } else { *x }).to_le_bytes());
        }
        for i in self.triangles.iter().flatten() {
            out.extend_from_slice(&i.to_le_bytes());
        }
        out.resize(
            HEADER_BYTES + 24 * v + usize::try_from(pad8(12 * t as u64)).unwrap_or(0),
            0,
        );
        for f in &self.faces {
            out.extend_from_slice(&f.to_le_bytes());
        }
        let padded = out.len().div_ceil(8) * 8;
        out.resize(padded, 0);
        out
    }

    /// Vertex positions in millimetres.
    pub fn vertices(&self) -> &[[f64; 3]] {
        &self.vertices
    }

    /// Vertex index triples, counter-clockwise seen from outside.
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// Per-triangle face-table index.
    pub fn faces(&self) -> &[u16] {
        &self.faces
    }

    /// Declared face-table entry count.
    pub const fn face_count(&self) -> u32 {
        self.face_count
    }

    /// Coordinate space of the vertices.
    pub const fn space(&self) -> Space {
        self.space
    }

    /// The three corner positions of triangle `t`, or `None` when out of range.
    pub fn triangle_points(&self, t: usize) -> Option<[[f64; 3]; 3]> {
        let [a, b, c] = *self.triangles.get(t)?;
        let at = |i: u32| self.vertices.get(usize::try_from(i).ok()?).copied();
        Some([at(a)?, at(b)?, at(c)?])
    }

    /// Applies a rigid transform to every vertex, as F06 `world = R*local + t`.
    ///
    /// Triangle indices and winding are unchanged; the result is in world space.
    /// The map is the production [`RigidTransform`] evaluation order, whose
    /// frame certificate is approximate; this does not certify exact rigidity.
    pub fn transformed(&self, transform: &RigidTransform) -> Result<Self, MeshError> {
        let numeric = || MeshError::numeric(Stage::Validation);
        let mut vertices = Vec::with_capacity(self.vertices.len());
        for [x, y, z] in &self.vertices {
            let p = transform
                .apply_point(Point3::new(*x, *y, *z))
                .map_err(|_| numeric())?;
            let q = [p.x, p.y, p.z];
            if q.iter().any(|v| v.abs() > MAX_COORDINATE_MM) {
                return Err(err(MeshCode::AboveLimit, Fault::Coordinates));
            }
            vertices.push(q.map(|v| if v == 0.0 { 0.0 } else { v }));
        }
        Ok(Self {
            vertices,
            triangles: self.triangles.clone(),
            faces: self.faces.clone(),
            face_count: self.face_count,
            space: Space::World,
        })
    }

    /// Places an element-local body mesh inside its element by a rigid transform: the same map as
    /// [`Self::transformed`], but the result stays in element-local space, because the transform
    /// is part of building the body (a member's section frame), not a move to world coordinates.
    pub fn placed(&self, transform: &RigidTransform) -> Result<Self, MeshError> {
        let mut placed = self.transformed(transform)?;
        placed.space = self.space;
        Ok(placed)
    }

    /// FNV-1a over counts, coordinate bits and indices; ties analyses to a mesh.
    pub(crate) fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |word: u64| {
            for byte in word.to_le_bytes() {
                h ^= u64::from(byte);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        mix(self.vertices.len() as u64);
        mix(self.triangles.len() as u64);
        self.vertices
            .iter()
            .flatten()
            .for_each(|v| mix(v.to_bits()));
        self.triangles
            .iter()
            .flatten()
            .for_each(|i| mix(u64::from(*i)));
        h
    }

    /// F06 section 4 local scale and geometric floor for this mesh.
    ///
    /// The size is the plate rule's `D`: the largest distance from vertex 0,
    /// scanned in ascending order with `>`, evaluated by the checked G01 norm.
    /// A size above the plate cap (2,000,000 mm) is `GEO_ABOVE_LIMIT`.
    pub fn tolerance(&self) -> Result<Tolerance, MeshError> {
        let numeric = || MeshError::numeric(Stage::Validation);
        let origin = self
            .vertices
            .first()
            .map(|[x, y, z]| Point3::new(*x, *y, *z))
            .ok_or_else(numeric)?;
        let mut size = 0.0;
        for [x, y, z] in &self.vertices {
            let d = origin
                .checked_distance_to(Point3::new(*x, *y, *z))
                .map_err(|_| numeric())?;
            if d > size {
                size = d;
            }
        }
        let far = Point3::new(size, 0.0, 0.0);
        Tolerance::for_plate(&[Point3::ORIGIN, far, far]).map_err(|e| match e {
            beamcraft_geom::MathError::AboveLimit => err(MeshCode::AboveLimit, Fault::Coordinates),
            _ => numeric(),
        })
    }
}
