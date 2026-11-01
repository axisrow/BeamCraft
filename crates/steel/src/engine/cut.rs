// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F04 command application for `CutByPlane` (A03, amendment A-2): one owned `plane_cut`
//! operation record appended to a valid plate, beam or column, replayed with the rest.
//!
//! The engine never touches USD. It checks the record admission the command layer owes (target
//! lifecycle, record identity, ordinal capacity), appends the record to the authored state and
//! declares the footprint; the adapter re-evaluates the body through the ordinary
//! [`crate::engine::command::evaluate_plate`] / [`crate::engine::member::evaluate_member`] replay and commits
//! the result as one transaction. A rejection changes nothing.
//!
//! Feature stability follows F06 section 5: the faces of the base body keep their ids, the cap
//! face of the new record is `cut:f.p<ordinal>`, and a feature the plane removes or splits no
//! longer resolves — the explicit loss, never a silent rename.

use crate::engine::command::AuthoredPlate;
use crate::model::command::{CutByPlane, CutRecord};
use beamcraft_engine::footprint::Footprint;
use beamcraft_model::command::CommandError;
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
    Member(crate::engine::member::AuthoredMember),
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
pub(crate) enum Found<'a> {
    Plate(&'a AuthoredPlate),
    Member(&'a crate::engine::member::AuthoredMember),
}

impl Found<'_> {
    fn cuts(&self) -> &[CutRecord] {
        match self {
            Self::Plate(authored) => &authored.cuts,
            Self::Member(authored) => &authored.cuts,
        }
    }

    /// The target with `record` appended as its next plane-cut record.
    pub(crate) fn appended(self, record: CutRecord) -> CutTarget {
        match self {
            Self::Plate(authored) => {
                let mut next = authored.clone();
                next.cuts.push(record);
                CutTarget::Plate(next)
            }
            Self::Member(authored) => {
                let mut next = authored.clone();
                next.cuts.push(record);
                CutTarget::Member(next)
            }
        }
    }
}

/// The shared admission of one appended operation record (F04 sections 4 and 8): the target
/// lifecycle, the record identity across the project and the per-element ordinal capacity.
/// Returns the found target and the next ordinal in creation order.
pub(crate) fn admit_record<'a>(
    target_id: ElementId,
    operation_id: OpId,
    plate: Option<&'a AuthoredPlate>,
    member: Option<&'a crate::engine::member::AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
) -> Result<(Found<'a>, u16), CommandError> {
    let found = match (plate, member) {
        (Some(authored), _) if authored.id == target_id => Found::Plate(authored),
        (_, Some(authored)) if authored.id == target_id => Found::Member(authored),
        _ => {
            return Err(CommandError::new(
                ErrorCode::NotFound,
                "target_id",
                "the element is not in the head revision",
            ));
        }
    };
    if operation_used(&operation_id) {
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
    Ok((found, ordinal.unwrap_or_default()))
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
    member: Option<&crate::engine::member::AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
) -> Result<CutByPlaneOutcome, CommandError> {
    command.validate_arguments()?;
    let (found, ordinal) = admit_record(
        command.target_id,
        command.operation_id,
        plate,
        member,
        operation_used,
    )?;
    let record = CutRecord {
        operation_id: command.operation_id,
        ordinal,
        point_mm: command.point_mm,
        normal: command.normal,
        keep: command.keep,
    };
    Ok(CutByPlaneOutcome {
        target: found.appended(record),
        ordinal,
        footprint: Footprint::for_update(command.target_id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
    })
}

/// Append using the admitted persistent next ordinal, including explicitly retired records.
/// The legacy API derives its first counter from live records; adapters with retirement metadata
/// use this API so deletion never reuses a semantic cap FeatureId.
pub fn cut_by_plane_after(
    command: &CutByPlane,
    plate: Option<&AuthoredPlate>,
    member: Option<&crate::engine::member::AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<CutByPlaneOutcome, CommandError> {
    command.validate_arguments()?;
    if next_ordinal > 999 {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "kernel_capacity",
            "cut ordinal capacity exhausted",
        ));
    }
    let result = cut_by_plane(command, plate, member, operation_used)?;
    pin_next_ordinal(result, next_ordinal)
}

/// Pins the appended record's ordinal to the persistent counter and refuses a counter that
/// regressed behind the live-derived one (see `cut_by_plane_after`). The shared postlude of
/// every adapter that allocates from retirement metadata.
pub(crate) fn pin_next_ordinal(
    mut result: CutByPlaneOutcome,
    next_ordinal: u16,
) -> Result<CutByPlaneOutcome, CommandError> {
    if next_ordinal < result.ordinal {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "next_ordinal",
            "cut counter regressed",
        ));
    }
    let cuts = match &mut result.target {
        CutTarget::Plate(p) => &mut p.cuts,
        CutTarget::Member(m) => &mut m.cuts,
    };
    let last = cuts.last_mut().ok_or_else(|| {
        CommandError::new(ErrorCode::InvalidArgument, "record", "appended cut missing")
    })?;
    last.ordinal = next_ordinal;
    result.ordinal = next_ordinal;
    Ok(result)
}

/// Replace a cut in memory without changing its record UUID, ordinal or placement. The
/// adapter evaluates the complete replay before publication, including all linked instances.
pub fn update_cut(
    command: &crate::model::command::UpdateCut,
    plate: Option<&AuthoredPlate>,
    member: Option<&crate::engine::member::AuthoredMember>,
) -> Result<CutByPlaneOutcome, CommandError> {
    command.validate_arguments()?;
    let c = &command.cut;
    let mut target = match (plate, member) {
        (Some(p), _) if p.id == c.target_id => CutTarget::Plate(p.clone()),
        (_, Some(m)) if m.id == c.target_id => CutTarget::Member(m.clone()),
        _ => {
            return Err(CommandError::new(
                ErrorCode::NotFound,
                "target_id",
                "cut target not found",
            ));
        }
    };
    let cuts = match &mut target {
        CutTarget::Plate(p) => &mut p.cuts,
        CutTarget::Member(m) => &mut m.cuts,
    };
    let r = cuts
        .iter_mut()
        .find(|r| r.operation_id == c.operation_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "operation_id",
                "cut record not found on target",
            )
        })?;
    r.point_mm = c.point_mm;
    r.normal = c.normal;
    r.keep = c.keep;
    let ordinal = r.ordinal;
    Ok(CutByPlaneOutcome {
        target,
        ordinal,
        footprint: Footprint::for_update(c.target_id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
    })
}

/// Remove one cut while preserving all surviving record IDs/ordinals and the placement.
/// The layer adapter retires its ID and advances/preserves the lifetime counter atomically.
pub fn remove_cut(
    command: &crate::model::command::RemoveCut,
    plate: Option<&AuthoredPlate>,
    member: Option<&crate::engine::member::AuthoredMember>,
) -> Result<CutByPlaneOutcome, CommandError> {
    let mut target = match (plate, member) {
        (Some(p), _) if p.id == command.target_id => CutTarget::Plate(p.clone()),
        (_, Some(m)) if m.id == command.target_id => CutTarget::Member(m.clone()),
        _ => {
            return Err(CommandError::new(
                ErrorCode::NotFound,
                "target_id",
                "cut target not found",
            ));
        }
    };
    let cuts = match &mut target {
        CutTarget::Plate(p) => &mut p.cuts,
        CutTarget::Member(m) => &mut m.cuts,
    };
    let ordinal = cuts
        .iter()
        .find(|r| r.operation_id == command.operation_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "operation_id",
                "cut record not found on target",
            )
        })?
        .ordinal;
    cuts.retain(|r| r.operation_id != command.operation_id);
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
    use crate::engine::command::evaluate_plate;
    use crate::model::command::KeepSide;
    use beamcraft_model::command::Placement;

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
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material: None,
            class: 0,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
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
            e.target == beamcraft_engine::footprint::Target::Element(id(5))
                && e.group == beamcraft_engine::footprint::Group::ShapeInputs
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
    #[test]
    fn update_keeps_record_identity_and_ordinal_and_has_its_own_fingerprint() {
        let p = plate();
        let c = CutByPlane {
            target_id: p.id,
            operation_id: op(60),
            point_mm: [800.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        };
        let added = cut_by_plane(&c, Some(&p), None, &|_| false).unwrap();
        let CutTarget::Plate(p) = added.target else {
            panic!("plate");
        };
        let edit = crate::model::command::UpdateCut {
            cut: CutByPlane {
                point_mm: [600.0, 0.0, 0.0],
                ..c
            },
        };
        let r = update_cut(&edit, Some(&p), None).unwrap();
        assert_eq!(r.ordinal, 0);
        assert_eq!(
            r.target.cuts().first().unwrap().operation_id,
            c.operation_id
        );
        assert_ne!(
            edit.content_fingerprint().unwrap(),
            edit.cut.content_fingerprint().unwrap()
        );
        assert!(
            update_cut(
                &crate::model::command::UpdateCut {
                    cut: CutByPlane {
                        operation_id: op(61),
                        ..c
                    }
                },
                Some(&p),
                None
            )
            .is_err()
        );
    }
    #[test]
    fn removal_keeps_surviving_ordinals_and_append_honors_retirement_capacity() {
        let p = plate();
        let c = CutByPlane {
            target_id: p.id,
            operation_id: op(70),
            point_mm: [800.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        };
        let CutTarget::Plate(p) = cut_by_plane(&c, Some(&p), None, &|_| false).unwrap().target
        else {
            panic!("plate");
        };
        let second = CutByPlane {
            operation_id: op(71),
            ..c
        };
        let CutTarget::Plate(p) = cut_by_plane_after(&second, Some(&p), None, &|_| false, 3)
            .unwrap()
            .target
        else {
            panic!("plate");
        };
        let r = remove_cut(
            &crate::model::command::RemoveCut {
                target_id: p.id,
                operation_id: op(70),
            },
            Some(&p),
            None,
        )
        .unwrap();
        assert_eq!(r.ordinal, 0);
        assert_eq!(r.target.cuts().len(), 1);
        assert_eq!(r.target.cuts().first().unwrap().ordinal, 3);
        assert!(cut_by_plane_after(&second, Some(&p), None, &|_| false, 1000).is_err());
        assert!(cut_by_plane_after(&second, Some(&p), None, &|_| false, 0).is_err());
    }
}
