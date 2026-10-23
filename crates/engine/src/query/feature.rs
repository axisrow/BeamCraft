// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V04 FeatureIds and `LookupFeature` (F06 sections 5 and 9) over a pinned view snapshot.
//!
//! A FeatureId is `fid1:` + ElementId + `:` + local id, the local id being `plate:` or `member:`
//! and a face (`f.x0`), edge (`e.x0.y1`) or vertex (`v.x0.y1.z1`) role. It is built from the rule
//! version, the identity, the body kind and the role tokens only, never from a triangle, vertex
//! or buffer index, so it survives every edit that keeps the topology (dimensions, placement,
//! rename) and a copy gets new ones.
//!
//! The roles exist only for a first-scope box body. The engine re-reads each element's mesh blob
//! and grants the 26 roles only when the blob has exactly the canonical box layout of F06
//! section 6 (8 vertices, the 12 canonical triangles with their face indices, 6 faces) and every
//! face is a rectangle. Any other body (a plate with through holes, for example) has no
//! FeatureIds: F06 rule 9 forbids inventing box roles for it. Feature geometry comes from the
//! blob's element-local positions plus the snapshot's translation, nothing else.

use crate::element::{ElementId, ElementKind};
use crate::error::{ViewCode, ViewError};
use crate::query::mesh::MeshEntry;
use crate::query::snapshot::{ElementEntry, ViewSnapshot};

/// The rule version of every FeatureId produced here.
pub const FEATURE_RULE: &str = "fid1";

/// Longest qualified FeatureId (F06 section 5).
pub const MAX_FEATURE_ID_BYTES: usize = 64;

/// Vertex quads of the six faces, wound counter-clockwise seen from outside (F06 section 6).
const FACE_QUADS: [[usize; 4]; 6] = [
    [0, 4, 6, 2],
    [1, 3, 7, 5],
    [0, 1, 5, 4],
    [2, 6, 7, 3],
    [0, 2, 3, 1],
    [4, 5, 7, 6],
];
const FACE_ROLES: [&str; 6] = ["f.x0", "f.x1", "f.y0", "f.y1", "f.z0", "f.z1"];
/// The canonical triangles and their face indices; a blob with any other layout is not a box.
const BOX_TRIANGLES: [[u32; 3]; 12] = [
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
const BOX_TRIANGLE_FACES: [u16; 12] = [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5];
const EDGE_ROLES: [&str; 12] = [
    "e.x0.y0", "e.x0.y1", "e.x1.y0", "e.x1.y1", "e.x0.z0", "e.x0.z1", "e.x1.z0", "e.x1.z1",
    "e.y0.z0", "e.y0.z1", "e.y1.z0", "e.y1.z1",
];
const EDGE_VERTICES: [[usize; 2]; 12] = [
    [0, 4],
    [2, 6],
    [1, 5],
    [3, 7],
    [0, 2],
    [4, 6],
    [1, 3],
    [5, 7],
    [0, 1],
    [4, 5],
    [2, 3],
    [6, 7],
];
const VERTEX_ROLES: [&str; 8] = [
    "v.x0.y0.z0",
    "v.x1.y0.z0",
    "v.x0.y1.z0",
    "v.x1.y1.z0",
    "v.x0.y0.z1",
    "v.x1.y0.z1",
    "v.x0.y1.z1",
    "v.x1.y1.z1",
];

/// Relative tolerance of the rectangle check of a face (orthogonal sides, closing corner). Kernel
/// boxes are rectangles to rounding; a blob outside it is not treated as a box.
const RECTANGLE_TOLERANCE: f64 = 1e-9;

/// The kind of a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FeatureKind {
    /// A planar face.
    Face,
    /// A straight edge.
    Edge,
    /// A vertex.
    Vertex,
}

impl FeatureKind {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Face => "face",
            Self::Edge => "edge",
            Self::Vertex => "vertex",
        }
    }

    /// Parses an exact wire token.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "face" => Some(Self::Face),
            "edge" => Some(Self::Edge),
            "vertex" => Some(Self::Vertex),
            _ => None,
        }
    }

    /// The tie-break rank of F06 section 9: face 0, edge 1, vertex 2.
    pub const fn rank(self) -> u8 {
        match self {
            Self::Face => 0,
            Self::Edge => 1,
            Self::Vertex => 2,
        }
    }
}

/// The body kind token of a FeatureId's local part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// A plate.
    Plate,
    /// A straight member (beam or column).
    Member,
}

impl BodyKind {
    /// The token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plate => "plate",
            Self::Member => "member",
        }
    }

    /// The body kind of an element kind, for the kinds that have one.
    pub const fn of(kind: ElementKind) -> Option<Self> {
        match kind {
            ElementKind::Plate => Some(Self::Plate),
            ElementKind::Beam | ElementKind::Column => Some(Self::Member),
            _ => None,
        }
    }
}

/// A parsed, well-formed FeatureId. Holds no index; equality is textual.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FeatureId {
    element: ElementId,
    body: BodyKindToken,
    kind: FeatureKind,
    /// Position of the role in its table (faces, edges or vertices).
    role: u8,
}

/// Ordered stand-in for [`BodyKind`] inside the derived `Ord`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BodyKindToken {
    Plate,
    Member,
}

impl From<BodyKind> for BodyKindToken {
    fn from(kind: BodyKind) -> Self {
        match kind {
            BodyKind::Plate => Self::Plate,
            BodyKind::Member => Self::Member,
        }
    }
}

fn role_table(kind: FeatureKind) -> &'static [&'static str] {
    match kind {
        FeatureKind::Face => &FACE_ROLES,
        FeatureKind::Edge => &EDGE_ROLES,
        FeatureKind::Vertex => &VERTEX_ROLES,
    }
}

impl FeatureId {
    fn new(element: ElementId, body: BodyKind, kind: FeatureKind, role: usize) -> Option<Self> {
        role_table(kind).get(role)?;
        Some(Self {
            element,
            body: body.into(),
            kind,
            role: u8::try_from(role).ok()?,
        })
    }

    /// Parses a qualified FeatureId.
    ///
    /// Text longer than 64 bytes, another rule version, a malformed ElementId, an unknown body
    /// kind or role is `GEO_FEATURE_ID_INVALID`. A local part that is a bare number (a triangle,
    /// vertex or buffer index) is `GEO_TRIANGLE_ADDRESS_FORBIDDEN`: indices are never addresses.
    pub fn parse(text: &str) -> Result<Self, ViewError> {
        let invalid = || ViewError::new(ViewCode::FeatureIdInvalid);
        if text.len() > MAX_FEATURE_ID_BYTES || !text.is_ascii() {
            return Err(invalid());
        }
        let rest = text
            .strip_prefix(FEATURE_RULE)
            .and_then(|r| r.strip_prefix(':'))
            .ok_or_else(invalid)?;
        let (element, local) = rest.split_once(':').ok_or_else(invalid)?;
        let (body, token) = local.split_once(':').unwrap_or(("", local));
        if !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ViewError::new(ViewCode::TriangleAddressForbidden));
        }
        let element = ElementId::parse(element).map_err(|_| invalid())?;
        if element.to_string() != rest.split_once(':').map_or("", |(e, _)| e) {
            // Only the canonical lowercase form is a FeatureId.
            return Err(invalid());
        }
        let body = match body {
            "plate" => BodyKind::Plate,
            "member" => BodyKind::Member,
            _ => return Err(invalid()),
        };
        for kind in [FeatureKind::Face, FeatureKind::Edge, FeatureKind::Vertex] {
            if let Some(role) = role_table(kind).iter().position(|r| *r == token) {
                return Self::new(element, body, kind, role).ok_or_else(invalid);
            }
        }
        Err(invalid())
    }

    /// The owning element.
    pub const fn element(&self) -> ElementId {
        self.element
    }

    /// The feature kind.
    pub const fn kind(&self) -> FeatureKind {
        self.kind
    }

    /// The body kind of the local part.
    pub const fn body(&self) -> BodyKind {
        match self.body {
            BodyKindToken::Plate => BodyKind::Plate,
            BodyKindToken::Member => BodyKind::Member,
        }
    }

    /// The role token, for example `f.x0`.
    pub fn role(&self) -> &'static str {
        role_table(self.kind)
            .get(usize::from(self.role))
            .copied()
            .unwrap_or_default()
    }

    /// The local id: body kind and role, for example `plate:f.x0`.
    pub fn local_id(&self) -> String {
        format!("{}:{}", self.body().as_str(), self.role())
    }
}

impl core::fmt::Display for FeatureId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{FEATURE_RULE}:{}:{}", self.element, self.local_id())
    }
}

pub(crate) type V3 = [f64; 3];

pub(crate) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub(crate) fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub(crate) fn finite(a: V3) -> bool {
    a.iter().all(|v| v.is_finite())
}

/// One rectangular face of a box body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Face {
    /// Corners in quad order, counter-clockwise seen from outside.
    pub(crate) corners: [V3; 4],
    /// Vertex roles of the corners (indices into the vertex table).
    pub(crate) vertices: [usize; 4],
    /// Outward unit normal.
    pub(crate) normal: V3,
}

/// The 26 features of one box body, in one space (element-local or world).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoxFeatures {
    pub(crate) element: ElementId,
    pub(crate) body: BodyKind,
    pub(crate) vertices: [V3; 8],
    pub(crate) faces: [Face; 6],
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// The box geometry of a structurally valid blob, or `None` when it is not the canonical box.
fn box_vertices(mesh: &MeshEntry) -> Option<[V3; 8]> {
    let bytes = mesh.bytes();
    if mesh.vertex_count() != 8 || mesh.triangle_count() != 12 || read_u32(bytes, 20)? != 6 {
        return None;
    }
    let mut vertices = [[0.0; 3]; 8];
    for (i, vertex) in vertices.iter_mut().enumerate() {
        for (axis, value) in vertex.iter_mut().enumerate() {
            let at = 32 + i * 24 + axis * 8;
            *value = f64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?);
        }
    }
    // Indices start after the 8 positions; 12 triangles take 144 bytes (a multiple of 8).
    let indices = 32 + 8 * 24;
    for (t, triangle) in BOX_TRIANGLES.iter().enumerate() {
        for (k, expected) in triangle.iter().enumerate() {
            if read_u32(bytes, indices + (t * 3 + k) * 4)? != *expected {
                return None;
            }
        }
    }
    let faces = indices + 144;
    for (t, expected) in BOX_TRIANGLE_FACES.iter().enumerate() {
        let at = faces + t * 2;
        let face = u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?);
        if face != *expected {
            return None;
        }
    }
    Some(vertices)
}

fn face_of(vertices: &[V3; 8], quad: [usize; 4]) -> Option<Face> {
    let corner = |i: usize| vertices.get(i).copied();
    let corners = [
        corner(quad[0])?,
        corner(quad[1])?,
        corner(quad[2])?,
        corner(quad[3])?,
    ];
    let [a, b, c, d] = corners;
    let (u, v) = (sub(b, a), sub(d, a));
    let (lu, lv) = (norm(u), norm(v));
    if !(lu > 0.0 && lv > 0.0 && lu.is_finite() && lv.is_finite()) {
        return None;
    }
    // A rectangle: orthogonal sides and the fourth corner closes the parallelogram.
    let size = lu.max(lv);
    if dot(u, v).abs() > RECTANGLE_TOLERANCE * lu * lv
        || norm(sub(c, add(b, v))) > RECTANGLE_TOLERANCE * size
    {
        return None;
    }
    let n = cross(u, v);
    let ln = norm(n);
    if !(ln > 0.0 && ln.is_finite()) {
        return None;
    }
    Some(Face {
        corners,
        vertices: quad,
        normal: scale(n, 1.0 / ln),
    })
}

impl BoxFeatures {
    /// The world-space features of a snapshot element, or `None` when its body has no FeatureIds
    /// (no box layout, or a kind without a body kind).
    pub(crate) fn world(snapshot: &ViewSnapshot, entry: &ElementEntry) -> Option<Self> {
        Self::placed(snapshot, entry, entry.placement_mm)
    }

    /// The element-local features (no translation).
    pub(crate) fn local(snapshot: &ViewSnapshot, entry: &ElementEntry) -> Option<Self> {
        Self::placed(snapshot, entry, [0.0; 3])
    }

    fn placed(snapshot: &ViewSnapshot, entry: &ElementEntry, by: V3) -> Option<Self> {
        let body = BodyKind::of(entry.kind)?;
        let mesh = snapshot.mesh_at(entry.full.mesh_slot)?;
        let local = box_vertices(mesh)?;
        // The rectangle check and the normals use element-local positions: a translation far
        // from the origin adds rounding to world coordinates that says nothing about the body,
        // and a placement-only move must keep every FeatureId.
        let mut faces = Vec::with_capacity(6);
        for quad in FACE_QUADS {
            faces.push(face_of(&local, quad)?);
        }
        let shift = |v: V3| add(v, by);
        let vertices = local.map(shift);
        if !vertices.iter().all(|v| finite(*v)) {
            return None;
        }
        for face in &mut faces {
            face.corners = face.corners.map(shift);
        }
        Some(Self {
            element: entry.id,
            body,
            vertices,
            faces: faces.try_into().ok()?,
        })
    }

    pub(crate) fn id(&self, kind: FeatureKind, role: usize) -> Option<FeatureId> {
        FeatureId::new(self.element, self.body, kind, role)
    }

    pub(crate) fn edge(&self, role: usize) -> Option<[V3; 2]> {
        let [a, b] = *EDGE_VERTICES.get(role)?;
        Some([*self.vertices.get(a)?, *self.vertices.get(b)?])
    }

    /// Centre of a face, by role.
    pub(crate) fn face_centre(&self, role: usize) -> Option<V3> {
        let f = self.faces.get(role)?;
        let sum = f.corners.iter().fold([0.0; 3], |s, c| add(s, *c));
        Some(scale(sum, 0.25))
    }
}

/// The geometry of one feature in one space.
#[derive(Debug, Clone, PartialEq)]
pub enum FeatureGeometry {
    /// A face: its outward unit normal and four corners with their vertex FeatureIds, in quad
    /// order (counter-clockwise seen from outside).
    Face {
        /// Outward unit normal.
        normal: [f64; 3],
        /// Corner positions in millimetres.
        corners: [[f64; 3]; 4],
        /// Vertex FeatureIds of the corners.
        vertex_ids: [FeatureId; 4],
    },
    /// An edge: its two end vertices.
    Edge {
        /// End positions in millimetres.
        ends: [[f64; 3]; 2],
        /// Vertex FeatureIds of the ends.
        vertex_ids: [FeatureId; 2],
    },
    /// A vertex position in millimetres.
    Vertex {
        /// Position.
        position: [f64; 3],
    },
}

/// Result of [`lookup_feature`].
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureLookup {
    /// The FeatureId looked up.
    pub id: FeatureId,
    /// Geometry in element-local millimetres.
    pub local: FeatureGeometry,
    /// Geometry in world millimetres (local plus the element's translation).
    pub world: FeatureGeometry,
}

fn geometry(features: &BoxFeatures, id: &FeatureId) -> Option<FeatureGeometry> {
    let role = usize::from(id.role);
    let vertex = |i: usize| features.id(FeatureKind::Vertex, i);
    Some(match id.kind {
        FeatureKind::Face => {
            let f = features.faces.get(role)?;
            let [a, b, c, d] = f.vertices;
            FeatureGeometry::Face {
                normal: f.normal,
                corners: f.corners,
                vertex_ids: [vertex(a)?, vertex(b)?, vertex(c)?, vertex(d)?],
            }
        }
        FeatureKind::Edge => {
            let [a, b] = *EDGE_VERTICES.get(role)?;
            FeatureGeometry::Edge {
                ends: features.edge(role)?,
                vertex_ids: [vertex(a)?, vertex(b)?],
            }
        }
        FeatureKind::Vertex => FeatureGeometry::Vertex {
            position: *features.vertices.get(role)?,
        },
    })
}

/// `LookupFeature`: the kind, owner, role and geometry of one feature of the snapshot.
///
/// A malformed id is `GEO_FEATURE_ID_INVALID`; an index used as an address is
/// `GEO_TRIANGLE_ADDRESS_FORBIDDEN`. A well-formed id is `GEO_FEATURE_NOT_FOUND` when its element
/// is not in the snapshot, its body kind disagrees with the element's kind, or the element's body
/// has no FeatureIds (not a box): a dangling reference is reported, never repaired by index.
pub fn lookup_feature(snapshot: &ViewSnapshot, text: &str) -> Result<FeatureLookup, ViewError> {
    let id = FeatureId::parse(text)?;
    let not_found = || ViewError::new(ViewCode::FeatureNotFound);
    let entry = snapshot.element(&id.element).ok_or_else(not_found)?;
    if BodyKind::of(entry.kind) != Some(id.body()) {
        return Err(not_found());
    }
    let world = BoxFeatures::world(snapshot, entry)
        .and_then(|f| geometry(&f, &id))
        .ok_or_else(not_found)?;
    let local = BoxFeatures::local(snapshot, entry)
        .and_then(|f| geometry(&f, &id))
        .ok_or_else(not_found)?;
    Ok(FeatureLookup { id, local, world })
}

/// Lists the 26 semantic addresses of one canonical box at this exact snapshot pin.
///
/// The order is faces, edges, then vertices, using each frozen F06 role table's order. No mesh
/// index is exposed as an address. A missing element or unsupported/non-box body is explicitly
/// `GEO_FEATURE_NOT_FOUND`, rather than an empty successful list or fabricated bounding box.
pub fn list_features(
    snapshot: &ViewSnapshot,
    element: &ElementId,
) -> Result<Vec<FeatureId>, ViewError> {
    let missing = || ViewError::new(ViewCode::FeatureNotFound);
    let entry = snapshot.element(element).ok_or_else(missing)?;
    let features = BoxFeatures::world(snapshot, entry).ok_or_else(missing)?;
    let mut result = Vec::with_capacity(26);
    for kind in [FeatureKind::Face, FeatureKind::Edge, FeatureKind::Vertex] {
        for role in 0..role_table(kind).len() {
            result.push(features.id(kind, role).ok_or_else(missing)?);
        }
    }
    Ok(result)
}

/// Whether the element's body has FeatureIds (a canonical box of a plate, beam or column).
pub fn has_features(snapshot: &ViewSnapshot, id: &ElementId) -> bool {
    snapshot
        .element(id)
        .and_then(|e| BoxFeatures::world(snapshot, e))
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Limits;
    use crate::query::snapshot::ViewPin;
    use crate::synthetic::plate_scene;
    use beamcraft_model::digest::Sha256;

    fn snapshot() -> ViewSnapshot {
        let pin = ViewPin::new(Sha256::digest(b"v04").unwrap(), 1);
        ViewSnapshot::build(pin, plate_scene(2, false).unwrap(), &Limits::default()).unwrap()
    }

    #[test]
    fn feature_ids_round_trip_and_refuse_indices_and_foreign_forms() {
        let s = snapshot();
        let e = &s.elements()[0];
        let face = format!("fid1:{}:plate:f.z1", e.id);
        let parsed = FeatureId::parse(&face).unwrap();
        assert_eq!(parsed.to_string(), face);
        assert_eq!(parsed.kind(), FeatureKind::Face);
        assert_eq!(parsed.local_id(), "plate:f.z1");
        // The longest form, a member vertex, fits the 64-byte bound.
        let vertex = format!("fid1:{}:member:v.x1.y1.z1", e.id);
        assert_eq!(vertex.len(), 59);
        assert!(FeatureId::parse(&vertex).is_ok());
        for bad in [
            format!("fid2:{}:plate:f.z1", e.id),
            format!("fid1:{}:slab:f.z1", e.id),
            format!("fid1:{}:plate:f.w0", e.id),
            format!("fid1:{}:plate:e.y0.x0", e.id),
            "fid1:019A7BF0-0000-7000-8000-0000000000AB:plate:f.z1".to_owned(),
            format!("fid1:{}:plate:f.z1:x", e.id),
            "fid1:0199e7a0:plate:f.z1".to_owned(),
            format!("fid1:{}:plate:{}", e.id, "f".repeat(40)),
        ] {
            assert_eq!(
                FeatureId::parse(&bad).unwrap_err().code(),
                ViewCode::FeatureIdInvalid,
                "{bad}"
            );
        }
        for index in [
            format!("fid1:{}:plate:7", e.id),
            format!("fid1:{}:11", e.id),
        ] {
            assert_eq!(
                FeatureId::parse(&index).unwrap_err().code(),
                ViewCode::TriangleAddressForbidden
            );
        }
    }

    #[test]
    fn a_box_has_26_features_with_outward_rectangular_faces() {
        let s = snapshot();
        let e = &s.elements()[1];
        let features = BoxFeatures::world(&s, e).unwrap();
        // Opposite faces have opposite normals, and each normal points away from the centre.
        let centre = scale(
            features.vertices.iter().fold([0.0; 3], |a, v| add(a, *v)),
            0.125,
        );
        for (role, face) in features.faces.iter().enumerate() {
            let out = sub(features.face_centre(role).unwrap(), centre);
            assert!(dot(out, face.normal) > 0.0, "{role}");
            assert!((norm(face.normal) - 1.0).abs() < 1e-12);
        }
        for kind in [FeatureKind::Face, FeatureKind::Edge, FeatureKind::Vertex] {
            for role in 0..role_table(kind).len() {
                let id = features.id(kind, role).unwrap();
                let found = lookup_feature(&s, &id.to_string()).unwrap();
                assert_eq!(found.id, id);
            }
        }
        // World geometry is local geometry plus the translation.
        let id = format!("fid1:{}:plate:v.x1.y1.z1", e.id);
        let found = lookup_feature(&s, &id).unwrap();
        let (FeatureGeometry::Vertex { position: w }, FeatureGeometry::Vertex { position: l }) =
            (&found.world, &found.local)
        else {
            panic!("vertex geometry expected");
        };
        assert_eq!(*w, add(*l, e.placement_mm));
    }

    #[test]
    fn unknown_elements_and_wrong_body_kinds_are_not_found() {
        let s = snapshot();
        let e = &s.elements()[0];
        let code = |text: String| lookup_feature(&s, &text).unwrap_err().code();
        assert_eq!(
            code(format!("fid1:{}:member:f.z1", e.id)),
            ViewCode::FeatureNotFound
        );
        assert_eq!(
            code("fid1:019a7bf0-0000-7000-8000-0000000000ff:plate:f.z1".to_owned()),
            ViewCode::FeatureNotFound
        );
    }

    #[test]
    fn listed_addresses_are_complete_unique_and_resolve_at_the_same_pin() {
        let s = snapshot();
        for element in s.elements() {
            let features = list_features(&s, &element.id).unwrap();
            assert_eq!(features.len(), 26);
            let unique: std::collections::BTreeSet<_> = features.iter().cloned().collect();
            assert_eq!(unique.len(), 26);
            for (kind, count) in [
                (FeatureKind::Face, 6),
                (FeatureKind::Edge, 12),
                (FeatureKind::Vertex, 8),
            ] {
                assert_eq!(features.iter().filter(|f| f.kind() == kind).count(), count);
            }
            for feature in features {
                assert_eq!(feature.element(), element.id);
                assert_eq!(
                    lookup_feature(&s, &feature.to_string()).unwrap().id,
                    feature
                );
            }
        }
        assert_eq!(
            list_features(
                &s,
                &ElementId::parse("019a7bf0-0000-7000-8000-0000000000ff").unwrap()
            )
            .unwrap_err()
            .code(),
            ViewCode::FeatureNotFound
        );
    }
}
