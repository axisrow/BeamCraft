// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `CopyElement` for plates and members (F04 section 7, P05).
//!
//! A copy has a fresh identity and the source's authored inputs, with an optional label and a
//! translation added to the placement. The shape inputs, the pinned material and the pinned
//! profile are shared by value: definitions are immutable, so the copy follows the same digests.
//! Because the body is element-local, the copy's body and mesh are the source's, byte for byte;
//! only a later edit of the copy that changes its shape produces different geometry.
//!
//! Relationship authoring belongs to the adapter and layer engine: a duplicate keeps its
//! level and immutable definitions, but joins no assembly and takes no connections (DV-REL-011).

use crate::command::AuthoredPlate;
use crate::member::AuthoredMember;
use beamcraft_geom::canonical_f64_bytes;
use beamcraft_model::command::{CommandError, CopyElement, CutRecord, Placement};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, Uuid7};

fn copied_placement(source: &Placement, command: &CopyElement) -> Result<Placement, CommandError> {
    let [ox, oy, oz] = command.offset_mm;
    let [sx, sy, sz] = source.translation_mm;
    let moved = Placement {
        translation_mm: [sx + ox, sy + oy, sz + oz],
        orientation_xyzw: source.orientation_xyzw,
    };
    moved.validate()?;
    Ok(moved)
}

/// The fresh operation id of one copied record (F04 section 7: fresh operation ids, the same
/// ordinals). The derivation is deterministic — a UUIDv7 hashed from the copy's identity, the
/// source record and the ordinal — so a retried copy reproduces the exact same state. The
/// version and variant bits are forced before the check, so the refusal is unreachable and
/// means only that a derived identity could not be formed.
fn copied_operation_id(
    record: &CutRecord,
    ordinal: usize,
    new_id: &ElementId,
) -> Result<OpId, CommandError> {
    let mut seed = Vec::with_capacity(96);
    seed.extend_from_slice(b"BeamCraft/A03/CopyCutRecord\0");
    seed.extend_from_slice(new_id.as_bytes());
    seed.extend_from_slice(record.operation_id.as_bytes());
    seed.extend_from_slice(&(ordinal as u64).to_le_bytes());
    for value in record.point_mm.into_iter().chain(record.normal) {
        seed.extend_from_slice(&canonical_f64_bytes(value).unwrap_or([0xff; 8]));
    }
    let digest = Sha256::digest(&seed).unwrap_or_else(|_| ContentDigest::from_bytes([0; 32]));
    let mut bytes = [0u8; 16];
    for (out, value) in bytes.iter_mut().zip(digest.as_bytes().iter()) {
        *out = *value;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid7::from_bytes(bytes).map(OpId::from_uuid).map_err(|_| {
        CommandError::new(
            ErrorCode::LimitExceeded,
            "new_id",
            "the copy record identity could not be derived",
        )
    })
}

/// The copy's records: fresh operation ids, the same ordinals and the same plane parameters.
fn copied_records(
    records: &[CutRecord],
    new_id: &ElementId,
) -> Result<Vec<CutRecord>, CommandError> {
    records
        .iter()
        .enumerate()
        .map(|(ordinal, record)| {
            Ok(CutRecord {
                operation_id: copied_operation_id(record, ordinal, new_id)?,
                ..*record
            })
        })
        .collect()
}

/// The authored plate of the copy. Fails with `LIMIT_EXCEEDED` when the moved placement leaves
/// the coordinate range.
pub fn copy_plate(
    source: &AuthoredPlate,
    command: &CopyElement,
) -> Result<AuthoredPlate, CommandError> {
    command.validate_arguments()?;
    let mut next = source.clone();
    next.id = command.new_id;
    if let Some(name) = &command.name {
        next.name.clone_from(name);
    }
    next.cuts = copied_records(&source.cuts, &command.new_id)?;
    next.placement = copied_placement(&source.placement, command)?;
    Ok(next)
}

/// The authored member of the copy; see [`copy_plate`].
pub fn copy_member(
    source: &AuthoredMember,
    command: &CopyElement,
) -> Result<AuthoredMember, CommandError> {
    command.validate_arguments()?;
    let mut next = source.clone();
    next.id = command.new_id;
    if let Some(name) = &command.name {
        next.name.clone_from(name);
    }
    next.cuts = copied_records(&source.cuts, &command.new_id)?;
    next.placement = copied_placement(&source.placement, command)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::error::ErrorCode;
    use beamcraft_model::id::ElementId;

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn plate() -> AuthoredPlate {
        AuthoredPlate {
            id: id(5),
            name: "Plate".into(),
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            placement: Placement {
                translation_mm: [10.0, 20.0, 30.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    fn command() -> CopyElement {
        CopyElement {
            source_id: id(5),
            new_id: id(6),
            name: Some("Copy".into()),
            offset_mm: [100.0, 0.0, -5.0],
        }
    }

    #[test]
    fn a_copy_keeps_the_shape_and_moves_the_placement() {
        let copy = copy_plate(&plate(), &command()).unwrap();
        assert_eq!(copy.id, id(6));
        assert_eq!(copy.name, "Copy");
        assert_eq!(copy.placement.translation_mm, [110.0, 20.0, 25.0]);
        assert_eq!(copy.contour_points_mm, plate().contour_points_mm);
        assert_eq!(copy.thickness_mm, 10.0);
    }

    #[test]
    fn the_label_defaults_to_the_source() {
        let mut c = command();
        c.name = None;
        assert_eq!(copy_plate(&plate(), &c).unwrap().name, "Plate");
    }

    #[test]
    fn a_placement_out_of_range_is_refused() {
        let mut source = plate();
        source.placement.translation_mm[0] = 999_999_999.0;
        let mut c = command();
        c.offset_mm = [10.0, 0.0, 0.0];
        assert_eq!(
            copy_plate(&source, &c).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn an_equal_identity_is_refused() {
        let mut c = command();
        c.new_id = c.source_id;
        assert_eq!(
            copy_plate(&plate(), &c).unwrap_err().code,
            ErrorCode::DuplicateId
        );
    }

    #[test]
    fn a_copy_mints_fresh_record_ids_keeps_ordinals_and_is_deterministic() {
        use beamcraft_model::command::KeepSide;
        use beamcraft_model::id::OpId;
        let record = |ordinal: u16, point: [f64; 3]| CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000aa").unwrap(),
            ordinal,
            point_mm: point,
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        };
        let mut source = plate();
        source.cuts = vec![record(4, [250.0, 0.0, 0.0]), record(9, [150.0, 0.0, 0.0])];
        let copy = copy_plate(&source, &command()).unwrap();
        assert_eq!(copy.id, command().new_id);
        assert_eq!(copy.cuts.len(), 2);
        assert_eq!(copy.cuts[0].ordinal, 4);
        assert_eq!(copy.cuts[1].ordinal, 9);
        assert_eq!(copy.cuts[0].point_mm, [250.0, 0.0, 0.0]);
        assert_ne!(copy.cuts[0].operation_id, source.cuts[0].operation_id);
        assert_ne!(copy.cuts[1].operation_id, copy.cuts[0].operation_id);
        // A retried copy reproduces the exact same ids.
        let again = copy_plate(&source, &command()).unwrap();
        assert_eq!(again.cuts, copy.cuts);
        // A copy of another element mints other ids from the same source record.
        let mut other = command();
        other.new_id = ElementId::parse("019a7bf0-0000-7000-8000-0000000000ff").unwrap();
        let elsewhere = copy_plate(&source, &other).unwrap();
        assert_ne!(elsewhere.cuts[0].operation_id, copy.cuts[0].operation_id);
    }
}
