// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Read side of P01: from the authored plates of a committed revision to the view contract.
//!
//! The adapter that reads USD hands over [`AuthoredPlate`] records (typed, decoded from the
//! committed model layer). This module re-evaluates each one with the same kernel path that
//! `CreatePlate` used, so a viewer receives genuine kernel meshes, quantities and a pinned
//! [`crate::ViewSnapshot`] built from committed data, never from a mock.

use crate::command::{AuthoredPlate, MaterialLookup, PlateBody, Quantities, evaluate_plate};
use crate::element::ElementKind;
use crate::member::{
    AuthoredMember, MemberBody, ProfileLookup, evaluate_member, member_quantities,
};
use crate::scene::SceneElement;
use beamcraft_geom::Translation3;
use beamcraft_model::command::{CommandError, MemberKind, label_is_valid};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::BTreeSet;

/// A committed plate with its re-evaluated body.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateRecord {
    /// The authored state read from the committed model layer.
    pub authored: AuthoredPlate,
    /// The body the kernel evaluates from it.
    pub body: PlateBody,
    /// The quantity records of that body.
    pub quantities: Quantities,
}

/// Re-evaluates committed authored plates, ordered by element id.
///
/// An element id listed twice is `DUPLICATE_ID`. A plate that no longer evaluates (a model edited
/// outside BeamCraft, for example) is rejected with the evaluation's own code and no partial
/// list. The label rule is re-checked because the layer is untrusted input. A pinned material
/// that `materials` cannot resolve is rejected with the lookup's code (`NOT_FOUND`): the mass of
/// a plate is never silently dropped.
pub fn evaluate_committed(
    plates: &[AuthoredPlate],
    materials: MaterialLookup<'_>,
) -> Result<Vec<PlateRecord>, CommandError> {
    let mut seen = BTreeSet::new();
    let mut records = Vec::with_capacity(plates.len().min(crate::limits::MAX_ELEMENTS));
    for authored in plates {
        if !seen.insert(authored.id) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "id",
                "element id appears twice",
            ));
        }
        if !label_is_valid(&authored.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "committed label is not valid",
            ));
        }
        authored.placement.validate()?;
        let material = authored.material.as_ref().map(materials).transpose()?;
        let body = evaluate_plate(authored)?;
        let quantities = body.quantities_for(material)?;
        records.push(PlateRecord {
            authored: authored.clone(),
            body,
            quantities,
        });
    }
    records.sort_by_key(|r| r.authored.id);
    Ok(records)
}

/// A committed member with its re-evaluated body.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberRecord {
    /// The authored state read from the committed model layer.
    pub authored: AuthoredMember,
    /// The body the kernel evaluates from it and its profile.
    pub body: MemberBody,
    /// The quantity records of that body.
    pub quantities: Quantities,
}

/// Re-evaluates committed authored members, ordered by element id.
///
/// The pinned profile and material are resolved through the lookups; a definition that cannot
/// be resolved is rejected with the lookup's code (`NOT_FOUND`), never dropped. The label rule
/// and the placement are re-checked because the layer is untrusted input.
pub fn evaluate_committed_members(
    members: &[AuthoredMember],
    profiles: ProfileLookup<'_>,
    materials: MaterialLookup<'_>,
) -> Result<Vec<MemberRecord>, CommandError> {
    let mut seen = BTreeSet::new();
    let mut records = Vec::with_capacity(members.len().min(crate::limits::MAX_ELEMENTS));
    for authored in members {
        if !seen.insert(authored.id) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "id",
                "element id appears twice",
            ));
        }
        if !label_is_valid(&authored.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "committed label is not valid",
            ));
        }
        authored.placement.validate()?;
        let profile = profiles(&authored.profile)?;
        let material = authored.material.as_ref().map(materials).transpose()?;
        let body = evaluate_member(authored, profile)?;
        let quantities = member_quantities(&body, material)?;
        records.push(MemberRecord {
            authored: authored.clone(),
            body,
            quantities,
        });
    }
    records.sort_by_key(|r| r.authored.id);
    Ok(records)
}

/// The snapshot-builder input for committed plates. Each placement must be a pure translation
/// (identity rotation); another rotation is `UNSUPPORTED_GEOMETRY`, because the view contract
/// carries translations only.
pub fn scene_elements(records: &[PlateRecord]) -> Result<Vec<SceneElement>, CommandError> {
    scene_elements_with_members(records, &[])
}

/// The snapshot-builder input for committed plates and members, in element-id order.
pub fn scene_elements_with_members(
    plates: &[PlateRecord],
    members: &[MemberRecord],
) -> Result<Vec<SceneElement>, CommandError> {
    let translation = |placement: &beamcraft_model::command::Placement| {
        if !placement.is_identity_rotation() {
            return Err(CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "placement.orientation_xyzw",
                "rotated placement is not evaluated in this slice",
            ));
        }
        let [x, y, z] = placement.translation_mm;
        Translation3::new(x, y, z).map_err(|_| {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "placement.translation_mm",
                "value is not finite",
            )
        })
    };
    let mut elements = Vec::with_capacity(plates.len().saturating_add(members.len()));
    for r in plates {
        elements.push(SceneElement::new(
            r.authored.id,
            ElementKind::Plate,
            translation(&r.authored.placement)?,
            r.body.solid.scene_mesh(),
        ));
    }
    for r in members {
        let kind = match r.authored.kind {
            MemberKind::Beam => ElementKind::Beam,
            MemberKind::Column => ElementKind::Column,
        };
        elements.push(SceneElement::new(
            r.authored.id,
            kind,
            translation(&r.authored.placement)?,
            r.body.solid.scene_mesh(),
        ));
    }
    elements.sort_by_key(SceneElement::id);
    Ok(elements)
}

/// The ids of committed plates, for the `DUPLICATE_ID` check of a new command.
pub fn ids(records: &[PlateRecord]) -> BTreeSet<ElementId> {
    records.iter().map(|r| r.authored.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{MaterialInfo, no_materials};
    use beamcraft_model::command::Placement;
    use beamcraft_model::digest::{ContentDigest, Sha256};

    // Shadows the glob import: these tests have no material definitions unless they say so.
    fn evaluate_committed(plates: &[AuthoredPlate]) -> Result<Vec<PlateRecord>, CommandError> {
        super::evaluate_committed(plates, &no_materials)
    }

    fn plate(n: u8, width: f64, x: f64) -> AuthoredPlate {
        AuthoredPlate {
            id: ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap(),
            name: format!("Plate {n}"),
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [width, 0.0, 0.0],
                [width, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            placement: Placement {
                translation_mm: [x, 0.0, 0.0],
                ..Placement::IDENTITY
            },
            material: None,
            holes_mm: Vec::new(),
        }
    }

    #[test]
    fn committed_members_join_plates_in_one_id_ordered_scene() {
        use crate::member::ProfileInfo;
        let profile: ContentDigest = Sha256::digest(b"profile").unwrap();
        let profiles = |d: &ContentDigest| {
            if *d == profile {
                Ok(ProfileInfo::Rectangle {
                    width_mm: 200.0,
                    height_mm: 300.0,
                    declared_area_mm2: None,
                })
            } else {
                crate::member::no_profiles(d)
            }
        };
        let member = |n: u8, kind: MemberKind| AuthoredMember {
            id: ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap(),
            kind,
            name: format!("Member {n}"),
            axis_points_mm: [[0.0; 3], [0.0, 0.0, 3000.0]],
            profile,
            roll_rad: None,
            placement: Placement::IDENTITY,
            material: None,
        };
        let members = super::evaluate_committed_members(
            &[member(8, MemberKind::Column), member(3, MemberKind::Beam)],
            &profiles,
            &no_materials,
        )
        .unwrap();
        assert!(members[0].authored.id < members[1].authored.id);
        assert_eq!(members[0].quantities.volume_mm3, 200.0 * 300.0 * 3000.0);
        let plates = evaluate_committed(&[plate(5, 1000.0, 0.0)]).unwrap();
        let scene = scene_elements_with_members(&plates, &members).unwrap();
        let ids: Vec<_> = scene.iter().map(SceneElement::id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert_eq!(scene.len(), 3);
        // A dangling profile is rejected, never dropped.
        let mut dangling = member(9, MemberKind::Beam);
        dangling.profile = Sha256::digest(b"absent").unwrap();
        assert_eq!(
            super::evaluate_committed_members(&[dangling], &profiles, &no_materials)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn a_committed_material_gives_the_plate_its_mass_and_a_dangling_one_is_rejected() {
        let steel: ContentDigest = Sha256::digest(b"steel").unwrap();
        let lookup = |d: &ContentDigest| {
            if *d == steel {
                Ok(MaterialInfo {
                    density_kg_m3: Some(1000.0),
                })
            } else {
                no_materials(d)
            }
        };
        let mut heavy = plate(5, 1000.0, 0.0);
        heavy.material = Some(steel);
        let records =
            super::evaluate_committed(&[heavy.clone(), plate(6, 1000.0, 0.0)], &lookup).unwrap();
        assert_eq!(records[0].quantities.mass_kg, Some(5.0));
        assert_eq!(records[1].quantities.mass_kg, None);
        heavy.material = Some(Sha256::digest(b"absent").unwrap());
        assert_eq!(
            super::evaluate_committed(&[heavy], &lookup)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn committed_plates_are_evaluated_sorted_and_become_a_scene() {
        let records =
            evaluate_committed(&[plate(9, 800.0, 0.0), plate(5, 1000.0, 2000.0)]).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records[0].authored.id < records[1].authored.id);
        assert_eq!(records[0].quantities.volume_mm3, 5_000_000.0);
        assert_eq!(records[1].quantities.volume_mm3, 4_000_000.0);
        let scene = scene_elements(&records).unwrap();
        assert_eq!(scene.len(), 2);
        assert_eq!(scene[0].id(), records[0].authored.id);
        assert_eq!(ids(&records).len(), 2);
    }

    #[test]
    fn duplicates_bad_labels_and_unsupported_rotation_are_rejected_without_a_partial_list() {
        assert_eq!(
            evaluate_committed(&[plate(5, 1000.0, 0.0), plate(5, 800.0, 0.0)])
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        let mut bad = plate(5, 1000.0, 0.0);
        bad.name = "line\nbreak".into();
        assert_eq!(
            evaluate_committed(&[bad]).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let mut rotated = plate(5, 1000.0, 0.0);
        let s = core::f64::consts::FRAC_1_SQRT_2;
        rotated.placement.orientation_xyzw = [0.0, 0.0, s, s];
        let records = evaluate_committed(&[rotated]).unwrap();
        assert_eq!(
            scene_elements(&records).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
        let mut broken = plate(5, 1000.0, 0.0);
        broken.thickness_mm = -1.0;
        assert!(evaluate_committed(&[broken]).is_err());
    }
}
