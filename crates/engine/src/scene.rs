// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The input of the snapshot builder: evaluated bodies and read-only reference representations.
//! The builder trusts neither the caller's `kind` nor any mesh: it re-validates every blob.

mod reference;
pub(crate) use reference::foreign_surface;
pub use reference::{ReferenceGeometry, reference_element};

use crate::element::{ElementId, ElementKind};
use beamcraft_geom::{Point3, Translation3};
use beamcraft_kernel::PlateResult;
use beamcraft_model::digest::ContentDigest;
use std::sync::Arc;

/// Transport bound for a single kernel-authored local face role.
pub(crate) const MAX_FACE_ROLE_BYTES: usize = 1024;

/// The mesh of one representation: the exact BCMESH01 blob, its digest and the vertex positions
/// (element-local millimetres) the bounds are computed from. Any sealed kernel body can be
/// offered, as can a display-only foreign reference mesh. The bytes and vertices are
/// shared, so elements that reuse one mesh (copies) hold it once in memory.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneMesh {
    /// The blob.
    pub blob: Arc<[u8]>,
    /// The digest.
    pub digest: ContentDigest,
    /// The vertices.
    pub vertices: Arc<[Point3]>,
    /// Stable local face roles kept beside the index-based BCMESH01 face table.
    pub face_table: Arc<[String]>,
}

impl SceneMesh {
    /// A mesh from its blob, digest and vertices. The snapshot builder re-validates the blob and
    /// checks that the digest is the blob's. Crate-private: outside the engine a mesh can only
    /// come from kernel results or the bounded reference conversion, so its vertices match its blob.
    pub(crate) fn new(blob: Vec<u8>, digest: ContentDigest, vertices: Vec<Point3>) -> Self {
        Self::shared(blob.into(), digest, vertices)
    }

    /// [`SceneMesh::new`] over bytes that are already shared (a validated bundle blob).
    pub(crate) fn shared(blob: Arc<[u8]>, digest: ContentDigest, vertices: Vec<Point3>) -> Self {
        Self {
            blob,
            digest,
            vertices: vertices.into(),
            face_table: Arc::from([]),
        }
    }

    /// Keeps kernel-authored semantic face roles beside the mesh buffer.
    #[must_use]
    pub(crate) fn with_face_table(mut self, face_table: Vec<String>) -> Self {
        self.face_table = face_table.into();
        self
    }
}

impl From<&PlateResult> for SceneMesh {
    fn from(result: &PlateResult) -> Self {
        let mesh = result.mesh();
        Self {
            blob: Arc::from(mesh.blob().as_slice()),
            digest: mesh.mesh_digest(),
            vertices: Arc::from(mesh.vertices().as_slice()),
            face_table: Arc::from(
                mesh.face_table()
                    .iter()
                    .map(|id| (*id).to_owned())
                    .collect::<Vec<_>>(),
            ),
        }
    }
}

impl From<PlateResult> for SceneMesh {
    fn from(result: PlateResult) -> Self {
        Self::from(&result)
    }
}

/// One element offered to the snapshot builder.
///
/// `full` is the representation with all operations applied. `simplified` is the optional
/// lightweight stand-in. When it is absent the part has no distinct simplified mesh and one mesh
/// serves both representations. A caller-supplied `simplified` is flagged not exact by the
/// snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneElement {
    /// The id.
    pub id: ElementId,
    /// The kind.
    pub kind: ElementKind,
    /// The placement.
    pub placement: Translation3,
    /// The orientation xyzw.
    pub orientation_xyzw: [f64; 4],
    /// The full.
    pub full: SceneMesh,
    /// The simplified.
    pub simplified: Option<SceneMesh>,
}

impl SceneElement {
    /// An element whose representation is `full`, placed by a translation (the only placement of
    /// the first scope).
    pub fn new(
        id: ElementId,
        kind: ElementKind,
        placement: Translation3,
        full: impl Into<SceneMesh>,
    ) -> Self {
        Self {
            id,
            kind,
            placement,
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            full: full.into(),
            simplified: None,
        }
    }

    /// Sets an admitted rigid orientation in quaternion `(x, y, z, w)` form.
    #[must_use]
    pub fn with_orientation(mut self, orientation_xyzw: [f64; 4]) -> Self {
        self.orientation_xyzw = orientation_xyzw;
        self
    }

    /// Adds a distinct simplified representation.
    #[must_use]
    pub fn with_simplified(mut self, simplified: impl Into<SceneMesh>) -> Self {
        self.simplified = Some(simplified.into());
        self
    }

    /// The element identity.
    pub const fn id(&self) -> ElementId {
        self.id
    }

    /// The element kind.
    pub const fn kind(&self) -> ElementKind {
        self.kind
    }
}
