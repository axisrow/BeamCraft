// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The BeamCraft IFC4 writer (E02, on the E01 contract `docs/contracts/ifc-export-v0.1.md`).
//!
//! A pure, std-only crate: the caller resolves every value into an [`ExportModel`] and the writer
//! emits one clear-text STEP file through a bounded sink. It reads no store, calls no kernel,
//! touches no file system and reads no clock. The same model gives the same bytes.
//!
//! * [`guid`]: the 22-character `IfcGloballyUniqueId` of an identity, its decoder and the
//!   synthetic ids of entities that are not elements.
//! * [`step`]: REAL numbers, strings, the UTC time stamp and the bounded hashing sink.
//! * [`model`]: the plain input structures and the bounds.
//! * [`write_ifc`]: the fixed entity templates in the order of section 11.2.
//! * [`verify`]: a structural reader used by tests to check a written file. It is a test
//!   harness (E01 section 1.3), not an importer: nothing in BeamCraft builds elements from it.

mod emit;
pub mod guid;
pub mod model;
pub mod step;
pub mod verify;

pub use emit::{
    EXPORTER_VERSION, Subject, Truncation, WriteError, WriteOutcome, global_id, storey_global_id,
    write_ifc,
};
pub use model::{
    Declared, ElementData, Entity, ExportModel, LevelData, Limits, MaterialData, Mesh, Pose,
    QuantityData,
};
