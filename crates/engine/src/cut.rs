// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F04 command application for `CutByPlane` (A03, amendment A-2): one owned `plane_cut`
//! operation record appended to a valid plate, beam or column, replayed with the rest.
//!
//! The engine never touches USD. It checks the record admission the command layer owes (target
//! lifecycle, record identity, ordinal capacity), appends the record to the authored state and
//! declares the footprint; the adapter re-evaluates the body through the ordinary
//! [`crate::command::evaluate_plate`] / [`crate::member::evaluate_member`] replay and commits
//! the result as one transaction. A rejection changes nothing.
//!
//! Feature stability follows F06 section 5: the faces of the base body keep their ids, the cap
//! face of the new record is `cut:f.p<ordinal>`, and a feature the plane removes or splits no
//! longer resolves — the explicit loss, never a silent rename.

use crate::command::AuthoredPlate;
use crate::footprint::Footprint;
use beamcraft_model::command::{CommandError, CutByPlane, CutRecord};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};

/// Largest operation ordinal the kernel names a cap face with (`cut:f.p<ordinal>` inside the
/// 64-byte FeatureId bound); one more record on one element is an implementation limit.
const MAX_ORDINAL: u16 = 999;

/// The next authored state of the cut target: exactly one of the two is set.
#[derive(Debug, Clone, PartialEq)]
pub enum CutTarget {
    /// A plate with the record appended.
    Plate(AuthoredPlate),
    /// A beam or column with the record appended.
    Member(crate::member::AuthoredMember),
}

impl CutTarget {
    /// The element the record was appended to.
    pub const fn id(&self) -> ElementId {
        match self {
            Self::Plate(authored) => authored.id,
            Self::Member(authored) => authored.id,
        }
    }

    /// The plane-cut records of the element in ascending ordinal order.
    pub fn cuts(&self) -> &[CutRecord] {
        match self {
            Self::Plate(authored) => &authored.cuts,
            Self::Member(authored) => &authored.cuts,
        }
    }
}

/// Everything one accepted `CutByPlane` produces for the adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct CutByPlaneOutcome {
    /// The target's authored state with the record appended.
    pub target: CutTarget,
    /// The ordinal the engine assigned to the new record.
    pub ordinal: u16,
    /// Declared read and write sets.
    pub footprint: Footprint,
    /// Digest of the typed arguments, for replay and reuse decisions.
    pub fingerprint: ContentDigest,
}

/// The found target, borrowed for the record admission.
enum Found<'a> {
    Plate(&'a AuthoredPlate),
    Member(&'a crate::member::AuthoredMember),
}

impl Found<'_> {
    fn cuts(&self) -> &[CutRecord] {
        match self {
            Self::Plate(authored) => &authored.cuts,
            Self::Member(authored) => &authored.cuts,
        }
    }
}

/// Applies `CutByPlane` to the target's authored state in memory: argument admission, target
/// lifecycle, record identity, ordinal capacity, then the append. Nothing is evaluated here;
/// the adapter re-evaluates the body and refuses the whole command if the replay does.
///
/// `operation_used` reports whether a record identity is already used in the project
/// (`DUPLICATE_ID`); the adapter closes it over every element's records.
pub fn cut_by_plane(
    command: &CutByPlane,
    plate: Option<&AuthoredPlate>,
    member: Option<&crate::member::AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
) -> Result<CutByPlaneOutcome, CommandError> {
    command.validate_arguments()?;
    let found = match (plate, member) {
        (Some(authored), _) if authored.id == command.target_id => Found::Plate(authored),
        (_, Some(authored)) if authored.id == command.target_id => Found::Member(authored),
        _ => {
            return Err(CommandError::new(
                ErrorCode::NotFound,
                "target_id",
                "the element is not in the head revision",
            ));
        }
    };
    if operation_used(&command.operation_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "operation_id",
            "operation record id is already owned",
        ));
    }
    // Ordinals come from the per-element counter in creation order and are never reused; the
    // records are appended in order, so the next ordinal follows the last one. Running past the
    // kernel's naming capacity is a capacity result, never invalid geometry.
    let ordinal = found
        .cuts()
        .last()
        .map_or(Some(0), |last| last.ordinal.checked_add(1));
    if !matches!(&ordinal, Some(ordinal) if *ordinal <= MAX_ORDINAL) {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "kernel_capacity",
            "the element has no operation ordinal left",
        ));
    }
    let ordinal = ordinal.unwrap_or_default();
    let record = CutRecord {
        operation_id: command.operation_id,
        ordinal,
        point_mm: command.point_mm,
        normal: command.normal,
        keep: command.keep,
    };
    let target = match found {
        Found::Plate(authored) => {
            let mut next = authored.clone();
            next.cuts.push(record);
            CutTarget::Plate(next)
        }
        Found::Member(authored) => {
            let mut next = authored.clone();
            next.cuts.push(record);
            CutTarget::Member(next)
        }
    };
    Ok(CutByPlaneOutcome {
        target,
        ordinal,
        footprint: Footprint::for_update(command.target_id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::evaluate_plate;
    use beamcraft_model::command::{KeepSide, Placement};

    fn id(byte: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{byte:02x}")).unwrap()
    }

    fn op(byte: u8) -> OpId {
        OpId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{byte:02x}")).unwrap()
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
            thickness_side: None,
            placement: Placement::IDENTITY,
            material: None,
            class: 0,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    fn command(target: ElementId) -> CutByPlane {
        CutByPlane {
            target_id: target,
            operation_id: op(200),
            point_mm: [250.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        }
    }

    fn none(_: &OpId) -> bool {
        false
    }

    #[test]
    fn a_cut_appends_the_record_with_the_next_ordinal_and_declares_the_footprint() {
        let source = plate();
        let outcome = cut_by_plane(&command(id(5)), Some(&source), None, &none).unwrap();
        let CutTarget::Plate(authored) = &outcome.target else {
            panic!("expected the plate target");
        };
        assert_eq!(authored.cuts.len(), 1);
        assert_eq!(authored.cuts[0].ordinal, 0);
        assert_eq!(outcome.ordinal, 0);
        assert_eq!(
            outcome.fingerprint,
            command(id(5)).content_fingerprint().unwrap()
        );
        assert!(outcome.footprint.writes().iter().any(|e| {
            e.target == crate::footprint::Target::Element(id(5))
                && e.group == crate::footprint::Group::ShapeInputs
        }));
        // The replay of the appended record gives the exact kept volume.
        let body = evaluate_plate(authored).unwrap();
        assert!((body.quantities().volume_mm3 - 1_250_000.0).abs() <= 1e-6);
        // A second cut takes the next ordinal, in creation order.
        let next = CutByPlane {
            operation_id: op(201),
            point_mm: [150.0, 0.0, 0.0],
            keep: KeepSide::Positive,
            ..command(id(5))
        };
        let outcome = cut_by_plane(&next, Some(authored), None, &none).unwrap();
        assert_eq!(outcome.ordinal, 1);
    }

    #[test]
    fn a_missing_target_a_used_record_or_an_exhausted_counter_is_refused() {
        let source = plate();
        assert_eq!(
            cut_by_plane(&command(id(9)), Some(&source), None, &none)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        let used = |o: &OpId| *o == op(200);
        assert_eq!(
            cut_by_plane(&command(id(5)), Some(&source), None, &used)
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        // The kernel names caps only up to ordinal 999; one more record is a capacity result.
        let mut full = plate();
        full.cuts = (0..=MAX_ORDINAL)
            .map(|ordinal| CutRecord {
                operation_id: op(200),
                ordinal,
                point_mm: [250.0, 0.0, 0.0],
                normal: [1.0, 0.0, 0.0],
                keep: KeepSide::Negative,
            })
            .collect();
        let refused = cut_by_plane(&command(id(5)), Some(&full), None, &none).unwrap_err();
        assert_eq!(refused.code, ErrorCode::LimitExceeded);
        assert_eq!(refused.field, "kernel_capacity");
    }
}
