// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the export command family.

pub(super) const COMMANDS: &[&str] = &["export-ifc", "ifc-guid"];

pub(super) const HELP: &str = concat!(
    "Export (E02, IFC4 STEP of one committed revision; experimental, E01 contract 0.1.0): ",
    "export-ifc --store ROOT --out FILE [--revision UUID] [--project-name TEXT] [--schema ",
    "IFC4] [--format step] [--strict] [--overwrite] [--report FILE] (the head or an explicit ",
    "revision: plates, beams and columns as IfcPlate, IfcBeam and IfcColumn with their kernel ",
    "mesh as an IfcTriangulatedFaceSet, every other kind such as assemblies, welds and ",
    "references as IfcBuildingElementProxy, levels as IfcBuildingStorey, the pinned material, ",
    "published current quantities and the property sets BeamCraft_Element and ",
    "BeamCraft_Declared; a deactivated element is not exported; stale, missing or unknown ",
    "quantities are left out and listed, never zero; the file is created new unless ",
    "--overwrite and is installed only after a complete write; the JSON report lists every ",
    "element with its GlobalId, the findings and the losses; --strict refuses a plate, beam ",
    "or column that would leave without mesh or placement; the ProjectId is read from the ",
    "store and there is no --project-id; no IFC is ever read); ifc-guid --id UUID | --decode ",
    "GLOBALID (the deterministic 22-character GlobalId of an ElementId, INVALID_ID for ",
    "anything but a canonical UUIDv7, or the reverse)",
);
