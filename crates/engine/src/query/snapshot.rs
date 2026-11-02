// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The pinned, immutable view snapshot and its read queries.
//!
//! A snapshot answers `ListElements`, `GetMeshSnapshot` and `QueryBounds` at exactly one pinned
//! revision and generation (F06 section 9). It holds the whole authoritative scene; what a
//! viewport draws is a read-time projection of it ([`crate::presentation`]), so view settings
//! never enter the token. Element indices are snapshot-local positions; they are never
//! identities and are never accepted as addresses by an engineering query.

use crate::element::{ElementId, ElementKind};
use crate::error::{ViewCode, ViewError};
use crate::limits::{Limits, MAX_PAGE_SIZE};
use crate::presentation::Representation;
use crate::query::bounds::Aabb;
use crate::query::mesh::MeshEntry;
use crate::scene::{MAX_FACE_ROLE_BYTES, SceneElement, SceneMesh};
use beamcraft_geom::{Point3, Translation3, canonical_f64_bytes};
use beamcraft_model::digest::{ContentDigest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The pinned context of a snapshot: the base revision and the workspace generation.
///
/// The project and workspace identities belong to the host that owns the pin; they are not part
/// of the engine's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewPin {
    revision: ContentDigest,
    generation: u64,
}

impl ViewPin {
    /// Pins a revision digest and a workspace generation.
    pub const fn new(revision: ContentDigest, generation: u64) -> Self {
        Self {
            revision,
            generation,
        }
    }
    /// The base revision digest.
    pub const fn revision(&self) -> ContentDigest {
        self.revision
    }
    /// The workspace generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// An opaque snapshot token: `vs1-` and 64 lowercase hexadecimal digits.
///
/// Compare for equality only. It covers the pin and the authoritative content of the snapshot
/// (ids, kinds, placements, mesh digests) and nothing about view settings or selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnapshotToken(String);

impl SnapshotToken {
    const PREFIX: &'static str = "vs1-";

    /// Parses the exact token spelling; anything else is `VIEW_ARGUMENT_INVALID`.
    pub fn parse(text: &str) -> Result<Self, ViewError> {
        let valid = text.len() == Self::PREFIX.len() + 64
            && text.starts_with(Self::PREFIX)
            && text
                .bytes()
                .skip(Self::PREFIX.len())
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(ViewError::new(ViewCode::ArgumentInvalid))
        }
    }

    /// The token text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn from_digest(digest: ContentDigest) -> Self {
        Self(format!("{}{digest}", Self::PREFIX))
    }
}

/// One representation of an element inside a snapshot.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RepresentationRef {
    /// SHA-256 of the mesh blob; fetch it with [`ViewSnapshot::mesh`].
    pub mesh_digest: ContentDigest,
    /// Position of the blob in [`ViewSnapshot::meshes`]; valid only within this snapshot.
    pub mesh_slot: u32,
    /// Whether this is the exact geometry. The full representation is always exact; a
    /// simplified one is exact only when it is the same mesh as the full one.
    pub exact: bool,
    /// World bounds of this representation, computed by the engine from mesh vertices.
    pub world_bounds: Aabb,
}

/// One element of a snapshot.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ElementEntry {
    /// The stable identity.
    pub id: ElementId,
    /// Snapshot-local position in ascending id order. Never an identity.
    pub index: u32,
    /// The element kind.
    pub kind: ElementKind,
    /// Translation placement in millimetres.
    pub placement_mm: [f64; 3],
    /// Rigid orientation in quaternion `(x, y, z, w)` form.
    pub orientation_xyzw: [f64; 4],
    /// The full representation (operations applied): the only one engineering uses.
    pub full: RepresentationRef,
    /// The simplified representation; the same mesh as `full` when there are no operations.
    pub simplified: RepresentationRef,
    /// Stable semantic face roles for the full evaluated body.
    pub full_face_table: Vec<String>,
    /// Stable semantic face roles for the simplified body.
    pub simplified_face_table: Vec<String>,
}

impl ElementEntry {
    /// The reference for a chosen representation.
    pub const fn representation(&self, representation: Representation) -> &RepresentationRef {
        match representation {
            Representation::Full => &self.full,
            Representation::Simplified => &self.simplified,
        }
    }
}

/// One page of `ListElements`.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementPage<'a> {
    /// Entries in ascending id order.
    pub entries: &'a [ElementEntry],
    /// Pass this as `after` to read the next page; `None` after the last page.
    pub next_cursor: Option<ElementId>,
}

/// An immutable view snapshot pinned to one revision and generation.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewSnapshot {
    pin: ViewPin,
    token: SnapshotToken,
    limits: Limits,
    elements: Vec<ElementEntry>,
    meshes: Vec<MeshEntry>,
    engineering_bounds: Option<Aabb>,
}

struct MeshRegistry {
    map: BTreeMap<ContentDigest, MeshEntry>,
    /// Blobs a caller has already validated (the meshes of a bundle). One enters the snapshot,
    /// without being validated again, only when an element offers exactly those bytes.
    admitted: BTreeMap<ContentDigest, MeshEntry>,
    total: usize,
    limit: usize,
}

impl MeshRegistry {
    fn add(&mut self, mesh: &SceneMesh) -> Result<ContentDigest, ViewError> {
        let digest = mesh.digest;
        match self.map.get(&digest) {
            // A digest already admitted names verified bytes: equal bytes reuse the entry (copies
            // share them, so most need no comparison), and different bytes under one digest are
            // tampered content, refused without paying another validation, hash and copy per
            // duplicate.
            Some(existing)
                if Arc::ptr_eq(existing.shared(), &mesh.blob)
                    || existing.bytes() == mesh.blob.as_ref() => {}
            Some(_) => return Err(ViewError::new(ViewCode::BlobInvalid)),
            None => {
                // New content: the structural check and digest verification run exactly once,
                // here or where the caller admitted these very bytes; they are shared, not copied.
                let entry = match self.admitted.remove(&digest) {
                    Some(entry) if Arc::ptr_eq(entry.shared(), &mesh.blob) => entry,
                    _ => MeshEntry::from_shared(Arc::clone(&mesh.blob), &digest)?,
                };
                self.total = self
                    .total
                    .checked_add(entry.bytes().len())
                    .filter(|total| *total <= self.limit)
                    .ok_or(ViewError::new(ViewCode::BufferLimit))?;
                self.map.insert(digest, entry);
            }
        }
        Ok(digest)
    }
}

struct Pending {
    id: ElementId,
    kind: ElementKind,
    placement: [f64; 3],
    orientation_xyzw: [f64; 4],
    full: (ContentDigest, Aabb),
    simplified: (ContentDigest, Aabb),
    full_face_table: Vec<String>,
    simplified_face_table: Vec<String>,
}

fn admit_face_table(
    mesh: &SceneMesh,
    kind: ElementKind,
    remaining: &mut usize,
) -> Result<(), ViewError> {
    let invalid = || ViewError::new(ViewCode::BlobInvalid);
    if !mesh.face_table.is_empty() {
        let count = mesh
            .blob
            .get(20..24)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map(u32::from_le_bytes)
            .ok_or_else(invalid)?;
        if usize::try_from(count).ok() != Some(mesh.face_table.len()) {
            return Err(invalid());
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for role in mesh.face_table.iter() {
        if role.is_empty()
            || role.len() > MAX_FACE_ROLE_BYTES
            || !role
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b":.-".contains(&b))
            || !seen.insert(role.as_str())
        {
            return Err(invalid());
        }
        let role_length = if kind.is_linear_member() && role.starts_with("extrusion:") {
            role.len().saturating_sub(2)
        } else {
            role.len()
        };
        let vertex_length = role
            .find(":f.c")
            .and_then(|at| role.get(at + 4..))
            .and_then(|s| s.split_once(".s"))
            .map(|_| role_length + 2);
        if role_length + 42 > 64 || vertex_length.is_some_and(|len| len + 42 > 64) {
            return Err(ViewError::new(ViewCode::TopologyLimit));
        }
        *remaining = remaining
            .checked_sub(4 + role.len())
            .ok_or(ViewError::new(ViewCode::BufferLimit))?;
    }
    Ok(())
}

/// Mesh-local bounds, computed once per distinct mesh: the placement is applied to the result,
/// not to every vertex.
fn local_bounds(mesh: &SceneMesh) -> Result<Aabb, ViewError> {
    let numeric = || ViewError::new(ViewCode::NumericUncertain);
    let mut bounds: Option<Aabb> = None;
    for vertex in mesh.vertices.iter() {
        let point = Aabb::at(*vertex);
        bounds = Some(bounds.map_or(point, |b| b.union(point)));
    }
    bounds.ok_or_else(numeric)
}

/// World bounds of local bounds under a rigid placement. Transform all eight corners because a
/// rotated local AABB's min/max corner pair is not sufficient to recover its world AABB.
fn placed_bounds(
    local: Aabb,
    placement: Translation3,
    orientation_xyzw: [f64; 4],
) -> Result<Aabb, ViewError> {
    let numeric = || ViewError::new(ViewCode::NumericUncertain);
    let [qx, qy, qz, qw] = orientation_xyzw;
    let norm = ((((qx * qx) + (qy * qy)) + (qz * qz)) + (qw * qw)).sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(numeric());
    }
    let mut bounds: Option<Aabb> = None;
    let [min_x, min_y, min_z] = local.min;
    let [max_x, max_y, max_z] = local.max;
    for mask in 0..8 {
        let point = [
            if (mask & 1) == 0 { min_x } else { max_x },
            if (mask & 2) == 0 { min_y } else { max_y },
            if (mask & 4) == 0 { min_z } else { max_z },
        ];
        let rotated = rotate_vector(point, orientation_xyzw);
        let world = placement
            .apply_point(Point3::new(rotated[0], rotated[1], rotated[2]))
            .map_err(|_| numeric())?;
        let point = Aabb {
            min: [world.x, world.y, world.z],
            max: [world.x, world.y, world.z],
        };
        bounds = Some(bounds.map_or(point, |b| b.union(point)));
    }
    bounds.ok_or_else(numeric)
}

pub(crate) use crate::solid::rotate_vector;

fn slot_of(meshes: &[MeshEntry], digest: &ContentDigest) -> Result<u32, ViewError> {
    let bad = || ViewError::new(ViewCode::BlobInvalid);
    let slot = meshes
        .binary_search_by(|m| m.digest().cmp(digest))
        .map_err(|_| bad())?;
    u32::try_from(slot).map_err(|_| bad())
}

impl ViewSnapshot {
    /// Builds the snapshot of `elements` at `pin`.
    ///
    /// The element count is checked before any work. Elements are ordered by id; a duplicate id
    /// is rejected. Each distinct blob is re-inspected and its digest recomputed once; identical
    /// blobs are stored once. Nothing is partially returned on failure.
    pub fn build(
        pin: ViewPin,
        elements: Vec<SceneElement>,
        limits: &Limits,
    ) -> Result<Self, ViewError> {
        Self::build_admitted(pin, elements, limits, BTreeMap::new())
    }

    /// [`ViewSnapshot::build`] with meshes the caller has already validated (see
    /// [`MeshRegistry`]): an element whose mesh is exactly one of them is not validated again.
    pub(crate) fn build_admitted(
        pin: ViewPin,
        mut elements: Vec<SceneElement>,
        limits: &Limits,
        admitted: BTreeMap<ContentDigest, MeshEntry>,
    ) -> Result<Self, ViewError> {
        if elements.len() > limits.max_elements() {
            return Err(ViewError::new(ViewCode::ElementLimit));
        }
        elements.sort_by_key(SceneElement::id);
        if elements
            .windows(2)
            .any(|pair| matches!(pair, [a, b] if a.id == b.id))
        {
            return Err(ViewError::new(ViewCode::DuplicateElement));
        }
        let mut registry = MeshRegistry {
            map: BTreeMap::new(),
            admitted,
            total: 0,
            limit: limits.max_total_mesh_bytes(),
        };
        let mut pending = Vec::with_capacity(elements.len());
        let mut local: BTreeMap<ContentDigest, Aabb> = BTreeMap::new();
        let mut metadata_remaining = limits.max_total_mesh_bytes();
        for element in &elements {
            let full_digest = registry.add(&element.full)?;
            admit_face_table(&element.full, element.kind, &mut metadata_remaining)?;
            admit_face_table(
                element.simplified.as_ref().unwrap_or(&element.full),
                element.kind,
                &mut metadata_remaining,
            )?;
            // The registry verified this mesh's bytes against its digest, so its local bounds
            // are trusted for every element that shares the digest.
            let full_local = match local.get(&full_digest) {
                Some(found) => *found,
                None => {
                    let found = local_bounds(&element.full)?;
                    local.insert(full_digest, found);
                    found
                }
            };
            let full_bounds =
                placed_bounds(full_local, element.placement, element.orientation_xyzw)?;
            let simplified = match &element.simplified {
                Some(plate) => {
                    let digest = registry.add(plate)?;
                    let plate_local = match local.get(&digest) {
                        Some(found) => *found,
                        None => {
                            let found = local_bounds(plate)?;
                            local.insert(digest, found);
                            found
                        }
                    };
                    (
                        digest,
                        placed_bounds(plate_local, element.placement, element.orientation_xyzw)?,
                    )
                }
                None => (full_digest, full_bounds),
            };
            pending.push(Pending {
                id: element.id,
                kind: element.kind,
                placement: element.placement.offset().components(),
                orientation_xyzw: element.orientation_xyzw,
                full: (full_digest, full_bounds),
                simplified,
                full_face_table: element.full.face_table.iter().cloned().collect(),
                simplified_face_table: element.simplified.as_ref().map_or_else(
                    || element.full.face_table.iter().cloned().collect(),
                    |mesh| mesh.face_table.iter().cloned().collect(),
                ),
            });
        }
        let meshes: Vec<MeshEntry> = registry.map.into_values().collect();
        let mut entries = Vec::with_capacity(pending.len());
        let mut engineering_bounds: Option<Aabb> = None;
        for (position, p) in pending.iter().enumerate() {
            let full = RepresentationRef {
                mesh_digest: p.full.0,
                mesh_slot: slot_of(&meshes, &p.full.0)?,
                exact: true,
                world_bounds: p.full.1,
            };
            let simplified = RepresentationRef {
                mesh_digest: p.simplified.0,
                mesh_slot: slot_of(&meshes, &p.simplified.0)?,
                exact: p.simplified.0 == p.full.0,
                world_bounds: p.simplified.1,
            };
            engineering_bounds =
                Some(engineering_bounds.map_or(full.world_bounds, |b| b.union(full.world_bounds)));
            entries.push(ElementEntry {
                id: p.id,
                index: u32::try_from(position)
                    .map_err(|_| ViewError::new(ViewCode::ElementLimit))?,
                kind: p.kind,
                placement_mm: p.placement,
                orientation_xyzw: p.orientation_xyzw,
                full,
                simplified,
                full_face_table: p.full_face_table.clone(),
                simplified_face_table: p.simplified_face_table.clone(),
            });
        }
        let token = SnapshotToken::from_digest(content_digest(pin, &entries)?);
        Ok(Self {
            pin,
            token,
            limits: *limits,
            elements: entries,
            meshes,
            engineering_bounds,
        })
    }

    /// The pinned context.
    pub const fn pin(&self) -> ViewPin {
        self.pin
    }

    /// The snapshot token.
    pub const fn token(&self) -> &SnapshotToken {
        &self.token
    }

    /// The effective limits this snapshot was built under.
    pub const fn limits(&self) -> Limits {
        self.limits
    }

    /// Number of elements.
    pub const fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether the snapshot has no element.
    pub const fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// All elements in ascending id order.
    pub fn elements(&self) -> &[ElementEntry] {
        &self.elements
    }

    /// All distinct mesh blobs, in ascending digest order.
    pub fn meshes(&self) -> &[MeshEntry] {
        &self.meshes
    }

    /// The blob with this digest, if the snapshot references it.
    pub fn mesh(&self, digest: &ContentDigest) -> Option<&MeshEntry> {
        self.meshes
            .binary_search_by(|m| m.digest().cmp(digest))
            .ok()
            .and_then(|i| self.meshes.get(i))
    }

    /// The blob at a snapshot-local slot.
    pub fn mesh_at(&self, slot: u32) -> Option<&MeshEntry> {
        self.meshes.get(usize::try_from(slot).ok()?)
    }

    /// The element with this id.
    pub fn element(&self, id: &ElementId) -> Option<&ElementEntry> {
        self.elements
            .binary_search_by(|e| e.id.cmp(id))
            .ok()
            .and_then(|i| self.elements.get(i))
    }

    /// The element at a snapshot-local index.
    pub fn element_at(&self, index: u32) -> Option<&ElementEntry> {
        self.elements.get(usize::try_from(index).ok()?)
    }

    /// `ListElements`: at most `page_size` (1 to 1,024) entries after the cursor id.
    ///
    /// The cursor is the last id of the previous page; it need not exist in the snapshot.
    pub fn list_elements(
        &self,
        after: Option<&ElementId>,
        page_size: usize,
    ) -> Result<ElementPage<'_>, ViewError> {
        if page_size == 0 || page_size > MAX_PAGE_SIZE {
            return Err(ViewError::new(ViewCode::ArgumentInvalid));
        }
        let start = after.map_or(0, |c| self.elements.partition_point(|e| e.id <= *c));
        let tail = self.elements.get(start..).unwrap_or_default();
        let entries = tail.get(..page_size).unwrap_or(tail);
        let next_cursor = if tail.len() > entries.len() {
            entries.last().map(|e| e.id)
        } else {
            None
        };
        Ok(ElementPage {
            entries,
            next_cursor,
        })
    }

    /// The full representation of an element: the only one volume, mass, bills of materials and
    /// snapping may use, whatever the view settings say.
    pub fn engineering_representation(
        &self,
        id: &ElementId,
    ) -> Result<&RepresentationRef, ViewError> {
        self.element(id)
            .map(|e| &e.full)
            .ok_or(ViewError::new(ViewCode::UnknownElement))
    }

    /// `QueryBounds` for one element's full representation, in world space.
    pub fn element_bounds(&self, id: &ElementId) -> Result<Aabb, ViewError> {
        self.engineering_representation(id).map(|r| r.world_bounds)
    }

    /// World bounds of every element's full representation, independent of any view setting.
    pub const fn engineering_bounds(&self) -> Option<Aabb> {
        self.engineering_bounds
    }
}

fn content_digest(pin: ViewPin, entries: &[ElementEntry]) -> Result<ContentDigest, ViewError> {
    let limit = || ViewError::new(ViewCode::BufferLimit);
    let numeric = || ViewError::new(ViewCode::NumericUncertain);
    let mut h = Sha256::new();
    let mut put = |bytes: &[u8]| h.update(bytes).map_err(|_| limit());
    put(b"BCVIEW-SNAPSHOT\0")?;
    put(pin.revision.as_bytes())?;
    put(&pin.generation.to_be_bytes())?;
    put(&u64::try_from(entries.len())
        .map_err(|_| limit())?
        .to_be_bytes())?;
    for e in entries {
        put(e.id.as_bytes())?;
        put(&[e.kind as u8])?;
        for c in e.placement_mm {
            put(&canonical_f64_bytes(c).map_err(|_| numeric())?)?;
        }
        for c in e.orientation_xyzw {
            put(&canonical_f64_bytes(c).map_err(|_| numeric())?)?;
        }
        put(e.full.mesh_digest.as_bytes())?;
        put(e.simplified.mesh_digest.as_bytes())?;
        for table in [&e.full_face_table, &e.simplified_face_table] {
            put(&u64::try_from(table.len())
                .map_err(|_| limit())?
                .to_be_bytes())?;
            for role in table {
                put(&u64::try_from(role.len())
                    .map_err(|_| limit())?
                    .to_be_bytes())?;
                put(role.as_bytes())?;
            }
        }
    }
    Ok(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneElement, SceneMesh};
    use beamcraft_geom::Translation3;
    use beamcraft_kernel::{RectangularPlate, rectangular_plate};

    fn plate_mesh() -> SceneMesh {
        SceneMesh::from(
            &rectangular_plate(RectangularPlate {
                width_mm: 100.0,
                length_mm: 80.0,
                thickness_mm: 10.0,
            })
            .unwrap(),
        )
    }

    fn placed(id: u64, mesh: SceneMesh, at: f64) -> SceneElement {
        SceneElement::new(
            ElementId::parse(&format!("01990000-0000-7000-8000-{id:012x}")).unwrap(),
            ElementKind::Plate,
            Translation3::new(at, 0.0, 0.0).unwrap(),
            mesh,
        )
    }

    fn pin() -> ViewPin {
        ViewPin::new(Sha256::digest(b"revision").unwrap(), 1)
    }

    #[test]
    fn copies_share_one_verified_mesh_and_keep_exact_translated_bounds() {
        let shared = plate_mesh();
        let snapshot = ViewSnapshot::build(
            pin(),
            vec![placed(0, shared.clone(), 0.0), placed(1, shared, 200.0)],
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(snapshot.meshes().len(), 1);
        let (first, second) = (snapshot.elements().first(), snapshot.elements().get(1));
        let (first, second) = (
            first.unwrap().full.world_bounds,
            second.unwrap().full.world_bounds,
        );
        assert_eq!(second.min[0] - first.min[0], 200.0);
        assert_eq!(second.max[0] - first.max[0], 200.0);
        assert_eq!(first.min[1], second.min[1]);
        assert_eq!(first.max[2], second.max[2]);
    }

    #[test]
    fn rigid_orientation_rotates_local_bounds_before_translation() {
        let half = core::f64::consts::FRAC_1_SQRT_2;
        let element = SceneElement::new(
            ElementId::parse("01990000-0000-7000-8000-000000000010").unwrap(),
            ElementKind::Plate,
            Translation3::new(-50.0, 70.0, 1000.0).unwrap(),
            plate_mesh(),
        )
        .with_orientation([0.0, 0.0, half, half]);
        let snapshot = ViewSnapshot::build(pin(), vec![element], &Limits::default()).unwrap();
        let bounds = snapshot.engineering_bounds().unwrap();
        for (actual, expected) in bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .zip([-130.0, 70.0, 1000.0, -50.0, 170.0, 1010.0])
        {
            assert!((actual - expected).abs() <= 1e-12, "{actual} != {expected}");
        }
    }

    #[test]
    fn a_mesh_is_validated_once_and_its_bytes_are_kept_not_copied() {
        let mesh = plate_mesh();
        // Copies sharing the bytes, and one element with an equal private copy of them.
        let private = SceneMesh::new(mesh.blob.to_vec(), mesh.digest, mesh.vertices.to_vec());
        let elements = vec![
            placed(0, mesh.clone(), 0.0),
            placed(1, mesh.clone(), 200.0),
            placed(2, private, 400.0),
        ];
        let s = ViewSnapshot::build(pin(), elements, &Limits::default()).unwrap();
        assert_eq!(s.meshes().len(), 1);
        assert!(Arc::ptr_eq(s.meshes()[0].shared(), &mesh.blob));
        assert_eq!(s.meshes()[0].digest(), mesh.digest);
    }

    #[test]
    fn different_bytes_under_one_admitted_digest_are_refused_on_the_reuse_path() {
        let good = plate_mesh();
        let digest = good.digest;
        let mut bytes = good.blob.as_ref().to_vec();
        let coordinate = 32 + 24;
        bytes[coordinate] ^= 0x01;
        let forged = SceneMesh::new(bytes, digest, good.vertices.as_ref().to_vec());
        let result = ViewSnapshot::build(
            pin(),
            vec![placed(0, good, 0.0), placed(1, forged, 400.0)],
            &Limits::default(),
        );
        assert!(matches!(result, Err(e) if e.code() == ViewCode::BlobInvalid));
    }
}
