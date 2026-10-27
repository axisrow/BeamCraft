// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Quantity records with input signatures, and their freshness (F04 section 5, P04).
//!
//! A record has a status, a value (only when `current`), the signature of the inputs it was
//! computed from and the algorithm label. Shape and mass have separate signatures, so a change
//! of material invalidates the mass and keeps the shape, and a rename invalidates nothing.
//!
//! The engine never trusts a stored record: it recomputes the expected signatures from the
//! authored inputs and compares. A record whose signature differs is `stale`; one that is absent
//! is `missing`. A `stale` value is never reported as `current`.
//!
//! These signatures are P04's own, taken over the typed authored inputs, the algorithm label and
//! the tolerance policy version. They are not the F06 section 7 shape key (`kernel::keys` is
//! planned): no claim of that key is made.

use crate::command::{AuthoredPlate, Quantities, QuantityStatus, VOLUME_ALGORITHM};
use crate::member::{AuthoredMember, ProfileInfo};
use beamcraft_geom::canonical_f64_bytes;
use beamcraft_geom::tolerance::TOLERANCE_POLICY_VERSION;
use beamcraft_model::command::{CutRecord, KeepSide, PlateSide};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::section::SectionShape;

/// A quantity record as the engine computes it and as the derived layer stores it.
///
/// The derived layer stores the NET volume and mass with the net signatures (`volume_mm3`,
/// `mass_kg`, `shape_signature`, `mass_signature`). The gross figures and the removed volume are
/// computed from the authored inputs and reported next to them; they are not stored (the stored
/// attribute set is unchanged), so the gross signature is what a reader compares to know whether
/// a gross figure it holds is still the present one.
#[derive(Debug, Clone, PartialEq)]
pub struct QuantityRecord {
    /// NET volume in mm3.
    pub volume_mm3: f64,
    /// GROSS volume in mm3: the nominal body before any operation.
    pub volume_gross_mm3: f64,
    /// Volume removed by operations in mm3, measured on the meshes (`0.0` without operations).
    pub removed_volume_mm3: f64,
    /// Status token of the volume.
    pub volume_status: &'static str,
    /// NET mass in kg, present exactly when `mass_status` is `current`.
    pub mass_kg: Option<f64>,
    /// GROSS mass in kg, present exactly when `mass_status` is `current`.
    pub mass_gross_kg: Option<f64>,
    /// Status token of the mass.
    pub mass_status: &'static str,
    /// Signature of the NET shape inputs: the gross inputs and the list of operations.
    pub shape_signature: ContentDigest,
    /// Signature of the GROSS inputs: the nominal body (outline and thickness, or axis, roll,
    /// section, area source and area), without any operation.
    pub gross_signature: ContentDigest,
    /// Signature of the NET mass inputs: the net shape signature plus the density or its absence.
    pub mass_signature: ContentDigest,
    /// Signature of the GROSS mass inputs: the gross signature plus the density or its absence.
    pub mass_gross_signature: ContentDigest,
    /// Algorithm label of the volume.
    pub algorithm: String,
    /// Effective section area of a member in mm2; `None` for a plate.
    pub section_area_mm2: Option<f64>,
    /// `catalogue` or `computed` for a member; `None` for a plate.
    pub area_source: Option<&'static str>,
}

/// What a reader found stored next to a mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredRecord {
    /// Stored volume in mm3.
    pub volume_mm3: f64,
    /// Stored volume status token.
    pub volume_status: String,
    /// Stored mass in kg.
    pub mass_kg: Option<f64>,
    /// Stored mass status token.
    pub mass_status: String,
    /// Stored shape signature.
    pub shape_signature: ContentDigest,
    /// Stored mass signature.
    pub mass_signature: ContentDigest,
    /// Stored algorithm label.
    pub algorithm: String,
}

/// Freshness of one stored quantity against the inputs as they are now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// The stored signature is the expected one and the stored status is `current`.
    Current,
    /// A record exists but its signature differs: a previous value, not the present one.
    Stale,
    /// No record is stored.
    Missing,
}

impl Freshness {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Missing => "missing",
        }
    }
}

/// Freshness of the stored volume and mass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assessment {
    /// Volume freshness.
    pub volume: Freshness,
    /// Mass freshness.
    pub mass: Freshness,
}

/// Compares a stored record with the one the engine computes from the present inputs.
pub fn assess(stored: Option<&StoredRecord>, expected: &QuantityRecord) -> Assessment {
    let Some(stored) = stored else {
        return Assessment {
            volume: Freshness::Missing,
            mass: Freshness::Missing,
        };
    };
    let volume = if stored.shape_signature == expected.shape_signature
        && stored.volume_status == expected.volume_status
        && stored.volume_mm3 == expected.volume_mm3
        && stored.algorithm == expected.algorithm
    {
        Freshness::Current
    } else {
        Freshness::Stale
    };
    let mass = if stored.mass_signature == expected.mass_signature
        && stored.mass_status == expected.mass_status
        && stored.mass_kg == expected.mass_kg
    {
        Freshness::Current
    } else {
        Freshness::Stale
    };
    Assessment { volume, mass }
}

fn digest(bytes: &[u8]) -> ContentDigest {
    // The encodings are bounded by the command caps, far below the digest's input limit; an
    // overflow would only make two different inputs share the digest of the empty message, which
    // the status of a stale record then exposes, so the fallback needs no panic.
    Sha256::digest(bytes).unwrap_or_else(|_| ContentDigest::from_bytes([0; 32]))
}

fn float(bytes: &mut Vec<u8>, value: f64) {
    bytes.extend_from_slice(&canonical_f64_bytes(value).unwrap_or([0xff; 8]));
}

/// Signature of the shape inputs of a plate: outline, thickness, thickness side (A12.1: the
/// side changes the body, so the mesh and this signature change with it), holes and cut
/// records, with the algorithm and tolerance policy. Name, placement and material are not
/// shape inputs.
pub fn plate_shape_signature(plate: &AuthoredPlate, algorithm: &str) -> ContentDigest {
    let mut bytes = Vec::with_capacity(256);
    bytes.extend_from_slice(b"BeamCraft/P04/PlateShape/v2\0");
    bytes.extend_from_slice(&TOLERANCE_POLICY_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(algorithm.len() as u64).to_le_bytes());
    bytes.extend_from_slice(algorithm.as_bytes());
    bytes.extend_from_slice(&(plate.contour_points_mm.len() as u64).to_le_bytes());
    for v in plate.contour_points_mm.iter().flatten() {
        float(&mut bytes, *v);
    }
    float(&mut bytes, plate.thickness_mm);
    bytes.push(match plate.thickness_side {
        None => 0,
        Some(PlateSide::Top) => 1,
        Some(PlateSide::Middle) => 2,
        Some(PlateSide::Bottom) => 3,
    });
    bytes.extend_from_slice(&(plate.holes_mm.len() as u64).to_le_bytes());
    for hole in &plate.holes_mm {
        bytes.extend_from_slice(&(hole.len() as u64).to_le_bytes());
        for v in hole.iter().flatten() {
            float(&mut bytes, *v);
        }
    }
    push_cuts(&mut bytes, &plate.cuts);
    digest(&bytes)
}

/// The cut records of a signature: each record's identity-free content in ordinal order. The
/// operation id is not a shape input; the geometry and the kept side are.
fn push_cuts(bytes: &mut Vec<u8>, cuts: &[CutRecord]) {
    bytes.extend_from_slice(&(cuts.len() as u64).to_le_bytes());
    for cut in cuts {
        float(bytes, f64::from(cut.ordinal));
        bytes.push(match cut.keep {
            KeepSide::Positive => 1,
            KeepSide::Negative => 0,
        });
        cut.point_mm.into_iter().chain(cut.normal).for_each(|v| {
            float(bytes, v);
        });
    }
}

/// Signature of the GROSS inputs of a plate: the outline and thickness, with the gross algorithm
/// label and tolerance policy. Holes, name, placement and material are not gross inputs, so a
/// hole edit changes the net signature and leaves this one unchanged.
pub fn plate_gross_signature(plate: &AuthoredPlate) -> ContentDigest {
    let mut bytes = Vec::with_capacity(160);
    bytes.extend_from_slice(b"BeamCraft/PMass/PlateGross/v1\0");
    bytes.extend_from_slice(&TOLERANCE_POLICY_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(VOLUME_ALGORITHM.len() as u64).to_le_bytes());
    bytes.extend_from_slice(VOLUME_ALGORITHM.as_bytes());
    bytes.extend_from_slice(&(plate.contour_points_mm.len() as u64).to_le_bytes());
    for v in plate.contour_points_mm.iter().flatten() {
        float(&mut bytes, *v);
    }
    float(&mut bytes, plate.thickness_mm);
    digest(&bytes)
}

/// Signature of the GROSS inputs of a member: axis, roll, the resolved section values, the area
/// source with the declared and the effective area, the algorithm and the tolerance policy.
/// Changing the declared area (or removing it) changes this signature, so a stored quantity
/// computed from another area is `stale`. The profile digest is not an input; its values are.
pub fn member_gross_signature(
    member: &AuthoredMember,
    profile: &ProfileInfo,
    algorithm: &str,
) -> ContentDigest {
    let mut bytes = Vec::with_capacity(192);
    bytes.extend_from_slice(b"BeamCraft/PMass/MemberGross/v1\0");
    bytes.extend_from_slice(&TOLERANCE_POLICY_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(algorithm.len() as u64).to_le_bytes());
    bytes.extend_from_slice(algorithm.as_bytes());
    for v in member.axis_points_mm.iter().flatten() {
        float(&mut bytes, *v);
    }
    match member.roll_rad {
        Some(roll) => {
            bytes.push(1);
            float(&mut bytes, roll);
        }
        None => bytes.push(0),
    }
    push_cuts(&mut bytes, &member.cuts);
    match profile {
        ProfileInfo::Rectangle {
            width_mm,
            height_mm,
            ..
        } => {
            bytes.extend_from_slice(b"rectangle\0");
            float(&mut bytes, *width_mm);
            float(&mut bytes, *height_mm);
        }
        ProfileInfo::Section { shape, .. } => {
            bytes.extend_from_slice(b"section\0");
            bytes.extend_from_slice(shape.token().as_bytes());
            bytes.push(0);
            match shape {
                SectionShape::Parametric(p) => {
                    p.values().iter().for_each(|v| float(&mut bytes, *v))
                }
                SectionShape::Custom(c) => {
                    bytes.extend_from_slice(&(c.cavities.len() as u64).to_le_bytes());
                    for contour in core::iter::once(&c.outer).chain(c.cavities.iter()) {
                        bytes.extend_from_slice(&(contour.len() as u64).to_le_bytes());
                        contour.iter().flatten().for_each(|v| float(&mut bytes, *v));
                    }
                }
            }
        }
    }
    let (area, source) = profile.effective_area();
    bytes.extend_from_slice(source.as_str().as_bytes());
    bytes.push(0);
    match profile.declared_area_mm2() {
        Some(declared) => {
            bytes.push(1);
            float(&mut bytes, declared);
        }
        None => bytes.push(0),
    }
    float(&mut bytes, area);
    digest(&bytes)
}

/// Signature of the NET inputs of a member: the gross signature and the list of operations, each
/// identified by the digest of its own authored content. A member has no operation yet, so the
/// list is empty today; a cut, bolt hole or cope adds its digest here and changes the net
/// signature without touching the gross one.
pub fn member_net_signature(gross: &ContentDigest, operations: &[ContentDigest]) -> ContentDigest {
    let mut bytes = Vec::with_capacity(48 + 32 * operations.len());
    bytes.extend_from_slice(b"BeamCraft/PMass/MemberNet/v1\0");
    bytes.extend_from_slice(gross.as_bytes());
    bytes.extend_from_slice(&(operations.len() as u64).to_le_bytes());
    for operation in operations {
        bytes.extend_from_slice(operation.as_bytes());
    }
    digest(&bytes)
}

/// The signature stored as a member's shape signature (the NET one, without operations).
pub fn member_shape_signature(
    member: &AuthoredMember,
    profile: &ProfileInfo,
    algorithm: &str,
) -> ContentDigest {
    member_net_signature(&member_gross_signature(member, profile, algorithm), &[])
}

/// Signature of the mass inputs: the shape signature and the density, or its absence.
pub fn mass_signature(shape: &ContentDigest, density_kg_m3: Option<f64>) -> ContentDigest {
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(b"BeamCraft/P04/Mass/v1\0");
    bytes.extend_from_slice(shape.as_bytes());
    match density_kg_m3 {
        Some(density) => {
            bytes.push(1);
            float(&mut bytes, density);
        }
        None => bytes.push(0),
    }
    digest(&bytes)
}

/// The record of the engine's quantities under the given net and gross shape signatures.
pub fn record_for(
    shape: ContentDigest,
    gross: ContentDigest,
    quantities: &Quantities,
) -> QuantityRecord {
    let token = |s: QuantityStatus| s.as_str();
    QuantityRecord {
        volume_mm3: quantities.volume_mm3,
        volume_gross_mm3: quantities.volume_gross_mm3,
        removed_volume_mm3: quantities.removed_volume_mm3,
        volume_status: token(quantities.volume_status),
        mass_kg: quantities.mass_kg,
        mass_gross_kg: quantities.mass_gross_kg,
        mass_status: token(quantities.mass_status),
        shape_signature: shape,
        gross_signature: gross,
        mass_signature: mass_signature(&shape, quantities.density_kg_m3),
        mass_gross_signature: mass_signature(&gross, quantities.density_kg_m3),
        algorithm: quantities.algorithm.to_owned(),
        section_area_mm2: quantities.section_area_mm2,
        area_source: quantities.area_source.map(|s| s.as_str()),
    }
}

/// The record the engine expects for a plate: its quantities under the signatures of its shape
/// inputs (net: outline, thickness and holes) and of its nominal body (gross: outline and
/// thickness).
pub fn plate_record(
    plate: &AuthoredPlate,
    algorithm: &str,
    quantities: &Quantities,
) -> QuantityRecord {
    record_for(
        plate_shape_signature(plate, algorithm),
        plate_gross_signature(plate),
        quantities,
    )
}

/// The record the engine expects for a member.
pub fn member_record(
    member: &AuthoredMember,
    profile: &ProfileInfo,
    quantities: &Quantities,
) -> QuantityRecord {
    let algorithm = quantities.algorithm;
    let gross = member_gross_signature(member, profile, algorithm);
    record_for(member_net_signature(&gross, &[]), gross, quantities)
}

/// Whether a stored algorithm label is one that an earlier slice wrote for a member. Such a
/// record is a superseded value (`stale`), not damage.
pub fn is_superseded_algorithm(label: &str) -> bool {
    crate::member::SUPERSEDED_MEMBER_ALGORITHMS.contains(&label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::command::Placement;
    use beamcraft_model::id::ElementId;

    fn plate() -> AuthoredPlate {
        AuthoredPlate {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
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
            placement: Placement::IDENTITY,
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    #[test]
    fn only_shape_inputs_enter_the_shape_signature() {
        let base = plate_shape_signature(&plate(), "a");
        let mut renamed = plate();
        renamed.name = "Other".into();
        renamed.material = Some(Sha256::digest(b"m").unwrap());
        renamed.placement.translation_mm = [5.0, 0.0, 0.0];
        assert_eq!(plate_shape_signature(&renamed, "a"), base);
        let mut thicker = plate();
        thicker.thickness_mm = 12.0;
        assert_ne!(plate_shape_signature(&thicker, "a"), base);
        let mut holed = plate();
        holed.holes_mm = vec![vec![[1.0, 1.0], [1.0, 2.0], [2.0, 2.0]]];
        assert_ne!(plate_shape_signature(&holed, "a"), base);
        assert_ne!(plate_shape_signature(&plate(), "b"), base);
    }

    #[test]
    fn density_changes_the_mass_signature_and_never_the_shape_signature() {
        let shape = plate_shape_signature(&plate(), "a");
        let none = mass_signature(&shape, None);
        assert_ne!(mass_signature(&shape, Some(7850.0)), none);
        assert_ne!(
            mass_signature(&shape, Some(7850.0)),
            mass_signature(&shape, Some(2700.0))
        );
        assert_eq!(mass_signature(&shape, None), none);
    }

    fn record(volume: f64, density: Option<f64>, shape: ContentDigest) -> QuantityRecord {
        let mass = density.map(|d| volume * d / 1e9);
        QuantityRecord {
            volume_mm3: volume,
            volume_gross_mm3: volume,
            removed_volume_mm3: 0.0,
            volume_status: "current",
            mass_kg: mass,
            mass_gross_kg: mass,
            mass_status: if density.is_some() {
                "current"
            } else {
                "unknown"
            },
            shape_signature: shape,
            gross_signature: shape,
            mass_signature: mass_signature(&shape, density),
            mass_gross_signature: mass_signature(&shape, density),
            algorithm: "a".into(),
            section_area_mm2: None,
            area_source: None,
        }
    }

    fn stored(r: &QuantityRecord) -> StoredRecord {
        StoredRecord {
            volume_mm3: r.volume_mm3,
            volume_status: r.volume_status.into(),
            mass_kg: r.mass_kg,
            mass_status: r.mass_status.into(),
            shape_signature: r.shape_signature,
            mass_signature: r.mass_signature,
            algorithm: r.algorithm.clone(),
        }
    }

    fn member() -> AuthoredMember {
        use beamcraft_model::command::MemberKind;
        AuthoredMember {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000009").unwrap(),
            kind: MemberKind::Beam,
            name: "Beam".into(),
            class: 0,
            axis_points_mm: [[0.0; 3], [6000.0, 0.0, 0.0]],
            profile: Sha256::digest(b"profile").unwrap(),
            roll_rad: None,
            placement: Placement::IDENTITY,
            material: None,
            cuts: Vec::new(),
        }
    }

    fn i_profile(declared: Option<f64>) -> ProfileInfo {
        use beamcraft_model::section::{ParametricSection, SectionFamily};
        ProfileInfo::Section {
            shape: SectionShape::Parametric(
                ParametricSection::new(SectionFamily::IShape, vec![298.0, 149.0, 5.5, 8.0, 13.0])
                    .unwrap(),
            ),
            declared_area_mm2: declared,
        }
    }

    fn member_record_of(declared: Option<f64>, density: Option<f64>) -> QuantityRecord {
        let profile = i_profile(declared);
        let body = crate::member::evaluate_member(&member(), profile.clone()).unwrap();
        let quantities = crate::member::member_quantities(
            &body,
            density.map(|d| crate::command::MaterialInfo {
                density_kg_m3: Some(d),
            }),
        )
        .unwrap();
        member_record(&member(), &profile, &quantities)
    }

    #[test]
    fn the_declared_area_and_its_source_are_gross_and_net_inputs_of_a_member() {
        let computed = member_record_of(None, Some(7850.0));
        let declared = member_record_of(Some(4080.0), Some(7850.0));
        let other = member_record_of(Some(4090.0), Some(7850.0));
        assert_eq!(computed.area_source, Some("computed"));
        assert_eq!(declared.area_source, Some("catalogue"));
        assert_ne!(computed.gross_signature, declared.gross_signature);
        assert_ne!(declared.gross_signature, other.gross_signature);
        assert_ne!(computed.shape_signature, declared.shape_signature);
        // A stored record of the declared area is stale against another declared area, and
        // against the computed one, but current against itself.
        let stored = stored(&declared);
        assert_eq!(assess(Some(&stored), &declared).volume, Freshness::Current);
        assert_eq!(assess(Some(&stored), &other).volume, Freshness::Stale);
        assert_eq!(assess(Some(&stored), &other).mass, Freshness::Stale);
        assert_eq!(assess(Some(&stored), &computed).volume, Freshness::Stale);
        // No operation: gross equals net, in value and in the mass figures.
        assert_eq!(declared.volume_gross_mm3, declared.volume_mm3);
        assert_eq!(declared.mass_gross_kg, declared.mass_kg);
        assert_eq!(declared.removed_volume_mm3, 0.0);
        // The density changes the mass signatures and neither shape signature.
        let unknown = member_record_of(Some(4080.0), None);
        assert_eq!(unknown.gross_signature, declared.gross_signature);
        assert_ne!(unknown.mass_signature, declared.mass_signature);
        assert_ne!(unknown.mass_gross_signature, declared.mass_gross_signature);
        assert_eq!(unknown.mass_kg, None);
    }

    #[test]
    fn a_future_operation_changes_the_net_signature_and_never_the_gross_one() {
        let record = member_record_of(None, None);
        let cut = Sha256::digest(b"cut").unwrap();
        let net = member_net_signature(&record.gross_signature, &[cut]);
        assert_ne!(net, record.shape_signature);
        assert_ne!(
            member_net_signature(&record.gross_signature, &[cut, cut]),
            net
        );
        assert_eq!(
            member_net_signature(&record.gross_signature, &[]),
            record.shape_signature
        );
    }

    #[test]
    fn a_hole_edit_changes_the_plate_net_and_not_the_gross_signature_or_value() {
        use crate::command::evaluate_plate;
        let record = |holes: Vec<Vec<[f64; 2]>>| {
            let mut p = plate();
            p.holes_mm = holes;
            let body = evaluate_plate(&p).unwrap();
            let q = body.quantities();
            plate_record(&p, body.algorithm(), &q)
        };
        let solid = record(vec![]);
        let one = record(vec![vec![
            [100.0, 100.0],
            [100.0, 200.0],
            [200.0, 200.0],
            [200.0, 100.0],
        ]]);
        let two = record(vec![
            vec![
                [100.0, 100.0],
                [100.0, 200.0],
                [200.0, 200.0],
                [200.0, 100.0],
            ],
            vec![
                [400.0, 100.0],
                [400.0, 150.0],
                [450.0, 150.0],
                [450.0, 100.0],
            ],
        ]);
        assert_eq!(solid.gross_signature, one.gross_signature);
        assert_eq!(one.gross_signature, two.gross_signature);
        assert_eq!(one.volume_gross_mm3, 5_000_000.0);
        assert_eq!(solid.volume_gross_mm3, one.volume_gross_mm3);
        assert_eq!(two.volume_gross_mm3, one.volume_gross_mm3);
        assert_ne!(solid.shape_signature, one.shape_signature);
        assert_ne!(one.shape_signature, two.shape_signature);
        assert!(one.volume_mm3 < solid.volume_mm3 && two.volume_mm3 < one.volume_mm3);
        assert!((one.removed_volume_mm3 - 100_000.0).abs() <= 1e-6);
        assert!((two.removed_volume_mm3 - 125_000.0).abs() <= 1e-6);
        // A thickness change is a gross input.
        let mut thicker = plate();
        thicker.thickness_mm = 12.0;
        assert_ne!(plate_gross_signature(&thicker), solid.gross_signature);
    }

    #[test]
    fn a_record_with_a_superseded_label_reads_stale_not_corrupt() {
        let now = member_record_of(None, Some(7850.0));
        let mut old = stored(&now);
        old.algorithm =
            "kernel-a07-section-member-volume/1;detailed-arcs-max(0.25mm,0.5%r);tolerance-policy-3"
                .into();
        old.volume_mm3 = 24_555_000.0;
        // An old record was signed with the old signature bytes, which no longer match.
        old.shape_signature = Sha256::digest(b"old shape").unwrap();
        old.mass_signature = Sha256::digest(b"old mass").unwrap();
        let assessment = assess(Some(&old), &now);
        assert_eq!(assessment.volume, Freshness::Stale);
        assert_eq!(assessment.mass, Freshness::Stale);
        assert!(is_superseded_algorithm(&old.algorithm));
        assert!(is_superseded_algorithm(
            "kernel-p03-straight-member-volume/1;tolerance-policy-3"
        ));
        assert!(!is_superseded_algorithm(&now.algorithm));
        assert!(!is_superseded_algorithm("anything else"));
    }

    #[test]
    fn a_stored_record_is_current_stale_or_missing_never_assumed() {
        let shape = plate_shape_signature(&plate(), "a");
        let now = record(5e6, Some(7850.0), shape);
        let old = stored(&now);
        assert_eq!(
            assess(Some(&old), &now),
            Assessment {
                volume: Freshness::Current,
                mass: Freshness::Current
            }
        );
        // Material changed: the volume is still current, the stored mass is stale.
        let relabelled = record(5e6, Some(2700.0), shape);
        assert_eq!(
            assess(Some(&old), &relabelled),
            Assessment {
                volume: Freshness::Current,
                mass: Freshness::Stale
            }
        );
        // Shape changed: both are stale.
        let mut thicker = plate();
        thicker.thickness_mm = 12.0;
        let new_shape = plate_shape_signature(&thicker, "a");
        assert_eq!(
            assess(Some(&old), &record(6e6, Some(7850.0), new_shape)),
            Assessment {
                volume: Freshness::Stale,
                mass: Freshness::Stale
            }
        );
        // A tampered value under a matching signature is not current either.
        let mut forged = old.clone();
        forged.volume_mm3 = 1.0;
        assert_eq!(assess(Some(&forged), &now).volume, Freshness::Stale);
        assert_eq!(
            assess(None, &now),
            Assessment {
                volume: Freshness::Missing,
                mass: Freshness::Missing
            }
        );
    }
}
