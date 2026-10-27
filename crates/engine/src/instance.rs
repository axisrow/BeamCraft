// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P11 shared-definition groups, independent of storage and UI.
use crate::{command::AuthoredPlate, member::AuthoredMember};
use beamcraft_model::{
    command::{CommandError, CopyElement},
    error::ErrorCode,
    id::ElementId,
    instance::{InstanceLink, PartKind},
};
use std::collections::BTreeMap;

/// One validated, flat graph of owned parts and shared-definition relations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstanceGraph {
    parts: BTreeMap<ElementId, PartKind>,
    links: BTreeMap<ElementId, ElementId>,
}
fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "instance", message)
}
impl InstanceGraph {
    /// Validates all links. Chains, cycles, missing definitions, duplicate sources and different
    /// kinds are refused; a copy of an instance must link directly to its canonical definition.
    pub fn build(
        parts: BTreeMap<ElementId, PartKind>,
        links: &[InstanceLink],
    ) -> Result<Self, CommandError> {
        if parts.len() > crate::limits::MAX_ELEMENTS
            || links.len() > beamcraft_model::instance::MAX_PART_INSTANCES
        {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "instances",
                "too many part instances",
            ));
        }
        let mut graph = Self {
            parts,
            links: BTreeMap::new(),
        };
        for link in links {
            if link.instance == link.definition
                || graph.links.insert(link.instance, link.definition).is_some()
            {
                return Err(invalid("instance has a self or duplicate definition link"));
            }
            let source = graph
                .parts
                .get(&link.instance)
                .ok_or_else(|| invalid("instance is not an owned supported part"))?;
            let target = graph.parts.get(&link.definition).ok_or_else(|| {
                CommandError::new(
                    ErrorCode::DanglingRelationship,
                    "definition",
                    "instance definition is absent",
                )
            })?;
            if source != target {
                return Err(invalid("instance and definition kinds differ"));
            }
        }
        if graph.links.values().any(|id| graph.links.contains_key(id)) {
            return Err(invalid("instance definitions must be canonical parts"));
        }
        Ok(graph)
    }
    /// Canonical definition identity; an independent part owns its own definition.
    pub fn definition(&self, id: ElementId) -> Result<ElementId, CommandError> {
        if !self.parts.contains_key(&id) {
            return Err(CommandError::new(
                ErrorCode::NotFound,
                "part",
                "part is not in the pinned model",
            ));
        }
        Ok(self.links.get(&id).copied().unwrap_or(id))
    }
    /// Whether this part resolves its shape inputs from another part.
    pub fn is_instance(&self, id: ElementId) -> bool {
        self.links.contains_key(&id)
    }
    /// All parts sharing a definition, including its owner, in identity order.
    pub fn group(&self, id: ElementId) -> Result<Vec<ElementId>, CommandError> {
        let definition = self.definition(id)?;
        Ok(self
            .parts
            .keys()
            .filter(|id| self.links.get(id).copied().unwrap_or(**id) == definition)
            .copied()
            .collect())
    }
    /// Number of parts sharing a definition, including its owner. Bounded by the link cap.
    pub fn group_size(&self, id: ElementId) -> Result<usize, CommandError> {
        let definition = self.definition(id)?;
        Ok(1 + self
            .links
            .values()
            .filter(|target| **target == definition)
            .count())
    }
    /// Reads group membership and canonical selection for an atomic shared-definition edit.
    pub fn dependencies(&self, id: ElementId) -> Result<crate::footprint::Footprint, CommandError> {
        use crate::footprint::{Entry, Footprint, Group, Target};
        let mut reads = Vec::new();
        for member in self.group(id)? {
            for group in [Group::Lifecycle, Group::Relationship] {
                reads.push(Entry {
                    target: Target::Element(member),
                    group,
                });
            }
        }
        Footprint::from_entries(&reads, &[])
    }
    /// Persistent links in instance identity order.
    pub fn links(&self) -> impl Iterator<Item = InstanceLink> + '_ {
        self.links
            .iter()
            .map(|(instance, definition)| InstanceLink {
                instance: *instance,
                definition: *definition,
            })
    }
}

/// Instance authored state resolved for evaluation. Shared operations retain the canonical IDs;
/// they are evaluated for each part, while the single authored definition owns the records.
pub fn copy_plate_instance(
    source: &AuthoredPlate,
    command: &CopyElement,
) -> Result<AuthoredPlate, CommandError> {
    let mut next = crate::copy::copy_plate(source, command)?;
    next.cuts.clone_from(&source.cuts);
    next.hole_records.clone_from(&source.hole_records);
    Ok(next)
}
/// Member counterpart of [`copy_plate_instance`].
pub fn copy_member_instance(
    source: &AuthoredMember,
    command: &CopyElement,
) -> Result<AuthoredMember, CommandError> {
    let mut next = crate::copy::copy_member(source, command)?;
    next.cuts.clone_from(&source.cuts);
    Ok(next)
}
