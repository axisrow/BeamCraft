// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The input of the snapshot builder: evaluated bodies and read-only reference representations.
//! The builder trusts neither the caller's `kind` nor any mesh: it re-validates every blob.

mod reference;
pub use reference::{ReferenceGeometry, reference_element};

use crate::element::{ElementId, ElementKind};
use beamcraft_geom::{Point3, Translation3};
use beamcraft_kernel::PlateResult;
use beamcraft_model::digest::ContentDigest;
use std::sync::Arc;

/// The mesh of one representation: the exact BCMESH01 blob, its digest and the vertex positions
/// (element-local millimetres) the bounds are computed from. Any sealed kernel body can be
/// offered, as can a display-only foreign reference mesh. The bytes and vertices are
/// shared, so elements that reuse one mesh (copies) hold it once in memory.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneMesh {
    pub(crate) blob: Arc<[u8]>,
    pub(crate) digest: ContentDigest,
    pub(crate) vertices: Arc<[Point3]>,
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
        }
    }
}

impl From<&PlateResult> for SceneMesh {
    fn from(result: &PlateResult) -> Self {
        let mesh = result.mesh();
        Self {
            blob: Arc::from(mesh.blob().as_slice()),
            digest: mesh.mesh_digest(),
            vertices: Arc::from(mesh.vertices().as_slice()),
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
    pub(crate) id: ElementId,
    pub(crate) kind: ElementKind,
    pub(crate) placement: Translation3,
    pub(crate) full: SceneMesh,
    pub(crate) simplified: Option<SceneMesh>,
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
            full: full.into(),
            simplified: None,
        }
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
