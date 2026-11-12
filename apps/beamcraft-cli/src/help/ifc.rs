// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the ifc input command family.

pub(super) const HELP: &str = concat!(
    "IFC input, read-only (E04 study; the reference workflow contract XR-SOURCE-002): ",
    "ifc-inspect --source FILE (pins what one received IFC file declares, from the bytes ",
    "alone: the SHA-256 and size of the original bytes, the declared FILE_SCHEMA, the ",
    "descriptive FILE_NAME header, the entity inventory, the GlobalId inventory with its ",
    "duplicates, the declared project length unit in metres per unit, the conversion-based ",
    "length units and an explicit loss list of everything not interpreted; refuses bytes ",
    "that are not an ISO 10303-21 physical file or larger than 32 MiB; opens no store and ",
    "writes nothing; this measures the input, it converts and imports nothing). ",
    "ifc-shapes --source FILE (reads the placed triangle bodies of the same file: every ",
    "product whose representation is a tessellated IfcProductDefinitionShape, with its ",
    "resolved IfcLocalPlacement chain, the declared length unit, per-body vertex, triangle ",
    "and bounds facts in millimetres and its own loss list; a file without a usable SI ",
    "length unit counts its products but reads no body; nothing is converted into a ",
    "BeamCraft element and no store is opened)",
);
