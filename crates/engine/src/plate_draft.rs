// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F04 authored plate admission independently of supported solid evaluation.
//! An unsupported authored plate has no mesh, bounds, quantity value or current key.

use crate::command::{AuthoredPlate, CreatePlateOutcome, MaterialLookup, create_plate};
use crate::footprint::Footprint;
use crate::query::committed::{BodyCache, PlateRecord};
use crate::transaction::PriorOp;
use beamcraft_model::command::{
    CommandError, CreatePlate, HOLE_POINT_CAP, MAX_HOLES, label_is_valid,
};
use beamcraft_model::definition::MaterialSpec;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use std::collections::BTreeSet;

/// Valid authored input whose solid is outside the supported contour evaluation scope.
#[derive(Debug, Clone, PartialEq)]
pub struct UnsupportedPlate {
    /// Original inputs, with no repaired or projected coordinates.
    pub authored: AuthoredPlate,
    /// Stable reason for the absent geometry and quantity values.
    pub reason: &'static str,
}

/// Result of admitting authored data; geometry evaluation remains a distinct state.
#[derive(Debug, Clone, PartialEq)]
pub enum PlateDraft {
    /// Complete supported body and quantities.
    Evaluated(Box<CreatePlateOutcome>),
    /// Valid authored data, with no partial geometry or quantity values.
    Unsupported(Box<UnsupportedPlate>),
}

/// Admits a plate as authored data. Invalid, over-cap or uncertain contours are still refused.
/// Only validated contours outside the implemented plane/rectangle scope can be retained;
/// operations on an unsupported contour still require their own supported adapters.
pub fn create_plate_draft(
    command: &CreatePlate,
    existing: &dyn Fn(&ElementId) -> bool,
    materials: MaterialLookup<'_>,
) -> Result<PlateDraft, CommandError> {
    let error = match create_plate(command, existing, materials) {
        Ok(outcome) => return Ok(PlateDraft::Evaluated(Box::new(outcome))),
        Err(error) => error,
    };
    if error.code != ErrorCode::UnsupportedGeometry
        || error.field != "contour_points_mm"
        || !command.holes_mm.is_empty()
    {
        return Err(error);
    }
    // Repeat the complete geometric admission: an arbitrary evaluation failure must never
    // become an accepted authored-only draft (in particular numeric uncertainty and bad topology).
    let contour =
        beamcraft_kernel::plate_contour::admit(&command.contour_points_mm, command.thickness_mm)?;
    let unsupported_plane =
        command.contour_points_mm.iter().any(|p| p[2] != 0.0) && contour.rectangle_mm.is_none();
    if !unsupported_plane {
        return Err(error);
    }
    if let Some(material) = command
        .material_digest
        .as_ref()
        .map(materials)
        .transpose()?
    {
        MaterialSpec {
            density_kg_m3: material.density_kg_m3,
            ..MaterialSpec::default()
        }
        .validate()?;
    }
    Ok(PlateDraft::Unsupported(Box::new(UnsupportedPlate {
        authored: AuthoredPlate::from_command(command),
        reason: if contour.exactly_planar {
            "contour_plane_unsupported"
        } else {
            "contour_not_exactly_planar"
        },
    })))
}

/// One admitted authored operation, before the host atomically commits it.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftTransaction {
    /// Operation identity for replay and outcome lookup.
    pub op_id: OpId,
    /// Authored input with either a complete body or explicit unsupported geometry.
    pub outcome: PlateDraft,
    /// Digest of the unmodified typed command arguments.
    pub fingerprint: ContentDigest,
    /// Declared write set, including the absent/replaced derived results.
    pub footprint: Footprint,
}

/// Pure application/replay decision for authored plate drafts.
#[derive(Debug, Clone, PartialEq)]
pub enum DraftAdmission {
    /// Apply only after atomic host publication.
    Apply(Box<DraftTransaction>),
    /// Return the recorded result without another revision.
    Replay,
}

/// F04 admission with the same replay/reuse ordering as evaluated commands.
pub fn admit_create_plate_draft(
    op_id: OpId,
    command: &CreatePlate,
    prior: PriorOp,
    existing: &dyn Fn(&ElementId) -> bool,
    materials: MaterialLookup<'_>,
) -> Result<DraftAdmission, CommandError> {
    if let PriorOp::Accepted { fingerprint } = prior {
        return if command.content_fingerprint()? == fingerprint {
            Ok(DraftAdmission::Replay)
        } else {
            Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            ))
        };
    }
    let outcome = create_plate_draft(command, existing, materials)?;
    Ok(DraftAdmission::Apply(Box::new(DraftTransaction {
        op_id,
        outcome,
        fingerprint: command.content_fingerprint()?,
        footprint: Footprint::for_create(command.new_id, command.material_digest)?,
    })))
}

/// Re-evaluates committed authored plates without dropping valid unsupported elements.
/// The two lists are disjoint and ordered by identity; malformed input fails the whole read.
pub fn evaluate_committed_drafts(
    plates: &[AuthoredPlate],
    materials: MaterialLookup<'_>,
) -> Result<(Vec<PlateRecord>, Vec<UnsupportedPlate>), CommandError> {
    evaluate_committed_drafts_cached(plates, materials, &mut BodyCache::disabled())
}

/// [`evaluate_committed_drafts`] with a caller-owned [`BodyCache`]: a body already evaluated for
/// bit-identical shape inputs is shared instead of evaluated again, and every refusal is the
/// same as without a cache. The per-plate checks, their order and the unsupported-classification
/// rules are those of the uncached call.
pub fn evaluate_committed_drafts_cached(
    plates: &[AuthoredPlate],
    materials: MaterialLookup<'_>,
    cache: &mut BodyCache,
) -> Result<(Vec<PlateRecord>, Vec<UnsupportedPlate>), CommandError> {
    if plates.len() > crate::limits::MAX_ELEMENTS {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "elements",
            "too many elements",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut evaluated = Vec::new();
    let mut unsupported = Vec::new();
    for authored in plates {
        // to_command owns its fields: bound every variable-sized field before cloning it.
        if !label_is_valid(&authored.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "committed label is invalid",
            ));
        }
        if authored.contour_points_mm.len() > 1024
            || authored.holes_mm.len() > MAX_HOLES
            || authored.hole_records.len() > MAX_HOLES
            || authored
                .holes_mm
                .iter()
                .chain(authored.hole_records.iter().map(|r| &r.polygon_mm))
                .any(|h| h.len() > HOLE_POINT_CAP)
        {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "contour_points_mm",
                "committed plate exceeds count caps",
            ));
        }
        if seen.contains(&authored.id) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "id",
                "element id appears twice",
            ));
        }
        authored.placement.validate()?;
        let material = authored.material.as_ref().map(materials).transpose()?;
        match cache.plate(authored) {
            Ok(body) => {
                let quantities = body.quantities_for(material)?;
                evaluated.push(PlateRecord {
                    authored: authored.clone(),
                    body,
                    quantities,
                });
            }
            Err(error) => {
                match create_plate_draft(&authored.to_command(), &|_| false, materials)? {
                    PlateDraft::Unsupported(record) => unsupported.push(*record),
                    // The cache only mirrors `evaluate_plate`, so an evaluation it refuses is a
                    // kernel refusal; the draft admission classifying it differently would mean
                    // the two evaluations disagreed, which is never expected.
                    PlateDraft::Evaluated(_) => return Err(error),
                }
            }
        }
        seen.insert(authored.id);
    }
    evaluated.sort_by_key(|p| p.authored.id);
    unsupported.sort_by_key(|p| p.authored.id);
    Ok((evaluated, unsupported))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::no_materials;
    use beamcraft_model::command::Placement;

    fn plate(n: u8, orientation_xyzw: [f64; 4]) -> AuthoredPlate {
        AuthoredPlate {
            id: ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap(),
            name: format!("Plate {n}"),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            placement: Placement {
                translation_mm: [0.0, 0.0, 0.0],
                orientation_xyzw,
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    #[test]
    fn rotated_and_identity_plates_share_the_same_cached_local_body() {
        let s = core::f64::consts::FRAC_1_SQRT_2;
        let mut cache = BodyCache::new();
        let rotated = [plate(1, [0.0, 0.0, s, s]), plate(2, [0.0, 0.0, 0.0, 1.0])];
        let (evaluated, unsupported) =
            evaluate_committed_drafts_cached(&rotated, &no_materials, &mut cache).unwrap();
        assert!(unsupported.is_empty());
        assert_eq!(evaluated.len(), 2);
        assert_eq!(evaluated[0].body.solid, evaluated[1].body.solid);
        // The identity neighbour still evaluates, cached or not.
        let (evaluated, unsupported) = evaluate_committed_drafts_cached(
            &[plate(2, [0.0, 0.0, 0.0, 1.0])],
            &no_materials,
            &mut cache,
        )
        .unwrap();
        assert!(unsupported.is_empty());
        assert_eq!(evaluated.len(), 1);
    }
}
