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

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::command::{CutRecord, HoleRecord, KeepSide, Placement};
    use beamcraft_model::digest::ContentDigest;
    use beamcraft_model::id::OpId;

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn link(instance: u8, definition: u8) -> InstanceLink {
        InstanceLink {
            instance: id(instance),
            definition: id(definition),
        }
    }

    fn parts() -> BTreeMap<ElementId, PartKind> {
        BTreeMap::from([
            (id(1), PartKind::Plate),
            (id(2), PartKind::Plate),
            (id(3), PartKind::Plate),
            (id(4), PartKind::Beam),
        ])
    }

    #[test]
    fn build_refuses_self_duplicate_missing_and_kind_mismatched_links() {
        // A self link.
        assert!(InstanceGraph::build(parts(), &[link(1, 1)]).is_err());
        // Two links from one instance.
        assert!(InstanceGraph::build(parts(), &[link(2, 1), link(2, 3)]).is_err());
        // The instance or the definition is not an owned part.
        assert!(InstanceGraph::build(parts(), &[link(9, 1)]).is_err());
        let dangling = InstanceGraph::build(parts(), &[link(2, 9)]).unwrap_err();
        assert_eq!(dangling.code, ErrorCode::DanglingRelationship);
        // The kinds of instance and definition differ.
        assert!(InstanceGraph::build(parts(), &[link(4, 1)]).is_err());
    }

    #[test]
    fn build_refuses_chains_and_cycles() {
        // A chain: the definition of an instance is never itself an instance.
        assert!(InstanceGraph::build(parts(), &[link(2, 1), link(3, 2)]).is_err());
        // A two-link cycle is refused by the same rule.
        assert!(InstanceGraph::build(parts(), &[link(2, 1), link(1, 2)]).is_err());
    }

    #[test]
    fn a_group_resolves_its_canonical_definition_and_size() {
        let g = InstanceGraph::build(parts(), &[link(2, 1), link(3, 1)]).unwrap();
        // Instances resolve to their canonical definition.
        assert_eq!(g.definition(id(2)).unwrap(), id(1));
        assert_eq!(g.definition(id(3)).unwrap(), id(1));
        // The definition and independent parts own themselves.
        assert_eq!(g.definition(id(1)).unwrap(), id(1));
        assert_eq!(g.definition(id(4)).unwrap(), id(4));
        // A part outside the pinned model is refused.
        assert_eq!(g.definition(id(9)).unwrap_err().code, ErrorCode::NotFound);
        // The group is every member in identity order, owner included.
        assert_eq!(g.group(id(2)).unwrap(), vec![id(1), id(2), id(3)]);
        assert_eq!(g.group(id(1)).unwrap(), vec![id(1), id(2), id(3)]);
        assert_eq!(g.group(id(4)).unwrap(), vec![id(4)]);
        assert_eq!(g.group_size(id(3)).unwrap(), 3);
        assert_eq!(g.group_size(id(4)).unwrap(), 1);
        // Instances are marked; the definition and independent parts are not.
        assert!(g.is_instance(id(2)));
        assert!(!g.is_instance(id(1)));
        assert!(!g.is_instance(id(4)));
    }

    fn plate() -> AuthoredPlate {
        AuthoredPlate {
            id: id(1),
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
            corners: Vec::new(),
            placement: Placement {
                translation_mm: [10.0, 20.0, 30.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: vec![HoleRecord {
                operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000aa").unwrap(),
                ordinal: 0,
                polygon_mm: vec![[10., 10.], [20., 10.], [20., 20.], [10., 20.]],
                kind: beamcraft_model::command::HoleKind::Hole,
            }],
            cuts: vec![],
            bends: Vec::new(),
        }
    }

    fn command() -> CopyElement {
        CopyElement {
            source_id: id(1),
            new_id: id(2),
            name: Some("Instance".into()),
            offset_mm: [100.0, 0.0, 0.0],
        }
    }

    #[test]
    fn a_plate_instance_copy_retains_the_canonical_records() {
        let mut source = plate();
        source.cuts = vec![CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000ab").unwrap(),
            ordinal: 0,
            point_mm: [500., 0., 0.],
            normal: [1., 0., 0.],
            keep: KeepSide::Negative,
        }];
        let copy = copy_plate_instance(&source, &command()).unwrap();
        // Identity, label and placement are the copy's own.
        assert_eq!(copy.id, id(2));
        assert_eq!(copy.name, "Instance");
        assert_eq!(copy.placement.translation_mm, [110.0, 20.0, 30.0]);
        // The shared-definition records stay canonical: byte for byte the definition's.
        assert_eq!(copy.hole_records, source.hole_records);
        assert_eq!(copy.cuts, source.cuts);
        // An independent duplicate mints fresh record identities instead.
        let duplicate = crate::copy::copy_plate(&source, &command()).unwrap();
        assert_ne!(duplicate.hole_records, source.hole_records);
        assert_ne!(duplicate.cuts, source.cuts);
    }

    #[test]
    fn a_member_instance_copy_retains_the_canonical_cuts() {
        let source = AuthoredMember {
            curved: false,
            id: id(1),
            kind: beamcraft_model::command::MemberKind::Beam,
            name: "Beam".into(),
            class: 0,
            axis_points_mm: vec![[0.; 3], [2000., 0., 0.]],
            profile: ContentDigest::from_bytes([7; 32]),
            roll_rad: None,
            section_placement: None,
            placement: Placement {
                translation_mm: [0., 0., 0.],
                orientation_xyzw: [0., 0., 0., 1.],
            },
            material: None,
            cuts: vec![CutRecord {
                operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000ac").unwrap(),
                ordinal: 0,
                point_mm: [1000., 0., 0.],
                normal: [1., 0., 0.],
                keep: KeepSide::Negative,
            }],
            hole_records: Vec::new(),
        };
        let copy = copy_member_instance(&source, &command()).unwrap();
        assert_eq!(copy.id, id(2));
        assert_eq!(copy.placement.translation_mm, [100.0, 0.0, 0.0]);
        assert_eq!(copy.cuts, source.cuts);
    }
}
