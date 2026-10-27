// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Read side of P01: from the authored plates of a committed revision to the view contract.
//!
//! The adapter that reads USD hands over [`AuthoredPlate`] records (typed, decoded from the
//! committed model layer). This module re-evaluates each one with the same kernel path that
//! `CreatePlate` used, so a viewer receives genuine kernel meshes, quantities and a pinned
//! [`crate::ViewSnapshot`] built from committed data, never from a mock. A [`BodyCache`] lets
//! elements with bit-identical shape inputs share one kernel evaluation.

use crate::command::{AuthoredPlate, MaterialLookup, PlateBody, Quantities, evaluate_plate};
use crate::element::ElementKind;
use crate::limits::MAX_ELEMENTS;
use crate::member::{
    AuthoredMember, MemberBody, ProfileInfo, ProfileLookup, evaluate_member, member_quantities,
};
use crate::scene::SceneElement;
use crate::solid::SolidMesh;
use beamcraft_geom::Translation3;
use beamcraft_model::command::{
    CommandError, CornerTreatment, CutRecord, KeepSide, MemberKind, PlateSide, label_is_valid,
};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use beamcraft_model::section::{CustomSection, SectionShape};
use std::collections::{BTreeMap, BTreeSet};

/// Bytes of keys and bodies that a [`BodyCache`] holds at most: 32 MiB.
pub const BODY_CACHE_BYTES: usize = 32 * 1024 * 1024;

/// Kernel bodies already evaluated, by the exact bits of their complete shape inputs (issue 26).
///
/// A plate body is a function of the plate's contour, thickness, holes and cut alone, and a
/// member body of the member's axis, roll, cut and resolved profile: identity, label, kind,
/// placement and material never reach the kernel. The key is an injective byte encoding of
/// exactly those inputs. Every list is length-prefixed, every option and variant is tagged and
/// every `f64` is stored as its raw bits, so `-0.0` and `0.0` are different keys, and no digest,
/// stored or computed, ever stands in for an input. The profile enters by its resolved content,
/// not by the digest that pins it.
///
/// The cache never outlives its owner, so the algorithm of every entry is the running build's,
/// and a changed input is a different key: nothing is ever invalidated in place. It holds at
/// most [`MAX_ELEMENTS`] entries and [`BODY_CACHE_BYTES`] bytes of keys and bodies; an insert
/// that would exceed either clears it first, and a body larger than the whole budget is returned
/// without being kept. A failed evaluation is never stored, so a refusal always comes from fresh
/// kernel work. The cache replaces kernel calls only: whoever reads a committed revision still
/// compares its stored mesh with the body returned here.
#[derive(Debug)]
pub struct BodyCache {
    plates: BTreeMap<Vec<u8>, PlateBody>,
    members: BTreeMap<Vec<u8>, MemberBody>,
    bytes: usize,
    budget: usize,
    hits: u64,
    misses: u64,
}

impl Default for BodyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BodyCache {
    /// An empty cache with the full [`BODY_CACHE_BYTES`] budget.
    pub const fn new() -> Self {
        Self::with_budget(BODY_CACHE_BYTES)
    }

    /// An empty cache with a smaller budget; a larger one is lowered to [`BODY_CACHE_BYTES`].
    pub const fn with_budget(bytes: usize) -> Self {
        Self {
            plates: BTreeMap::new(),
            members: BTreeMap::new(),
            bytes: 0,
            budget: if bytes < BODY_CACHE_BYTES {
                bytes
            } else {
                BODY_CACHE_BYTES
            },
            hits: 0,
            misses: 0,
        }
    }

    /// A cache that keeps nothing and builds no key: every call evaluates.
    pub const fn disabled() -> Self {
        Self::with_budget(0)
    }

    /// Calls answered from the cache since it was created (a diagnostic, never model data).
    pub const fn hits(&self) -> u64 {
        self.hits
    }

    /// Calls that evaluated with the kernel since the cache was created; a disabled cache
    /// counts none.
    pub const fn misses(&self) -> u64 {
        self.misses
    }

    /// The body of a plate: the kept one for bit-identical shape inputs, otherwise
    /// [`evaluate_plate`]'s result, kept when it fits.
    pub fn plate(&mut self, plate: &AuthoredPlate) -> Result<PlateBody, CommandError> {
        if self.budget == 0 {
            return evaluate_plate(plate);
        }
        let key = plate_key(plate);
        if let Some(body) = self.plates.get(&key) {
            self.hits = self.hits.saturating_add(1);
            return Ok(body.clone());
        }
        self.misses = self.misses.saturating_add(1);
        let body = evaluate_plate(plate)?;
        if self.admit(key.len().saturating_add(solid_bytes(&body.solid))) {
            self.plates.insert(key, body.clone());
        }
        Ok(body)
    }

    /// The body of a member with its resolved profile: the kept one for bit-identical shape
    /// inputs and profile content, otherwise [`evaluate_member`]'s result, kept when it fits.
    pub fn member(
        &mut self,
        member: &AuthoredMember,
        profile: ProfileInfo,
    ) -> Result<MemberBody, CommandError> {
        // Placement is excluded from the body key, but every caller still receives admission.
        member.placement.validate()?;
        if self.budget == 0 {
            return evaluate_member(member, profile);
        }
        let key = member_key(member, &profile);
        if let Some(body) = self.members.get(&key) {
            self.hits = self.hits.saturating_add(1);
            return Ok(body.clone());
        }
        self.misses = self.misses.saturating_add(1);
        let body = evaluate_member(member, profile)?;
        // The key holds the profile content, so it also accounts for the profile in the body.
        if self.admit(key.len().saturating_add(solid_bytes(&body.solid))) {
            self.members.insert(key, body.clone());
        }
        Ok(body)
    }

    /// Makes room for an entry of `size` bytes: clears the cache when the entry would exceed
    /// the byte budget or the entry cap, and refuses an entry larger than the whole budget.
    fn admit(&mut self, size: usize) -> bool {
        if size > self.budget {
            return false;
        }
        let entries = self.plates.len().saturating_add(self.members.len());
        if self.bytes.saturating_add(size) > self.budget || entries >= MAX_ELEMENTS {
            self.plates.clear();
            self.members.clear();
            self.bytes = 0;
        }
        self.bytes = self.bytes.saturating_add(size);
        true
    }
}

/// Bytes a kept body occupies: its blob, its `f64` vertices and its `u32` triangles.
fn solid_bytes(solid: &SolidMesh) -> usize {
    solid
        .blob()
        .len()
        .saturating_add(solid.vertices().len().saturating_mul(24))
        .saturating_add(solid.triangles().len().saturating_mul(12))
}

fn push_len(key: &mut Vec<u8>, len: usize) {
    key.extend_from_slice(&(len as u64).to_le_bytes());
}

fn push_f64(key: &mut Vec<u8>, value: f64) {
    key.extend_from_slice(&value.to_bits().to_le_bytes());
}

fn push_option(key: &mut Vec<u8>, value: Option<f64>) {
    match value {
        Some(value) => {
            key.push(1);
            push_f64(key, value);
        }
        None => key.push(0),
    }
}

fn push_points<const N: usize>(key: &mut Vec<u8>, points: &[[f64; N]]) {
    push_len(key, points.len());
    for value in points.iter().flatten() {
        push_f64(key, *value);
    }
}

fn push_cut(key: &mut Vec<u8>, cuts: &[CutRecord]) {
    push_len(key, cuts.len());
    for cut in cuts {
        push_f64(key, f64::from(cut.ordinal));
        key.push(match cut.keep {
            KeepSide::Positive => 1,
            KeepSide::Negative => 0,
        });
        for value in cut.point_mm.iter().chain(cut.normal.iter()) {
            push_f64(key, *value);
        }
    }
}

/// The key of a plate body. Every field is named, so a field added to [`AuthoredPlate`] does not
/// compile here until it is classified as a shape input or not. The record ids and ordinals of
/// the A04 hole records are identity, not geometry: the body is a function of the polygons
/// alone, so only they enter the key.
fn plate_key(plate: &AuthoredPlate) -> Vec<u8> {
    let AuthoredPlate {
        id: _,
        name: _,
        class: _,
        contour_points_mm,
        thickness_mm,
        thickness_side,
        corners,
        placement: _,
        material: _,
        holes_mm,
        hole_records,
        cuts,
    } = plate;
    let mut key = Vec::with_capacity(160);
    key.extend_from_slice(b"BeamCraft/BodyCache/Plate/v4\0");
    push_points(&mut key, contour_points_mm);
    push_f64(&mut key, *thickness_mm);
    // The side changes the body: the mesh is laid off from the contour plane (A12.1).
    key.push(match thickness_side {
        None => 0,
        Some(PlateSide::Top) => 1,
        Some(PlateSide::Middle) => 2,
        Some(PlateSide::Bottom) => 3,
    });
    // The treatments change the evaluated outline the body is extruded from (A12.2).
    push_len(&mut key, corners.len());
    for corner in corners {
        push_len(&mut key, corner.vertex);
        key.push(match corner.treatment {
            CornerTreatment::Chamfer { .. } => 1,
            CornerTreatment::Round { .. } => 2,
        });
        match corner.treatment {
            CornerTreatment::Chamfer { d1_mm, d2_mm } => {
                push_f64(&mut key, d1_mm);
                push_f64(&mut key, d2_mm);
            }
            CornerTreatment::Round { radius_mm } => push_f64(&mut key, radius_mm),
        }
    }
    push_len(&mut key, holes_mm.len());
    for hole in holes_mm {
        push_points(&mut key, hole);
    }
    push_len(&mut key, hole_records.len());
    for record in hole_records {
        push_points(&mut key, &record.polygon_mm);
    }
    push_cut(&mut key, cuts);
    key
}

/// The key of a member body with its resolved profile. Every field of [`AuthoredMember`] and of
/// the profile is named, as for [`plate_key`]; the pinned profile digest is not an input, its
/// resolved values are. Placement is excluded: both translation and rotation belong to the
/// instance and never change the element-local body.
fn member_key(member: &AuthoredMember, profile: &ProfileInfo) -> Vec<u8> {
    let AuthoredMember {
        id: _,
        kind: _,
        name: _,
        class: _,
        axis_points_mm,
        profile: _,
        roll_rad,
        section_placement,
        placement: _,
        material: _,
        cuts,
    } = member;
    let mut key = Vec::with_capacity(224);
    key.extend_from_slice(b"BeamCraft/BodyCache/Member/v4\0");
    push_points(&mut key, axis_points_mm);
    push_option(&mut key, *roll_rad);
    match section_placement {
        None => key.push(0),
        Some(p) => {
            key.push(1);
            key.extend_from_slice(p.plane.as_str().as_bytes());
            key.extend_from_slice(p.depth.as_str().as_bytes());
            for v in [
                p.plane_offset_mm,
                p.depth_offset_mm,
                p.start_offset_mm,
                p.end_offset_mm,
            ] {
                push_f64(&mut key, v);
            }
        }
    }
    push_cut(&mut key, cuts);
    match profile {
        ProfileInfo::Rectangle {
            width_mm,
            height_mm,
            declared_area_mm2,
        } => {
            key.push(1);
            push_f64(&mut key, *width_mm);
            push_f64(&mut key, *height_mm);
            push_option(&mut key, *declared_area_mm2);
        }
        ProfileInfo::Section {
            shape,
            declared_area_mm2,
        } => {
            key.push(2);
            match shape {
                SectionShape::Parametric(section) => {
                    key.push(1);
                    let family = section.family().token();
                    push_len(&mut key, family.len());
                    key.extend_from_slice(family.as_bytes());
                    push_len(&mut key, section.values().len());
                    for value in section.values() {
                        push_f64(&mut key, *value);
                    }
                }
                SectionShape::Custom(CustomSection { outer, cavities }) => {
                    key.push(2);
                    push_points(&mut key, outer);
                    push_len(&mut key, cavities.len());
                    for cavity in cavities {
                        push_points(&mut key, cavity);
                    }
                }
            }
            push_option(&mut key, *declared_area_mm2);
        }
    }
    key
}

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
/// a plate is never silently dropped. Plates with bit-identical shape inputs share one kernel
/// evaluation within the call.
pub fn evaluate_committed(
    plates: &[AuthoredPlate],
    materials: MaterialLookup<'_>,
) -> Result<Vec<PlateRecord>, CommandError> {
    evaluate_committed_cached(plates, materials, &mut BodyCache::new())
}

/// [`evaluate_committed`] with a caller-owned [`BodyCache`], so a body evaluated by an earlier
/// call (the head before the commit verifier, for example) is not evaluated again. The checks,
/// their order and every refusal are the same as without a cache.
pub fn evaluate_committed_cached(
    plates: &[AuthoredPlate],
    materials: MaterialLookup<'_>,
    cache: &mut BodyCache,
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
        let body = cache.plate(authored)?;
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
/// and the placement are re-checked because the layer is untrusted input. Members with
/// bit-identical shape inputs and profile content share one kernel evaluation within the call.
pub fn evaluate_committed_members(
    members: &[AuthoredMember],
    profiles: ProfileLookup<'_>,
    materials: MaterialLookup<'_>,
) -> Result<Vec<MemberRecord>, CommandError> {
    evaluate_committed_members_cached(members, profiles, materials, &mut BodyCache::new())
}

/// [`evaluate_committed_members`] with a caller-owned [`BodyCache`]; the checks, their order and
/// every refusal are the same as without a cache.
pub fn evaluate_committed_members_cached(
    members: &[AuthoredMember],
    profiles: ProfileLookup<'_>,
    materials: MaterialLookup<'_>,
    cache: &mut BodyCache,
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
        let body = cache.member(authored, profile)?;
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

/// The snapshot-builder input for committed plates, including their complete rigid pose.
pub fn scene_elements(records: &[PlateRecord]) -> Result<Vec<SceneElement>, CommandError> {
    scene_elements_with_members(records, &[])
}

/// The snapshot-builder input for committed plates and members, in element-id order.
///
/// Plates keep their meshes local and carry their authored quaternion. A member's rotation
/// is carried by its scene instance, just as for a plate.
pub fn scene_elements_with_members(
    plates: &[PlateRecord],
    members: &[MemberRecord],
) -> Result<Vec<SceneElement>, CommandError> {
    let translation = |placement: &beamcraft_model::command::Placement| {
        placement.validate()?;
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
        elements.push(
            SceneElement::new(
                r.authored.id,
                ElementKind::Plate,
                translation(&r.authored.placement)?,
                r.body.solid.scene_mesh(),
            )
            .with_orientation(r.authored.placement.orientation_xyzw),
        );
    }
    for r in members {
        let kind = match r.authored.kind {
            MemberKind::Beam => ElementKind::Beam,
            MemberKind::Column => ElementKind::Column,
        };
        // Every mesh stays local; the instance carries the complete rigid placement.
        let [x, y, z] = r.authored.placement.translation_mm;
        let translation = Translation3::new(x, y, z).map_err(|_| {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "placement.translation_mm",
                "value is not finite",
            )
        })?;
        elements.push(
            SceneElement::new(r.authored.id, kind, translation, r.body.solid.scene_mesh())
                .with_orientation(r.authored.placement.orientation_xyzw),
        );
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
    use beamcraft_model::id::OpId;

    // Shadows the glob import: these tests have no material definitions unless they say so.
    fn evaluate_committed(plates: &[AuthoredPlate]) -> Result<Vec<PlateRecord>, CommandError> {
        super::evaluate_committed(plates, &no_materials)
    }

    fn plate(n: u8, width: f64, x: f64) -> AuthoredPlate {
        AuthoredPlate {
            id: ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap(),
            name: format!("Plate {n}"),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [width, 0.0, 0.0],
                [width, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement {
                translation_mm: [x, 0.0, 0.0],
                ..Placement::IDENTITY
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
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
            class: 0,
            axis_points_mm: vec![[0.0; 3], [0.0, 0.0, 3000.0]],
            profile,
            roll_rad: None,
            section_placement: None,
            placement: Placement::IDENTITY,
            material: None,
            cuts: Vec::new(),
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
    fn committed_rotated_member_has_f06_pos_12_world_bounds_and_keeps_shape_quantities() {
        use crate::member::ProfileInfo;
        use crate::{Limits, ViewPin, ViewSnapshot};
        let profile = Sha256::digest(b"profile").unwrap();
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
        let half = core::f64::consts::FRAC_1_SQRT_2;
        let mut member = AuthoredMember {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000001102").unwrap(),
            kind: MemberKind::Beam,
            name: "F06 POS-12".into(),
            class: 0,
            axis_points_mm: vec![[0.0; 3], [4000.0, 0.0, 0.0]],
            profile,
            roll_rad: None,
            section_placement: None,
            placement: Placement {
                translation_mm: [-50.0, 70.0, 1000.0],
                orientation_xyzw: [0.0, 0.0, half, half],
            },
            material: None,
            cuts: Vec::new(),
        };
        let records =
            super::evaluate_committed_members(&[member.clone()], &profiles, &no_materials).unwrap();
        assert_eq!(records[0].quantities.volume_mm3, 240_000_000.0);
        let identity = {
            member.placement = Placement::IDENTITY;
            super::evaluate_committed_members(&[member], &profiles, &no_materials).unwrap()
        };
        // Placement preserves the local body and quantities; world bounds apply the instance pose.
        assert_eq!(records[0].quantities, identity[0].quantities);
        assert_eq!(records[0].body.solid, identity[0].body.solid);
        let elements = scene_elements_with_members(&[], &records).unwrap();
        let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
        let snapshot = ViewSnapshot::build(pin, elements, &Limits::default()).unwrap();
        let bounds = snapshot.element_bounds(&records[0].authored.id).unwrap();
        for (actual, expected) in bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .zip([-150.0, 70.0, 850.0, 50.0, 4070.0, 1150.0])
        {
            assert!((actual - expected).abs() <= 0.001, "{actual} != {expected}");
        }
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

    fn l_shape(n: u8) -> AuthoredPlate {
        let mut plate = plate(n, 1000.0, 0.0);
        plate.contour_points_mm = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 300.0, 0.0],
            [400.0, 300.0, 0.0],
            [400.0, 600.0, 0.0],
            [0.0, 600.0, 0.0],
        ];
        plate
    }

    fn square_hole() -> Vec<[f64; 2]> {
        // Clockwise, as the CLI generators author holes.
        vec![
            [100.0, 100.0],
            [100.0, 200.0],
            [200.0, 200.0],
            [200.0, 100.0],
        ]
    }

    fn end_cut() -> Vec<CutRecord> {
        vec![CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000c7").unwrap(),
            ordinal: 0,
            point_mm: [250.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        }]
    }

    #[test]
    fn a_kept_plate_body_is_the_body_a_fresh_evaluation_gives() {
        let mut holed = plate(1, 1000.0, 0.0);
        holed.holes_mm = vec![square_hole()];
        let mut cut = plate(2, 1000.0, 0.0);
        cut.cuts = end_cut();
        let mut holed_contour = l_shape(3);
        holed_contour.holes_mm = vec![square_hole()];
        let plates = [plate(4, 1000.0, 0.0), holed, cut, l_shape(5), holed_contour];
        let fresh =
            super::evaluate_committed_cached(&plates, &no_materials, &mut BodyCache::disabled())
                .unwrap();
        let mut cache = BodyCache::new();
        let cold = super::evaluate_committed_cached(&plates, &no_materials, &mut cache).unwrap();
        let warm = super::evaluate_committed_cached(&plates, &no_materials, &mut cache).unwrap();
        assert_eq!(cold, fresh);
        assert_eq!(warm, fresh);
        assert_eq!((cache.misses(), cache.hits()), (5, 5));
        // The public entry point shares the bodies of identical plates within one call.
        assert_eq!(evaluate_committed(&plates).unwrap(), fresh);
    }

    #[test]
    fn only_shape_inputs_select_a_kept_plate_body() {
        let base = plate(1, 1000.0, 0.0);
        let mut copy = plate(2, 1000.0, 3000.0);
        copy.name = "Copy".into();
        copy.material = Some(Sha256::digest(b"steel").unwrap());
        // The kernel reads none of the fields a copy changes, so sharing its body is exact.
        assert_eq!(
            evaluate_plate(&base).unwrap(),
            evaluate_plate(&copy).unwrap()
        );
        let mut cache = BodyCache::new();
        cache.plate(&base).unwrap();
        assert_eq!(cache.plate(&copy).unwrap(), evaluate_plate(&copy).unwrap());
        assert_eq!((cache.misses(), cache.hits()), (1, 1));
        let mut thicker = base.clone();
        thicker.thickness_mm = 12.0;
        let mut wider = base.clone();
        wider.contour_points_mm[1][0] = 1001.0;
        wider.contour_points_mm[2][0] = 1001.0;
        let mut holed = base.clone();
        holed.holes_mm = vec![square_hole()];
        let mut moved_hole = holed.clone();
        moved_hole.holes_mm[0][0][0] = 101.0;
        moved_hole.holes_mm[0][1][0] = 101.0;
        let mut cut = base.clone();
        cut.cuts = end_cut();
        let mut tilted = cut.clone();
        tilted.cuts[0].normal = [1.0, 0.1, 0.0];
        for changed in [thicker, wider, holed, moved_hole, cut, tilted] {
            let misses = cache.misses();
            assert_eq!(
                cache.plate(&changed).unwrap(),
                evaluate_plate(&changed).unwrap()
            );
            assert_eq!(cache.misses(), misses + 1, "{changed:?}");
        }
    }

    #[test]
    fn signed_zeros_are_different_keys() {
        let triangle = |x: f64| {
            let mut plate = plate(1, 1000.0, 0.0);
            plate.contour_points_mm = vec![[x, 0.0, 0.0], [600.0, 0.0, 0.0], [0.0, 400.0, 0.0]];
            plate
        };
        let (positive, negative) = (triangle(0.0), triangle(-0.0));
        assert_ne!(plate_key(&positive), plate_key(&negative));
        let mut cache = BodyCache::new();
        // Whatever the kernel makes of each, the cache returns exactly that.
        assert_eq!(cache.plate(&positive), evaluate_plate(&positive));
        assert_eq!(cache.plate(&negative), evaluate_plate(&negative));
        assert_eq!(cache.plate(&negative), evaluate_plate(&negative));
        assert_eq!(cache.misses(), 2);
    }

    #[test]
    fn a_refused_body_is_not_kept() {
        let mut broken = plate(1, 1000.0, 0.0);
        broken.thickness_mm = -1.0;
        let mut cache = BodyCache::new();
        let refusal = evaluate_plate(&broken).unwrap_err();
        assert_eq!(cache.plate(&broken).unwrap_err(), refusal);
        assert_eq!(cache.plate(&broken).unwrap_err(), refusal);
        assert_eq!((cache.misses(), cache.hits()), (2, 0));
        assert_eq!(
            super::evaluate_committed_cached(&[broken], &no_materials, &mut cache)
                .unwrap_err()
                .code,
            refusal.code
        );
    }

    #[test]
    fn the_budget_bounds_the_cache_and_a_disabled_cache_keeps_nothing() {
        let (first, second) = (plate(1, 1000.0, 0.0), plate(2, 800.0, 0.0));
        let size = plate_key(&first).len() + solid_bytes(&evaluate_plate(&first).unwrap().solid);
        // Room for one body: keeping the second clears the first.
        let mut cache = BodyCache::with_budget(size);
        cache.plate(&first).unwrap();
        cache.plate(&first).unwrap();
        cache.plate(&second).unwrap();
        cache.plate(&first).unwrap();
        assert_eq!((cache.misses(), cache.hits()), (3, 1));
        // A body larger than the whole budget is returned and not kept.
        let mut tiny = BodyCache::with_budget(size - 1);
        assert_eq!(tiny.plate(&first).unwrap(), evaluate_plate(&first).unwrap());
        tiny.plate(&first).unwrap();
        assert_eq!((tiny.misses(), tiny.hits()), (2, 0));
        let mut off = BodyCache::disabled();
        assert_eq!(off.plate(&first).unwrap(), evaluate_plate(&first).unwrap());
        off.plate(&first).unwrap();
        assert_eq!((off.misses(), off.hits()), (0, 0));
        // A budget above the default is lowered to it.
        assert_eq!(BodyCache::with_budget(usize::MAX).budget, BODY_CACHE_BYTES);
    }

    #[test]
    fn members_share_a_body_by_shape_inputs_and_profile_content() {
        use beamcraft_model::section::{ParametricSection, SectionFamily};
        let rectangle = |width_mm: f64| ProfileInfo::Rectangle {
            width_mm,
            height_mm: 300.0,
            declared_area_mm2: None,
        };
        let i_section = ProfileInfo::Section {
            shape: SectionShape::Parametric(
                ParametricSection::new(SectionFamily::IShape, vec![300.0, 150.0, 7.0, 10.0, 15.0])
                    .unwrap(),
            ),
            declared_area_mm2: None,
        };
        let member = |n: u8, kind: MemberKind| AuthoredMember {
            id: ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap(),
            kind,
            name: format!("Member {n}"),
            class: 0,
            axis_points_mm: vec![[0.0; 3], [3000.0, 0.0, 0.0]],
            profile: Sha256::digest(&[n]).unwrap(),
            roll_rad: None,
            section_placement: None,
            placement: Placement::IDENTITY,
            material: None,
            cuts: Vec::new(),
        };
        let beam = member(1, MemberKind::Beam);
        let mut other = member(2, MemberKind::Column);
        other.placement.translation_mm = [0.0, 5000.0, 0.0];
        other.material = Some(Sha256::digest(b"steel").unwrap());
        // Kind, placement, material and the pinned digest never reach the kernel.
        assert_eq!(
            evaluate_member(&beam, i_section.clone()).unwrap(),
            evaluate_member(&other, i_section.clone()).unwrap()
        );
        let mut cache = BodyCache::new();
        cache.member(&beam, i_section.clone()).unwrap();
        assert_eq!(
            cache.member(&other, i_section.clone()).unwrap(),
            evaluate_member(&other, i_section.clone()).unwrap()
        );
        assert_eq!((cache.misses(), cache.hits()), (1, 1));
        // Profile content, axis, roll and cut are part of the key.
        let mut rolled = beam.clone();
        rolled.roll_rad = Some(0.0);
        let mut longer = beam.clone();
        longer.axis_points_mm[1][0] = 3001.0;
        let mut cut = beam.clone();
        cut.cuts = end_cut();
        let declared = ProfileInfo::Rectangle {
            width_mm: 200.0,
            height_mm: 300.0,
            declared_area_mm2: Some(60_000.0),
        };
        for (changed, profile) in [
            (beam.clone(), rectangle(200.0)),
            (beam.clone(), rectangle(201.0)),
            (beam.clone(), declared),
            (rolled, rectangle(200.0)),
            (longer, rectangle(200.0)),
            (cut, rectangle(200.0)),
        ] {
            let misses = cache.misses();
            assert_eq!(
                cache.member(&changed, profile.clone()).unwrap(),
                evaluate_member(&changed, profile).unwrap()
            );
            assert_eq!(cache.misses(), misses + 1, "{changed:?}");
        }
        let sections =
            |_: &ContentDigest| -> Result<ProfileInfo, CommandError> { Ok(i_section.clone()) };
        let steel = Sha256::digest(b"steel").unwrap();
        let materials = |d: &ContentDigest| {
            if *d == steel {
                Ok(MaterialInfo {
                    density_kg_m3: Some(7850.0),
                })
            } else {
                no_materials(d)
            }
        };
        let members = [beam, other];
        let records =
            super::evaluate_committed_members_cached(&members, &sections, &materials, &mut cache)
                .unwrap();
        assert_eq!(cache.hits(), 3);
        // The shared body still carries each member's own mass.
        assert_eq!(records[0].quantities.mass_kg, None);
        assert!(records[1].quantities.mass_kg.is_some());
        assert_eq!(
            records,
            super::evaluate_committed_members(&members, &sections, &materials).unwrap()
        );
    }

    #[test]
    fn duplicate_ids_and_bad_labels_are_refused_and_plate_rotation_is_preserved() {
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
        let scene = scene_elements(&records).unwrap();
        assert_eq!(scene[0].orientation_xyzw, [0.0, 0.0, s, s]);
        let mut broken = plate(5, 1000.0, 0.0);
        broken.thickness_mm = -1.0;
        assert!(evaluate_committed(&[broken]).is_err());
    }
}
