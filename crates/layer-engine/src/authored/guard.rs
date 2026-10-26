// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U03 guards that consult the element index before a write or delete is planned.
//! A grant proves index evidence only; host authorization and F04/F06 validation stay separate.

use super::*;
use crate::index::{ElementId, ElementIndex, Ownership, dependent_collision};

/// Index-backed write verdict for one owned element. It is not host authorization and is only
/// as fresh as the context it names; any install or reopen makes it stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteGrant {
    /// Full element identity.
    pub id: ElementId,
    /// Canonical element path.
    pub path: String,
    /// The owned authored layer a delta for this element must write into.
    pub write_layer: String,
    /// Generation of the pinned context the verdict was computed against.
    pub generation: u64,
}

impl Author {
    /// Builds the complete index of the pinned snapshot.
    pub fn element_index(
        &self,
        context: &Context,
        control: &Control,
    ) -> Result<ElementIndex, Diagnostic> {
        self.reader.element_index(context, control)
    }
    /// Proves a complete, live, untampered index (see [`Reader::verify_index`]).
    pub fn verify_index(&self, index: &ElementIndex, control: &Control) -> Result<(), Diagnostic> {
        self.reader.verify_index(index, control)
    }
    /// Grants an index-backed write verdict. A stale index is `STALE_WORKSPACE`, a partial one
    /// `DEPENDENCY_UNAVAILABLE`, a forged one `INTEGRITY_ERROR`, a missing element `NOT_FOUND`
    /// and any foreign contribution `FOREIGN_READ_ONLY`.
    pub fn authorize_write(
        &self,
        index: &ElementIndex,
        id: &ElementId,
        control: &Control,
    ) -> Result<WriteGrant, Diagnostic> {
        self.verify_index(index, control)?;
        let entry = index.element(id)?;
        if entry.ownership != Ownership::Owned {
            return Err(fail(
                ErrorCode::ForeignReadOnly,
                "foreign composition is read-only",
            ));
        }
        Ok(WriteGrant {
            id: *id,
            path: entry.path.clone(),
            write_layer: self.targets.authored.clone(),
            generation: index.context().identity().generation,
        })
    }
    /// Checks that deleting the element is not blocked by an incoming relationship anywhere in
    /// the pinned view. Incomplete evidence is `DEPENDENCY_UNAVAILABLE`, never "no links".
    pub fn check_delete(
        &self,
        index: &ElementIndex,
        id: &ElementId,
        control: &Control,
    ) -> Result<(), Diagnostic> {
        self.authorize_write(index, id, control)?;
        if index.delete_blockers(id)?.is_empty() {
            Ok(())
        } else {
            Err(fail(
                ErrorCode::DependencyConflict,
                "element has incoming relationships",
            )
            .with_rule("DV-REL-009"))
        }
    }
    /// Certifies that lifecycle writes (delete, deactivate, create) do not collide with reads of
    /// the written ID, even when absent, or of elements that depend on it. `writes` and `reads`
    /// must come from different operations: one operation's own reads of an ID it writes would
    /// collide. Only a complete, live index can certify.
    pub fn certify_dependent_reads(
        &self,
        index: &ElementIndex,
        writes: &BTreeSet<Footprint>,
        reads: &BTreeSet<Footprint>,
        control: &Control,
    ) -> Result<(), Diagnostic> {
        self.verify_index(index, control)?;
        dependent_collision(index, writes, reads)
    }
}
