// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The read half of the discipline seam (#795): what the platform asks a discipline when it
//! validates and indexes a loaded layer.
//!
//! The write half is the hook carried by `Shape::Discipline` and `Change::Discipline`; a loaded
//! layer has no element object to carry behaviour, so the read half is a registry. Today the
//! registry holds the one built-in steel discipline; the apps will install it from the `steel`
//! crate once the steel half of this crate has moved there.

use super::*;
use std::collections::BTreeSet;
use std::fmt::Debug;

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
        rebase::steel_property_group(name)
    }
}

static STEEL: Steel = Steel;
static REGISTERED: [&dyn Discipline; 1] = [&STEEL];

/// The disciplines the platform reads, in registration order.
pub fn registered() -> &'static [&'static dyn Discipline] {
    &REGISTERED
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

/// The write group of a property name owned by a registered discipline.
pub(super) fn property_group(name: &str) -> Option<beamcraft_model::changeset::DependencyGroup> {
    registered()
        .iter()
        .find_map(|discipline| discipline.property_group(name))
}
