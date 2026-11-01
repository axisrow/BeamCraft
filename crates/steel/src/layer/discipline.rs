// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The steel discipline: what the platform asks when it validates and indexes the steel parts
//! and connections of a loaded layer (#388).
//!
//! The steel half of the read seam. The platform's registry
//! (`beamcraft_layer_engine::authored::disciplines`) starts empty; [`install`] registers the
//! steel discipline in it, deduplicated by pointer, and every platform question is answered by
//! the registered disciplines in registration order.

use super::definition;
use super::read;
use beamcraft_layer_engine::authored::Definition;
use beamcraft_layer_engine::authored::disciplines::{Discipline, register};
use beamcraft_layer_engine::{Diagnostic, Prim};
use beamcraft_model::changeset::DependencyGroup;
use std::collections::BTreeSet;
use std::fmt::Debug;

/// The steel discipline: plates, beams, columns, welds and bolt groups.
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
        read::steel_part(kind, prim, prims, operation_ids)
    }

    fn validate_connection(&self, prim: &Prim, prims: &[Prim]) -> Result<(), Diagnostic> {
        read::steel_connection(prim, prims)
    }

    fn property_group(&self, name: &str) -> Option<DependencyGroup> {
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
            | "steel:bolt:washerUnderNut" => DependencyGroup::ShapeInputs,
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

    fn read_definition(&self, prim: &Prim, kind: &str) -> Result<Option<Definition>, Diagnostic> {
        definition::read_definition(prim, kind)
    }

    fn delete_attributes(&self) -> &'static [(&'static str, &'static str)] {
        use beamcraft_layer_engine::authored::cut_history;
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
        read::connects(prims)
    }
}

static STEEL: Steel = Steel;

/// Registers the steel discipline with the platform registry. A repeated call is kept, not
/// duplicated, so a re-entry cannot shift the registration order.
pub fn install() {
    register(&STEEL);
}

#[cfg(test)]
mod tests {
    use beamcraft_layer_engine::authored::disciplines::registered;

    #[test]
    fn install_is_idempotent_and_names_the_steel_part_kinds() {
        super::install();
        super::install();
        let installed = registered();
        let steel = installed
            .iter()
            .find(|discipline| discipline.part_kinds().contains(&"plate"))
            .expect("the steel discipline is registered");
        assert_eq!(steel.part_kinds(), &["plate", "beam", "column"]);
        let entries = registered()
            .iter()
            .filter(|discipline| discipline.part_kinds().contains(&"plate"))
            .count();
        assert_eq!(
            entries, 1,
            "a repeated install must not repeat the discipline"
        );
    }
}
