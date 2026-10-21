// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V03: a pinned view snapshot as one bounded byte bundle, so a viewport in another process can
//! receive exactly what the engine built.
//!
//! The bundle carries the pin, the token, the elements (id, kind, translation, full and simplified
//! mesh slots) and the mesh blobs. It is a transport of an engine result, never a model format.
//! [`ViewSnapshot::from_bundle`] trusts nothing in it: every blob is re-inspected and its digest
//! recomputed, the snapshot is rebuilt with [`ViewSnapshot::build`] (bounds, deduplication,
//! limits), and the rebuilt token must equal the bundled one. A bundle therefore cannot carry a
//! geometry, a placement or an identity the engine would not produce from the same inputs.
//!
//! Layout (little endian): `BCVIEW01`, revision digest (32), generation (u64), token digest
//! (32), element count (u32), mesh count (u32); per mesh its byte length (u32) and bytes; per
//! element its id (16), kind (u8, registry order), translation (3 x f64), full slot (u32) and
//! simplified slot (u32). Nothing may follow.

use crate::element::{ElementId, ElementKind};
use crate::error::{ViewCode, ViewError};
use crate::limits::{Limits, MAX_MESH_BLOB_BYTES};
use crate::query::mesh::MeshEntry;
use crate::query::snapshot::{ViewPin, ViewSnapshot};
use crate::scene::{SceneElement, SceneMesh};
use beamcraft_geom::{Point3, Translation3};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::id::Uuid7;
use std::collections::BTreeMap;
use std::sync::Arc;

const MAGIC: &[u8; 8] = b"BCVIEW01";

fn invalid() -> ViewError {
    ViewError::new(ViewCode::ArgumentInvalid)
}

fn push_u32(out: &mut Vec<u8>, n: usize) -> Result<(), ViewError> {
    let n = u32::try_from(n).map_err(|_| ViewError::new(ViewCode::BufferLimit))?;
    out.extend_from_slice(&n.to_le_bytes());
    Ok(())
}

/// The largest bundle a snapshot under `limits` can produce: header, every mesh at the total
/// mesh-byte cap with its length, and every element record.
pub fn max_bundle_bytes(limits: &Limits) -> usize {
    const ELEMENT: usize = 16 + 1 + 24 + 4 + 4;
    let meshes = limits
        .max_total_mesh_bytes()
        .saturating_add(limits.max_elements().saturating_mul(2).saturating_mul(4));
    (8usize + 32 + 8 + 32 + 4 + 4)
        .saturating_add(meshes)
        .saturating_add(limits.max_elements().saturating_mul(ELEMENT))
}

/// Element-local vertex positions of a structurally valid BCMESH01 blob.
fn positions(entry: &MeshEntry) -> Result<Vec<Point3>, ViewError> {
    let bytes = entry.bytes();
    let count = usize::try_from(entry.vertex_count()).map_err(|_| invalid())?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut xyz = [0.0; 3];
        for (axis, value) in xyz.iter_mut().enumerate() {
            let offset = i
                .checked_mul(24)
                .and_then(|o| o.checked_add(32 + axis * 8))
                .ok_or_else(invalid)?;
            let raw = bytes
                .get(offset..offset + 8)
                .and_then(|b| <[u8; 8]>::try_from(b).ok())
                .ok_or_else(invalid)?;
            *value = f64::from_le_bytes(raw);
        }
        out.push(Point3::new(xyz[0], xyz[1], xyz[2]));
    }
    Ok(out)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], ViewError> {
        let end = self.at.checked_add(n).ok_or_else(invalid)?;
        let slice = self.bytes.get(self.at..end).ok_or_else(invalid)?;
        self.at = end;
        Ok(slice)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ViewError> {
        self.take(N)?.try_into().map_err(|_| invalid())
    }
    fn u32(&mut self) -> Result<u32, ViewError> {
        Ok(u32::from_le_bytes(self.array::<4>()?))
    }
    fn usize(&mut self) -> Result<usize, ViewError> {
        usize::try_from(self.u32()?).map_err(|_| invalid())
    }
    fn f64(&mut self) -> Result<f64, ViewError> {
        Ok(f64::from_le_bytes(self.array::<8>()?))
    }
}

impl ViewSnapshot {
    /// The bundle of this snapshot.
    pub fn to_bundle(&self) -> Result<Vec<u8>, ViewError> {
        const ELEMENT: usize = 16 + 1 + 24 + 4 + 4;
        // The exact size; the snapshot's own limits bound it (see `max_bundle_bytes`).
        let size = self
            .meshes()
            .iter()
            .fold(8usize + 32 + 8 + 32 + 4 + 4, |n, m| {
                n.saturating_add(4).saturating_add(m.bytes().len())
            })
            .saturating_add(self.len().saturating_mul(ELEMENT));
        let mut out = Vec::with_capacity(size.min(max_bundle_bytes(&self.limits())));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(self.pin().revision().as_bytes());
        out.extend_from_slice(&self.pin().generation().to_le_bytes());
        let token = self
            .token()
            .as_str()
            .strip_prefix("vs1-")
            .and_then(|hex| hex.parse::<ContentDigest>().ok())
            .ok_or_else(invalid)?;
        out.extend_from_slice(token.as_bytes());
        push_u32(&mut out, self.len())?;
        push_u32(&mut out, self.meshes().len())?;
        for mesh in self.meshes() {
            push_u32(&mut out, mesh.bytes().len())?;
            out.extend_from_slice(mesh.bytes());
        }
        for e in self.elements() {
            out.extend_from_slice(e.id.as_bytes());
            let kind = ElementKind::ALL
                .iter()
                .position(|k| *k == e.kind)
                .and_then(|i| u8::try_from(i).ok())
                .ok_or_else(invalid)?;
            out.push(kind);
            for v in e.placement_mm {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&e.full.mesh_slot.to_le_bytes());
            out.extend_from_slice(&e.simplified.mesh_slot.to_le_bytes());
        }
        Ok(out)
    }

    /// Rebuilds a snapshot from a bundle under `limits`, re-validating everything (see the module
    /// documentation). Any disagreement is an error; nothing is partially returned.
    pub fn from_bundle(bytes: &[u8], limits: &Limits) -> Result<Self, ViewError> {
        if bytes.len() > max_bundle_bytes(limits) {
            return Err(ViewError::new(ViewCode::BufferLimit));
        }
        let mut r = Reader { bytes, at: 0 };
        if r.take(8)? != MAGIC {
            return Err(invalid());
        }
        let revision = ContentDigest::from_bytes(r.array::<32>()?);
        let generation = u64::from_le_bytes(r.array::<8>()?);
        let token = ContentDigest::from_bytes(r.array::<32>()?);
        let element_count = r.usize()?;
        if element_count > limits.max_elements() {
            return Err(ViewError::new(ViewCode::ElementLimit));
        }
        let mesh_count = r.usize()?;
        if mesh_count > element_count.saturating_mul(2) {
            return Err(invalid());
        }
        let mut meshes = Vec::with_capacity(mesh_count);
        let mut admitted = BTreeMap::new();
        let mut total = 0usize;
        for _ in 0..mesh_count {
            let len = r.usize()?;
            if len > MAX_MESH_BLOB_BYTES {
                return Err(ViewError::new(ViewCode::BufferLimit));
            }
            total = total
                .checked_add(len)
                .filter(|t| *t <= limits.max_total_mesh_bytes())
                .ok_or(ViewError::new(ViewCode::BufferLimit))?;
            // One structural check, one SHA-256 and one copy per blob: the scene mesh and the
            // snapshot entry share these bytes, and the builder does not validate them again.
            let entry = MeshEntry::admit(r.take(len)?)?;
            let vertices = positions(&entry)?;
            meshes.push(SceneMesh::shared(
                Arc::clone(entry.shared()),
                entry.digest(),
                vertices,
            ));
            // A duplicate blob replaces its twin here; the mesh-count check below refuses it.
            admitted.insert(entry.digest(), entry);
        }
        let mut elements = Vec::with_capacity(element_count);
        for _ in 0..element_count {
            let id = ElementId::from_uuid(Uuid7::from_bytes(r.array::<16>()?)?);
            let kind = *ElementKind::ALL
                .get(usize::from(r.array::<1>()?[0]))
                .ok_or_else(invalid)?;
            let (x, y, z) = (r.f64()?, r.f64()?, r.f64()?);
            let placement = Translation3::new(x, y, z)
                .map_err(|_| ViewError::new(ViewCode::NumericUncertain))?;
            let full = meshes.get(r.usize()?).ok_or_else(invalid)?.clone();
            let simplified = meshes.get(r.usize()?).ok_or_else(invalid)?.clone();
            let element = SceneElement::new(id, kind, placement, full.clone());
            elements.push(if simplified == full {
                element
            } else {
                element.with_simplified(simplified)
            });
        }
        if r.at != bytes.len() {
            return Err(invalid());
        }
        let snapshot = Self::build_admitted(
            ViewPin::new(revision, generation),
            elements,
            limits,
            admitted,
        )?;
        // The rebuilt content must be exactly the bundled one: same token, and every mesh used.
        let expected = format!("vs1-{token}");
        if snapshot.token().as_str() != expected || snapshot.meshes().len() != mesh_count {
            return Err(ViewError::new(ViewCode::BlobInvalid));
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::plate_scene;
    use beamcraft_model::digest::Sha256;

    fn snapshot(count: u32, simplified: bool) -> ViewSnapshot {
        let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 7);
        ViewSnapshot::build(
            pin,
            plate_scene(count, simplified).unwrap(),
            &Limits::default(),
        )
        .unwrap()
    }

    #[test]
    fn a_bundle_rebuilds_the_identical_snapshot() {
        for simplified in [false, true] {
            let s = snapshot(12, simplified);
            let bytes = s.to_bundle().unwrap();
            let back = ViewSnapshot::from_bundle(&bytes, &Limits::default()).unwrap();
            assert_eq!(back, s);
            assert!(bytes.len() <= max_bundle_bytes(&Limits::default()));
        }
    }

    #[test]
    fn tampered_truncated_or_padded_bundles_are_refused() {
        let s = snapshot(3, false);
        let bytes = s.to_bundle().unwrap();
        let limits = Limits::default();
        for n in [0, 7, 8, 40, bytes.len() / 2, bytes.len() - 1] {
            assert!(ViewSnapshot::from_bundle(&bytes[..n], &limits).is_err());
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(ViewSnapshot::from_bundle(&longer, &limits).is_err());
        // A moved placement changes the token: the rebuilt snapshot disagrees.
        let mut moved = bytes.clone();
        let last_x = bytes.len() - 8 - 24;
        moved[last_x] ^= 0x01;
        assert!(ViewSnapshot::from_bundle(&moved, &limits).is_err());
        // Every single flipped byte is either refused or yields the same token.
        for i in 0..bytes.len() {
            let mut flipped = bytes.clone();
            flipped[i] ^= 0x80;
            if let Ok(back) = ViewSnapshot::from_bundle(&flipped, &limits) {
                assert_eq!(back.token(), s.token());
            }
        }
    }

    #[test]
    fn a_bundle_with_a_duplicated_or_unused_blob_is_refused() {
        let limits = Limits::default();
        let s = snapshot(2, false);
        assert_eq!(s.meshes().len(), 2);
        let bytes = s.to_bundle().unwrap();
        // Header: magic 8, revision 32, generation 8, token 32, element count 4, mesh count 4.
        let counts = 8 + 32 + 8 + 32;
        let first_len = u32::from_le_bytes(bytes[counts + 8..counts + 12].try_into().unwrap());
        let first = counts + 8..counts + 12 + first_len as usize;
        // A third blob, a byte-identical copy of the first, that no element uses.
        let mut unused = bytes[..first.end].to_vec();
        unused[counts + 4..counts + 8].copy_from_slice(&3u32.to_le_bytes());
        unused.extend_from_slice(&bytes[first.clone()]);
        unused.extend_from_slice(&bytes[first.end..]);
        assert_eq!(
            ViewSnapshot::from_bundle(&unused, &limits)
                .unwrap_err()
                .code(),
            ViewCode::BlobInvalid
        );
        // The same copy used by the second element instead of its own blob: one blob too many.
        let mut used = unused.clone();
        let simplified_slot = used.len() - 4;
        let full_slot = used.len() - 8;
        used[full_slot..simplified_slot].copy_from_slice(&2u32.to_le_bytes());
        used[simplified_slot..].copy_from_slice(&2u32.to_le_bytes());
        assert!(ViewSnapshot::from_bundle(&used, &limits).is_err());
    }

    #[test]
    fn a_rebuilt_snapshot_shares_one_copy_of_each_blob() {
        // 40 copies of one plate: one blob in the bundle, one entry in the snapshot.
        let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 3);
        let plate = plate_scene(1, false).unwrap().remove(0).full;
        let elements = (0..40u64)
            .map(|n| {
                let id = ElementId::parse(&format!("01990000-0000-7000-8000-{n:012x}")).unwrap();
                let at = Translation3::new(n as f64 * 200.0, 0.0, 0.0).unwrap();
                SceneElement::new(id, ElementKind::Plate, at, plate.clone())
            })
            .collect();
        let s = ViewSnapshot::build(pin, elements, &Limits::default()).unwrap();
        let back = ViewSnapshot::from_bundle(&s.to_bundle().unwrap(), &Limits::default()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.meshes().len(), 1);
    }
}
