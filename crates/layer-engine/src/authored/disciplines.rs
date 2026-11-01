// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The read half of the discipline seam (#795): what the platform asks a discipline when it
//! validates and indexes a loaded layer.
//!
//! The write half is the hook carried by `Shape::Discipline` and `Change::Discipline`; a loaded
//! layer has no element object to carry behaviour, so the read half is a registry. The registry
//! holds the built-in steel discipline and is installable: a discipline crate registers one
//! `&'static dyn Discipline` through [`register`], deduplicated by pointer, and every platform
//! question below is answered by the registered disciplines in registration order.

use super::*;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::ptr::eq as same_pointer;
use std::sync::Mutex;
use std::sync::PoisonError;

/// A discipline as the platform reads it back from a stored layer.
pub trait Discipline: Debug + Send + Sync {
    /// The `bim:elementKind` tokens of the parts of this discipline (the `Part` category).
    fn part_kinds(&self) -> &'static [&'static str];
    /// Validates one owned part prim of a kind in [`Discipline::part_kinds`]: its operation
    /// history, its shape inputs and its owned records. `operation_ids` collects every operation
    /// identity of the layer so a repeat is refused.
    fn validate_part(
        &self,
        kind: &str,
        prim: &Prim,
        prims: &[Prim],
        operation_ids: &mut BTreeSet<beamcraft_model::id::OpId>,
    ) -> Result<(), Diagnostic>;
    /// Validates the discipline's properties of a `connection` prim.
    fn validate_connection(&self, prim: &Prim, prims: &[Prim]) -> Result<(), Diagnostic>;
    /// The semantic write group of a property name this discipline owns, if it owns it.
    fn property_group(&self, name: &str) -> Option<beamcraft_model::changeset::DependencyGroup>;
    /// Validates the `bim:connects` records of the whole composed stage: the cross-element rules
    /// a discipline owns beyond one prim's own checks. The platform asks every registered
    /// discipline once per snapshot.
    fn validate_connections(&self, _prims: &[Prim]) -> Result<(), Diagnostic> {
        Ok(())
    }
    /// Whether a property name is one of the shared shape inputs an instance of this discipline
    /// reads from its canonical part (P11): a local opinion shadowing it on an instance prim is
    /// refused.
    fn shared_property(&self, _name: &str) -> bool {
        false
    }
    /// The discipline's attribute names of one owned kind that a whole-record change must drop,
    /// so no stale value of a field the new record no longer authors survives the edit.
    fn delete_attributes(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }
    /// The logical definition stored in a definition prim of this discipline, or `None` when the
    /// `bim:definitionKind` is not one this discipline owns. A stored layer is untrusted input,
    /// so the definition is re-admitted, never trusted.
    fn read_definition(&self, _prim: &Prim, _kind: &str) -> Result<Option<Definition>, Diagnostic> {
        Ok(None)
    }
    /// The canonical `bim:elementKind` token this discipline stores for an owned part token, if
    /// it owns the token at all. The platform compares part kinds through it (P11: an instance
    /// and its canonical part answer the same token) instead of matching on discipline kinds.
    fn part_kind(&self, _kind: &str) -> Option<&'static str> {
        None
    }
    /// Whether a definition prim property name is one this discipline's definitions own beyond
    /// the platform's own definition fields, and so belongs to the definition's content.
    fn definition_property(&self, _name: &str) -> bool {
        false
    }
}

/// The built-in steel discipline: plates, beams, columns, welds and bolt groups.
#[derive(Debug)]
struct Steel;

impl Discipline for Steel {
    fn part_kinds(&self) -> &'static [&'static str] {
        &["plate", "beam", "column"]
    }

    fn validate_part(
        &self,
        kind: &str,
        prim: &Prim,
        prims: &[Prim],
        operation_ids: &mut BTreeSet<beamcraft_model::id::OpId>,
    ) -> Result<(), Diagnostic> {
        validate::steel_part(kind, prim, prims, operation_ids)
    }

    fn validate_connection(&self, prim: &Prim, prims: &[Prim]) -> Result<(), Diagnostic> {
        validate::steel_connection(prim, prims)
    }

    fn property_group(&self, name: &str) -> Option<beamcraft_model::changeset::DependencyGroup> {
        use beamcraft_model::changeset::DependencyGroup as G;
        Some(match name {
            "steel:plate:contourPointsMm"
            | "steel:plate:thicknessMm"
            | "steel:plate:thicknessSide"
            | "steel:plate:cornerVertex"
            | "steel:plate:cornerKind"
            | "steel:plate:cornerD1Mm"
            | "steel:plate:cornerD2Mm"
            | "steel:plate:cornerRadiusMm"
            | "steel:plate:bendOperationId"
            | "steel:plate:bendKind"
            | "steel:plate:bendLineAMm"
            | "steel:plate:bendLineBMm"
            | "steel:plate:bendRadiusMm"
            | "steel:plate:bendRadiusBMm"
            | "steel:plate:bendAngleRad"
            | "steel:plate:holePointsMm"
            | "steel:plate:holeVertexCounts"
            | "steel:cut:operationId"
            | "steel:cut:ordinal"
            | "steel:cut:pointMm"
            | "steel:cut:normal"
            | "steel:cut:keep"
            | "steel:cut:nextOrdinal"
            | "steel:cut:retiredOperationIds"
            | "steel:hole:nextOrdinal"
            | "steel:hole:retiredOperationIds"
            | "steel:hole:pointsMm"
            | "steel:hole:vertexCounts"
            | "steel:hole:operationIds"
            | "steel:hole:ordinals"
            | "steel:member:pathType"
            | "bim:axisPointsMm"
            | "steel:member:rollRad"
            | "steel:member:planePosition"
            | "steel:member:planeOffsetMm"
            | "steel:member:depthPosition"
            | "steel:member:depthOffsetMm"
            | "steel:member:startOffsetMm"
            | "steel:member:endOffsetMm"
            | "bim:profile"
            | "steel:connection:type"
            | "steel:weld:type"
            | "steel:weld:sizeMm"
            | "steel:weld:location"
            | "steel:weld:side"
            | "steel:bolt:diameterMm"
            | "steel:bolt:count"
            | "steel:bolt:pitchMm"
            | "steel:bolt:originMm"
            | "steel:bolt:directionMm"
            | "steel:bolt:fastener"
            | "steel:bolt:point1Mm"
            | "steel:bolt:point2Mm"
            | "steel:bolt:planeNormalMm"
            | "steel:bolt:rotationRad"
            | "steel:bolt:onPlaneMm"
            | "steel:bolt:depthMm"
            | "steel:bolt:offsetStartMm"
            | "steel:bolt:spacingMode"
            | "steel:bolt:spacingValueMm"
            | "steel:bolt:spacingsMm"
            | "steel:bolt:lengthMode"
            | "steel:bolt:lengthMm"
            | "steel:bolt:washerUnderHead"
            | "steel:bolt:washerUnderNut" => G::ShapeInputs,
            _ => return None,
        })
    }

    fn shared_property(&self, name: &str) -> bool {
        matches!(
            name,
            "steel:plate:contourPointsMm"
                | "steel:plate:thicknessMm"
                | "steel:plate:thicknessSide"
                | "steel:plate:cornerVertex"
                | "steel:plate:cornerKind"
                | "steel:plate:cornerD1Mm"
                | "steel:plate:cornerD2Mm"
                | "steel:plate:cornerRadiusMm"
                | "steel:plate:bendOperationId"
                | "steel:plate:bendKind"
                | "steel:plate:bendLineAMm"
                | "steel:plate:bendLineBMm"
                | "steel:plate:bendRadiusMm"
                | "steel:plate:bendRadiusBMm"
                | "steel:plate:bendAngleRad"
                | "steel:plate:holePointsMm"
                | "steel:plate:holeVertexCounts"
                | "steel:hole:pointsMm"
                | "steel:hole:vertexCounts"
                | "steel:hole:operationIds"
                | "steel:hole:ordinals"
                | "steel:hole:kinds"
                | "steel:hole:generatedBy"
                | "steel:member:pathType"
                | "steel:member:rollRad"
                | "steel:member:planePosition"
                | "steel:member:planeOffsetMm"
                | "steel:member:depthPosition"
                | "steel:member:depthOffsetMm"
                | "steel:member:startOffsetMm"
                | "steel:member:endOffsetMm"
                | "bim:axisPointsMm"
                | "steel:cut:operationId"
                | "steel:cut:ordinal"
                | "steel:cut:pointMm"
                | "steel:cut:normal"
                | "steel:cut:keep"
                | "steel:cut:nextOrdinal"
                | "steel:cut:retiredOperationIds"
                | "steel:hole:nextOrdinal"
                | "steel:hole:retiredOperationIds"
        )
    }

    fn part_kind(&self, kind: &str) -> Option<&'static str> {
        match kind {
            "plate" => Some("plate"),
            "beam" => Some("beam"),
            "column" => Some("column"),
            _ => None,
        }
    }

    fn definition_property(&self, name: &str) -> bool {
        matches!(
            name,
            "bim:contentDigest"
                | "bim:schemaVersion"
                | "bim:units"
                | "bim:definitionKind"
                | "bim:densityKgM3"
                | "bim:materialGrade"
                | "bim:materialStandard"
                | "steel:profile:widthMm"
                | "steel:profile:heightMm"
                | "steel:profile:designation"
                | "steel:profile:standard"
                | "steel:profile:areaMm2"
                | "steel:profile:massPerMeterKg"
                | "steel:profile:surfacePerMeterM2"
                | "steel:profile:hMm"
                | "steel:profile:bMm"
                | "steel:profile:twMm"
                | "steel:profile:tfMm"
                | "steel:profile:rMm"
                | "steel:profile:tMm"
                | "steel:profile:dMm"
                | "steel:profile:cMm"
                | "steel:profile:outerPointsMm"
                | "steel:profile:cavityPointsMm"
                | "steel:profile:cavityVertexCounts"
        )
    }

    fn delete_attributes(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("steel:plate:contourPointsMm", "double3[]"),
            ("steel:plate:thicknessMm", "double"),
            ("steel:plate:holePointsMm", "double3[]"),
            ("steel:plate:holeVertexCounts", "int[]"),
            ("steel:cut:operationId", "string[]"),
            ("steel:cut:ordinal", "int[]"),
            ("steel:cut:pointMm", "double3[]"),
            ("steel:cut:normal", "double3[]"),
            ("steel:cut:keep", "token[]"),
            (cut_history::NEXT, "int"),
            (cut_history::RETIRED, "string[]"),
            ("steel:hole:pointsMm", "double3[]"),
            ("steel:hole:vertexCounts", "int[]"),
            ("steel:hole:operationIds", "string[]"),
            ("steel:hole:ordinals", "int[]"),
            (cut_history::HOLE_NEXT, "int"),
            (cut_history::HOLE_RETIRED, "string[]"),
            ("bim:axisPointsMm", "double3[]"),
            ("steel:member:pathType", "token"),
            ("steel:member:rollRad", "double"),
            ("steel:member:planePosition", "token"),
            ("steel:member:planeOffsetMm", "double"),
            ("steel:member:depthPosition", "token"),
            ("steel:member:depthOffsetMm", "double"),
            ("steel:member:startOffsetMm", "double"),
            ("steel:member:endOffsetMm", "double"),
            ("bim:profile", "rel"),
            ("steel:connection:type", "token"),
            ("steel:weld:type", "token"),
            ("steel:weld:sizeMm", "double"),
            ("steel:weld:location", "token"),
            ("steel:weld:side", "token"),
            ("steel:bolt:diameterMm", "double"),
            ("steel:bolt:count", "int"),
            ("steel:bolt:pitchMm", "double"),
            ("steel:bolt:originMm", "double3"),
            ("steel:bolt:directionMm", "double3"),
            ("bim:connects", "rel"),
            ("steel:bolt:fastener", "string"),
            ("steel:bolt:point1Mm", "double3"),
            ("steel:bolt:point2Mm", "double3"),
            ("steel:bolt:planeNormalMm", "double3"),
            ("steel:bolt:rotationRad", "double"),
            ("steel:bolt:onPlaneMm", "double"),
            ("steel:bolt:depthMm", "double"),
            ("steel:bolt:offsetStartMm", "double"),
            ("steel:bolt:spacingMode", "token"),
            ("steel:bolt:spacingValueMm", "double"),
            ("steel:bolt:spacingsMm", "double[]"),
            ("steel:bolt:lengthMode", "token"),
            ("steel:bolt:lengthMm", "double"),
            ("steel:bolt:washerUnderHead", "bool"),
            ("steel:bolt:washerUnderNut", "bool"),
            ("steel:bolt:holeMode", "token"),
            ("steel:bolt:holeClearanceMm", "double"),
        ]
    }

    fn validate_connections(&self, prims: &[Prim]) -> Result<(), Diagnostic> {
        validate::connects(prims)
    }
}

static STEEL: Steel = Steel;

/// The installed disciplines in registration order, deduplicated by pointer. The built-in steel
/// discipline is installed first, so it stays the first registered discipline.
fn registry() -> &'static Mutex<Vec<&'static dyn Discipline>> {
    static REGISTERED: Mutex<Vec<&'static dyn Discipline>> = Mutex::new(Vec::new());
    &REGISTERED
}

/// Runs `body` with the registry, installing the built-in steel discipline first.
fn with_registry<T>(body: impl FnOnce(&mut Vec<&'static dyn Discipline>) -> T) -> T {
    let mut registered = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if registered.is_empty() {
        registered.push(&STEEL);
    }
    body(&mut registered)
}

/// Installs one discipline. A discipline already registered by pointer is kept, not repeated,
/// so a re-entry of a discipline crate cannot shift the registration order.
pub fn register(discipline: &'static dyn Discipline) {
    with_registry(|registered| {
        if !registered
            .iter()
            .any(|installed| same_pointer(*installed, discipline))
        {
            registered.push(discipline);
        }
    });
}

/// The disciplines the platform reads, in registration order.
pub fn registered() -> Vec<&'static dyn Discipline> {
    with_registry(|registered| registered.clone())
}

/// Whether `kind` is a part kind of a registered discipline.
pub fn is_part_kind(kind: &str) -> bool {
    registered()
        .iter()
        .any(|discipline| discipline.part_kinds().contains(&kind))
}

/// Validates a part prim through the discipline that owns its kind; a kind no discipline owns
/// stays inert.
pub(super) fn validate_part(
    kind: &str,
    prim: &Prim,
    prims: &[Prim],
    operation_ids: &mut BTreeSet<beamcraft_model::id::OpId>,
) -> Result<(), Diagnostic> {
    match registered()
        .iter()
        .find(|discipline| discipline.part_kinds().contains(&kind))
    {
        Some(discipline) => discipline.validate_part(kind, prim, prims, operation_ids),
        None => Ok(()),
    }
}

/// Validates the discipline's properties of a `connection` prim.
///
/// Every `connection` prim goes to the first registered discipline, as it went through the steel
/// checks before the registry. With a second discipline this needs an ownership test per prim.
pub(super) fn validate_connection(prim: &Prim, prims: &[Prim]) -> Result<(), Diagnostic> {
    match registered().first() {
        Some(discipline) => discipline.validate_connection(prim, prims),
        None => Err(invalid().with_rule("DV-ATTR-002")),
    }
}

/// Validates the `bim:connects` records of the whole composed stage: every registered
/// discipline's cross-element rules, in registration order.
pub(super) fn validate_connections(prims: &[Prim]) -> Result<(), Diagnostic> {
    for discipline in registered() {
        discipline.validate_connections(prims)?;
    }
    Ok(())
}

/// The write group of a property name owned by a registered discipline.
pub(super) fn property_group(name: &str) -> Option<beamcraft_model::changeset::DependencyGroup> {
    registered()
        .iter()
        .find_map(|discipline| discipline.property_group(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A probe discipline of one kind no other test names, so installing it cannot shift any
    /// other answer of the registry.
    #[derive(Debug)]
    struct Probe;

    static PROBE: Probe = Probe;

    impl Discipline for Probe {
        fn part_kinds(&self) -> &'static [&'static str] {
            &["disciplines-probe-part"]
        }

        fn validate_part(
            &self,
            _kind: &str,
            _prim: &Prim,
            _prims: &[Prim],
            _operation_ids: &mut BTreeSet<beamcraft_model::id::OpId>,
        ) -> Result<(), Diagnostic> {
            Ok(())
        }

        fn validate_connection(&self, _prim: &Prim, _prims: &[Prim]) -> Result<(), Diagnostic> {
            Ok(())
        }

        fn property_group(
            &self,
            _name: &str,
        ) -> Option<beamcraft_model::changeset::DependencyGroup> {
            None
        }
    }

    #[test]
    fn registration_is_pointer_deduplicated() {
        let probe: &'static dyn Discipline = &PROBE;
        register(probe);
        register(probe);
        let installed = registered()
            .iter()
            .filter(|d| same_pointer(**d, probe))
            .count();
        assert_eq!(
            installed, 1,
            "a repeated registration must not repeat the discipline"
        );
        assert!(
            is_part_kind("disciplines-probe-part"),
            "a registered kind is answered by the registry"
        );
    }

    #[test]
    fn the_builtin_steel_discipline_stays_first() {
        let registered = registered();
        let first = registered.first().copied();
        assert!(
            first.is_some_and(|discipline| discipline.part_kinds().contains(&"plate")),
            "the built-in steel discipline is installed first"
        );
    }
}
