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

use crate::engine::command::AuthoredPlate;
use crate::engine::member::AuthoredMember;
use crate::model::command::CopyElement;
use crate::model::command::{CutRecord, HoleRecord};
use beamcraft_geom::canonical_f64_bytes;
use beamcraft_model::command::{CommandError, Placement};
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

/// Deterministic fresh identity of a duplicated A04 record. Instance copies retain the
/// canonical record identity instead; an independent duplicate must own a distinct one.
pub(crate) fn copied_hole(
    record: &HoleRecord,
    new_id: &ElementId,
) -> Result<HoleRecord, CommandError> {
    if record.polygon_mm.len() > beamcraft_model::limits::CONTOUR_POINT_CAP {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "hole",
            "copied hole exceeds the point cap",
        ));
    }
    let mut seed = b"BeamCraft/A04/CopyHoleRecord/v1\0".to_vec();
    seed.extend_from_slice(new_id.as_bytes());
    seed.extend_from_slice(record.operation_id.as_bytes());
    seed.extend_from_slice(&(record.ordinal as u64).to_le_bytes());
    for v in record.polygon_mm.iter().flatten() {
        seed.extend_from_slice(&canonical_f64_bytes(*v).map_err(|_| {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "hole",
                "copied hole coordinates must be finite",
            )
        })?);
    }
    let digest = Sha256::digest(&seed).map_err(|_| {
        CommandError::new(
            ErrorCode::LimitExceeded,
            "hole",
            "copied hole identity exceeds the digest limit",
        )
    })?;
    let mut bytes = [0u8; 16];
    for (out, value) in bytes.iter_mut().zip(digest.as_bytes()) {
        *out = *value;
    }
    if let Some(v) = bytes.get_mut(6) {
        *v = (*v & 0x0f) | 0x70;
    }
    if let Some(v) = bytes.get_mut(8) {
        *v = (*v & 0x3f) | 0x80;
    }
    let operation_id = Uuid7::from_bytes(bytes).map(OpId::from_uuid).map_err(|_| {
        CommandError::new(
            ErrorCode::InvalidId,
            "hole",
            "copied hole identity is invalid",
        )
    })?;
    Ok(HoleRecord {
        operation_id,
        // The duplicate is independent: an owner of the source record stays with the source,
        // so a copy or a split half is never governed by (or stripped by) another group (A14.4).
        owner: None,
        ..record.clone()
    })
}

/// The copy's records: fresh operation ids, the same ordinals and the same plane parameters.
pub(crate) fn copied_records(
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
    if source.hole_records.len() > crate::model::command::MAX_HOLES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "hole",
            "copied plate exceeds the hole record cap",
        ));
    }
    let hole_records = source
        .hole_records
        .iter()
        .map(|r| copied_hole(r, &command.new_id))
        .collect::<Result<Vec<_>, _>>()?;
    let mut next = source.clone();
    next.id = command.new_id;
    next.hole_records = hole_records;
    if let Some(name) = &command.name {
        next.name.clone_from(name);
    }
    next.cuts = copied_records(&source.cuts, &command.new_id)?;
    next.bends = copied_bends(&source.bends, &command.new_id)?;
    next.placement = copied_placement(&source.placement, command)?;
    Ok(next)
}

/// The copy's bend records (A10): fresh operation identities, the same specs.
fn copied_bends(
    records: &[crate::model::command::BendRecord],
    new_id: &ElementId,
) -> Result<Vec<crate::model::command::BendRecord>, CommandError> {
    records
        .iter()
        .map(|record| {
            let mut seed = b"BeamCraft/A10/CopyBendRecord/v1\0".to_vec();
            seed.extend_from_slice(new_id.as_bytes());
            seed.extend_from_slice(record.operation_id.as_bytes());
            let digest = Sha256::digest(&seed).map_err(|_| {
                CommandError::new(
                    ErrorCode::LimitExceeded,
                    "bend",
                    "copied bend identity exceeds the digest limit",
                )
            })?;
            let mut bytes = [0u8; 16];
            for (out, value) in bytes.iter_mut().zip(digest.as_bytes()) {
                *out = *value;
            }
            if let Some(v) = bytes.get_mut(6) {
                *v = (*v & 0x0f) | 0x70;
            }
            if let Some(v) = bytes.get_mut(8) {
                *v = (*v & 0x3f) | 0x80;
            }
            let operation_id = Uuid7::from_bytes(bytes).map(OpId::from_uuid).map_err(|_| {
                CommandError::new(
                    ErrorCode::InvalidId,
                    "bend",
                    "copied bend identity is invalid",
                )
            })?;
            Ok(crate::model::command::BendRecord {
                operation_id,
                spec: record.spec,
            })
        })
        .collect()
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
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement {
                translation_mm: [10.0, 20.0, 30.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
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
        use crate::model::command::KeepSide;
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

    #[test]
    fn a_copied_hole_id_is_deterministic_uuid7_and_bound_to_the_copy() {
        let record = HoleRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000aa").unwrap(),
            ordinal: 3,
            polygon_mm: vec![[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]],
            kind: crate::model::command::HoleKind::Hole,
            owner: None,
        };
        // A retried copy derives the exact same identity.
        let first = copied_hole(&record, &id(6)).unwrap();
        let again = copied_hole(&record, &id(6)).unwrap();
        assert_eq!(first.operation_id, again.operation_id);
        assert_ne!(first.operation_id, record.operation_id);
        // The derived identity carries the forced UUIDv7 version and RFC variant bits.
        let bytes = first.operation_id.as_bytes();
        assert_eq!(bytes[6] >> 4, 7);
        assert_eq!(bytes[8] >> 6, 2);
        // A copy under another identity mints another id; the rest of the record is kept.
        let elsewhere = copied_hole(&record, &id(7)).unwrap();
        assert_ne!(elsewhere.operation_id, first.operation_id);
        assert_eq!(elsewhere.ordinal, record.ordinal);
        assert_eq!(elsewhere.polygon_mm, record.polygon_mm);
    }

    #[test]
    fn a_copy_of_a_drilled_plate_drops_the_source_groups_ownership() {
        use crate::model::command::HoleKind;
        use beamcraft_model::id::OpId;
        let group = ElementId::parse("019a7bf0-0000-7000-8000-0000000000be").unwrap();
        let mut source = plate();
        source.hole_records = vec![HoleRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000aa").unwrap(),
            ordinal: 0,
            polygon_mm: vec![[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]],
            kind: HoleKind::Hole,
            owner: Some(group),
        }];
        let copy = copy_plate(&source, &command()).unwrap();
        assert_eq!(copy.hole_records.len(), 1);
        assert_eq!(copy.hole_records[0].ordinal, 0);
        assert_ne!(
            copy.hole_records[0].operation_id,
            source.hole_records[0].operation_id
        );
        // The duplicate is not governed by the source's generating group (A14.4).
        assert_eq!(copy.hole_records[0].owner, None);
    }
}
