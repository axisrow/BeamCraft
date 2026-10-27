// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Declared read and write sets of a command (F04 section 8).
//!
//! The engine derives them from command semantics; a caller's claim can never narrow them. Each
//! entry names an owned element or an immutable digest and one semantic group. Different property
//! names alone do not prove independence: the groups below are the unit of conflict.
//!
//! P01 knows one operation, so only `Create` is derived. Update, copy and delete footprints
//! arrive with their commands (P04, P05, P08).

use beamcraft_model::command::CommandError;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use std::collections::BTreeSet;

/// What an entry refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// An owned element, by full identity.
    Element(ElementId),
    /// An immutable definition, by full content digest.
    Definition(ContentDigest),
}

/// The semantic groups of F04 section 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// The label.
    Label,
    /// The complete shape-input group (contour, thickness, axis, roll, profile).
    ShapeInputs,
    /// The rigid placement.
    Placement,
    /// The material reference.
    MaterialRef,
    /// Relationships to other elements or definitions.
    Relationship,
    /// The element mark and its approval state (A08).
    Mark,
    /// Existence and identity.
    Lifecycle,
    /// Derived geometry, quantities and their signatures.
    Derived,
}

impl Group {
    /// The F04 token of the group.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Label => "label",
            Self::ShapeInputs => "shape_inputs",
            Self::Placement => "placement",
            Self::MaterialRef => "material_ref",
            Self::Relationship => "relationship",
            Self::Mark => "mark",
            Self::Lifecycle => "lifecycle",
            Self::Derived => "derived",
        }
    }
}

/// One footprint entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entry {
    /// The element or definition.
    pub target: Target,
    /// The semantic group.
    pub group: Group,
}

/// The declared read and write sets of one command, in deterministic order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Footprint {
    reads: BTreeSet<Entry>,
    writes: BTreeSet<Entry>,
}

impl Footprint {
    /// Reconstructs a bounded captured footprint from explicit entries. This checks structure,
    /// not authority: callers must verify its immutable evidence and command provenance before
    /// treating it as an engine-derived dependency set. Raw counts are capped before allocation.
    pub fn from_entries(reads: &[Entry], writes: &[Entry]) -> Result<Self, CommandError> {
        if reads.len().saturating_add(writes.len()) > MAX_FOOTPRINT_ENTRIES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "footprint",
                "captured footprint exceeds 4096 entries",
            ));
        }
        Ok(Self {
            reads: reads.iter().copied().collect(),
            writes: writes.iter().copied().collect(),
        })
    }

    /// The footprint of `CreatePlate` (F04 section 8: "read ID absence and pinned definitions;
    /// write new lifecycle, complete authored fields and relationships"): it reads the absence of
    /// the new ID (lifecycle), reads the pinned material definition's relationship when there is
    /// one, and writes the new element's lifecycle, every authored group (including the material
    /// reference and relationships, written as "none" when there is no material) and the derived
    /// group. A definition is immutable and is never written by this command.
    ///
    /// Fails with `LIMIT_EXCEEDED` if the footprint cannot be bounded (F04: at most 4096 entries).
    pub fn for_create(
        id: ElementId,
        material: Option<ContentDigest>,
    ) -> Result<Self, CommandError> {
        Self::for_create_with(id, None, material)
    }

    /// The footprint of `CreateStraightMember`: that of `CreatePlate` plus the read of the pinned
    /// profile definition, which is required and never written.
    pub fn for_create_member(
        id: ElementId,
        profile: ContentDigest,
        material: Option<ContentDigest>,
    ) -> Result<Self, CommandError> {
        Self::for_create_with(id, Some(profile), material)
    }

    /// The footprint of `AttachReference` (R01): it reads the absence of the new ID and writes the
    /// wrapper's lifecycle, label, placement and its source-input group (the pin and the units
    /// interpretation of a reference). It writes no derived group and names no definition: the
    /// foreign layer is a stored asset the wrapper points at, never a target of any command.
    pub fn for_attach(id: ElementId) -> Result<Self, CommandError> {
        let entry = |group| Entry {
            target: Target::Element(id),
            group,
        };
        let footprint = Self {
            reads: BTreeSet::from([entry(Group::Lifecycle)]),
            writes: BTreeSet::from([
                entry(Group::Lifecycle),
                entry(Group::Label),
                entry(Group::ShapeInputs),
                entry(Group::Placement),
            ]),
        };
        footprint.check_bound()?;
        Ok(footprint)
    }

    /// The footprint of `CopyElement` (P05): it reads the absence of the new ID, the source's
    /// lifecycle and every authored group the copy takes from it, and the pinned definitions
    /// (profile and material); it writes what `CreatePlate` writes for the new ID. The source is
    /// never written.
    pub fn for_copy(
        source: ElementId,
        new_id: ElementId,
        profile: Option<ContentDigest>,
        material: Option<ContentDigest>,
    ) -> Result<Self, CommandError> {
        let mut footprint = Self::for_create_with(new_id, profile, material)?;
        for group in [
            Group::Lifecycle,
            Group::Label,
            Group::ShapeInputs,
            Group::Placement,
            Group::MaterialRef,
            Group::Relationship,
        ] {
            footprint.reads.insert(Entry {
                target: Target::Element(source),
                group,
            });
        }
        footprint.check_bound()?;
        Ok(footprint)
    }

    fn for_create_with(
        id: ElementId,
        profile: Option<ContentDigest>,
        material: Option<ContentDigest>,
    ) -> Result<Self, CommandError> {
        let element = Target::Element(id);
        let entry = |group| Entry {
            target: element,
            group,
        };
        let mut reads = BTreeSet::from([entry(Group::Lifecycle)]);
        for digest in profile.into_iter().chain(material) {
            reads.insert(Entry {
                target: Target::Definition(digest),
                group: Group::Relationship,
            });
        }
        let writes = BTreeSet::from([
            entry(Group::Lifecycle),
            entry(Group::Label),
            entry(Group::ShapeInputs),
            entry(Group::Placement),
            entry(Group::MaterialRef),
            entry(Group::Relationship),
            entry(Group::Derived),
        ]);
        let footprint = Self { reads, writes };
        footprint.check_bound()?;
        Ok(footprint)
    }

    /// The footprint of `UpdateElement` (F04 section 8): it reads the element's lifecycle and the
    /// groups it changes, reads a newly pinned definition, and writes only the groups of the
    /// supported fields it changes, plus the derived group when the shape or the material does.
    /// A rename touches the label alone. A move or an orientation change reads the local shape
    /// and the placement and writes the placement plus the derived group (the world-measurement
    /// invalidation of F04 section 8). The local body is unchanged for either part kind.
    pub fn for_update(
        id: ElementId,
        name: bool,
        material: bool,
        shape: bool,
        placement: bool,
        new_material: Option<ContentDigest>,
    ) -> Result<Self, CommandError> {
        let entry = |group| Entry {
            target: Target::Element(id),
            group,
        };
        let mut reads = BTreeSet::from([entry(Group::Lifecycle)]);
        let mut writes = BTreeSet::new();
        if name {
            reads.insert(entry(Group::Label));
            writes.insert(entry(Group::Label));
        }
        if material {
            reads.insert(entry(Group::ShapeInputs));
            writes.extend([
                entry(Group::MaterialRef),
                entry(Group::Relationship),
                entry(Group::Derived),
            ]);
            if let Some(digest) = new_material {
                reads.insert(Entry {
                    target: Target::Definition(digest),
                    group: Group::Relationship,
                });
            }
        }
        if shape {
            reads.insert(entry(Group::ShapeInputs));
            writes.extend([entry(Group::ShapeInputs), entry(Group::Derived)]);
        }
        if placement {
            reads.extend([entry(Group::ShapeInputs), entry(Group::Placement)]);
            writes.extend([entry(Group::Placement), entry(Group::Derived)]);
        }
        let footprint = Self { reads, writes };
        footprint.check_bound()?;
        Ok(footprint)
    }

    /// The footprint of `DeleteElement` (P08): it reads the element's lifecycle and the
    /// relationships (the incoming-relationship index that decides whether deletion is allowed),
    /// and writes the element's whole authored state and its derived group, because removing the
    /// prim removes every group it holds. Pinned definitions are not read or written: the
    /// element's links to them go away with it and the definitions are immutable.
    pub fn for_delete(id: ElementId) -> Result<Self, CommandError> {
        let entry = |group| Entry {
            target: Target::Element(id),
            group,
        };
        let footprint = Self {
            reads: BTreeSet::from([entry(Group::Lifecycle), entry(Group::Relationship)]),
            writes: BTreeSet::from([
                entry(Group::Lifecycle),
                entry(Group::Label),
                entry(Group::ShapeInputs),
                entry(Group::Placement),
                entry(Group::MaterialRef),
                entry(Group::Relationship),
                entry(Group::Derived),
            ]),
        };
        footprint.check_bound()?;
        Ok(footprint)
    }

    /// The footprint of `CreateWeld` (A06): that of `CreatePlate` for the new weld, which writes
    /// its lifecycle, every authored group (the weld properties are its shape inputs, the
    /// `connects` record its relationships) and the derived group, plus the read of each connected
    /// part's lifecycle: their existence and kind decide acceptance. The `connects` record lives
    /// with the weld (domain vocabulary section 5.2), so no part is written. A later delete of a
    /// part writes its lifecycle and therefore conflicts with a concurrent weld creation.
    pub fn for_weld(id: ElementId, parts: [ElementId; 2]) -> Result<Self, CommandError> {
        let mut footprint = Self::for_create_with(id, None, None)?;
        for part in parts {
            footprint.reads.insert(Entry {
                target: Target::Element(part),
                group: Group::Lifecycle,
            });
        }
        footprint.check_bound()?;
        Ok(footprint)
    }

    /// The footprint of `CreateBolt` (A05): the same shape as [`Self::for_weld`], for a bolt
    /// group: it writes the group's own lifecycle, every authored group (the bolt properties are
    /// its shape inputs, the `connects` record its relationships) and the derived group, plus the
    /// read of each connected part's lifecycle. The record lives with the group, so no part is
    /// written and a later delete of a part conflicts with a concurrent group creation.
    pub fn for_bolt(id: ElementId, parts: [ElementId; 2]) -> Result<Self, CommandError> {
        Self::for_weld(id, parts)
    }

    /// The footprint of `AttachToAssembly` and `DetachFromAssembly` (A01): the `part-of` record
    /// lives with its source, so only the part's relationship group is written. It reads the
    /// part's lifecycle and relationships and, for an attach, the lifecycle and relationships of
    /// every assembly on the chain from the target upwards: they decide the target kind and the
    /// absence of a cycle, so a concurrent change of any of them must conflict. A detach passes
    /// an empty chain; the former assembly is neither read nor written.
    pub fn for_part_of(part: ElementId, chain: &[ElementId]) -> Result<Self, CommandError> {
        let mut reads = BTreeSet::new();
        for id in core::iter::once(&part).chain(chain) {
            for group in [Group::Lifecycle, Group::Relationship] {
                reads.insert(Entry {
                    target: Target::Element(*id),
                    group,
                });
            }
        }
        let footprint = Self {
            reads,
            writes: BTreeSet::from([Entry {
                target: Target::Element(part),
                group: Group::Relationship,
            }]),
        };
        footprint.check_bound()?;
        Ok(footprint)
    }

    /// The footprint of `SetAssemblyMainPart` (A16.1): the record lives on the assembly, so only
    /// the assembly's relationship group is written. It reads the assembly's lifecycle and
    /// relationships (its existence and kind decide acceptance) and the part's lifecycle and
    /// relationships: its existence and its membership decide whether it is a direct part of the
    /// assembly, so a concurrent detach of the part must conflict.
    pub fn for_main_part(assembly: ElementId, part: ElementId) -> Result<Self, CommandError> {
        let mut reads = BTreeSet::new();
        for id in [assembly, part] {
            for group in [Group::Lifecycle, Group::Relationship] {
                reads.insert(Entry {
                    target: Target::Element(id),
                    group,
                });
            }
        }
        let footprint = Self {
            reads,
            writes: BTreeSet::from([Entry {
                target: Target::Element(assembly),
                group: Group::Relationship,
            }]),
        };
        footprint.check_bound()?;
        Ok(footprint)
    }

    /// The footprint of `ExplodeAssembly` (A16): the assembly element is deleted, so its whole
    /// authored state is written ([`Self::for_delete`]), and every direct part's `part-of` record
    /// is removed, so each part's relationship group is written and its lifecycle and
    /// relationships are read: their existence and membership decide the parts of the assembly,
    /// so a concurrent attach, detach or delete of any of them must conflict. A part that is a
    /// nested assembly is only detached; its own parts are neither read nor written.
    pub fn for_explode(assembly: ElementId, parts: &[ElementId]) -> Result<Self, CommandError> {
        let mut footprint = Self::for_delete(assembly)?;
        for part in parts {
            footprint = footprint.union(Self::for_part_of(*part, &[])?)?;
        }
        Ok(footprint)
    }

    /// The footprint of a level assignment or unassignment (D2): the `bim:level` record lives with
    /// its source, so only the element's relationship group is written. It reads the element's
    /// lifecycle and relationships and, for an assignment, the level's lifecycle: its existence and
    /// kind decide acceptance, so a concurrent delete of the level must conflict. A level change
    /// has no geometric effect and writes no derived group.
    pub fn for_level(element: ElementId, level: Option<ElementId>) -> Result<Self, CommandError> {
        let mut reads = BTreeSet::new();
        for group in [Group::Lifecycle, Group::Relationship] {
            reads.insert(Entry {
                target: Target::Element(element),
                group,
            });
        }
        if let Some(level) = level {
            reads.insert(Entry {
                target: Target::Element(level),
                group: Group::Lifecycle,
            });
        }
        let footprint = Self {
            reads,
            writes: BTreeSet::from([Entry {
                target: Target::Element(element),
                group: Group::Relationship,
            }]),
        };
        footprint.check_bound()?;
        Ok(footprint)
    }

    /// The footprint of a mark change (A08: assign, replace, revoke or approve): the record
    /// lives with its source, so only the element's mark group is written. It reads the element's
    /// lifecycle (its existence decides acceptance) and its mark group (the current value and
    /// approval state decide it), so a concurrent change of either must conflict. A mark change
    /// has no geometric effect and writes no derived group. The withdrawal of an approval under a
    /// shape-input edit is `for_update` unioned with this.
    pub fn for_mark(element: ElementId) -> Result<Self, CommandError> {
        let entry = |group| Entry {
            target: Target::Element(element),
            group,
        };
        let footprint = Self {
            reads: BTreeSet::from([entry(Group::Lifecycle), entry(Group::Mark)]),
            writes: BTreeSet::from([entry(Group::Mark)]),
        };
        footprint.check_bound()?;
        Ok(footprint)
    }

    /// The footprint of two commands applied as one change: the union of their reads and writes.
    pub fn union(mut self, other: Self) -> Result<Self, CommandError> {
        self.reads.extend(other.reads);
        self.writes.extend(other.writes);
        self.check_bound()?;
        Ok(self)
    }

    /// The footprint of publishing one immutable definition (P02): it writes the definition's own
    /// entry and no element. A definition is content-addressed, so two commands that publish the
    /// same digest write identical bytes; the entry is still a write, because another command
    /// that reads the definition must not run against a half-published one.
    pub fn for_define(digest: ContentDigest) -> Self {
        let entry = Entry {
            target: Target::Definition(digest),
            group: Group::Relationship,
        };
        Self {
            reads: BTreeSet::from([entry]),
            writes: BTreeSet::from([entry]),
        }
    }

    fn check_bound(&self) -> Result<(), CommandError> {
        if self.reads.len().saturating_add(self.writes.len()) > MAX_FOOTPRINT_ENTRIES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "footprint",
                "footprint exceeds 4096 entries",
            ));
        }
        Ok(())
    }

    /// Entries the command reads.
    pub const fn reads(&self) -> &BTreeSet<Entry> {
        &self.reads
    }

    /// Entries the command writes.
    pub const fn writes(&self) -> &BTreeSet<Entry> {
        &self.writes
    }

    /// Whether two commands conflict: one writes an entry the other reads or writes.
    pub fn conflicts_with(&self, other: &Self) -> bool {
        self.writes
            .iter()
            .any(|e| other.writes.contains(e) || other.reads.contains(e))
            || other.writes.iter().any(|e| self.reads.contains(e))
    }

    /// Whether every entry of `observed` is covered by this declared set: the layer engine's
    /// own conservative footprint may only be a subset of what the engine declared.
    pub fn covers_writes(&self, observed: &BTreeSet<Entry>) -> bool {
        observed.is_subset(&self.writes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    #[test]
    fn create_reads_the_absence_and_writes_the_complete_authored_state() {
        let f = Footprint::for_create(id(5), None).unwrap();
        assert_eq!(f.reads().len(), 1);
        assert!(f.reads().iter().all(|e| e.group == Group::Lifecycle));
        let groups: Vec<_> = f.writes().iter().map(|e| e.group).collect();
        for group in [
            Group::Lifecycle,
            Group::Label,
            Group::ShapeInputs,
            Group::Placement,
            Group::MaterialRef,
            Group::Relationship,
            Group::Derived,
        ] {
            assert!(groups.contains(&group), "{group:?}");
        }
        assert!(
            f.writes()
                .iter()
                .all(|e| e.target == Target::Element(id(5)))
        );
    }

    #[test]
    fn publishing_a_definition_writes_only_that_definition() {
        let digest = beamcraft_model::digest::Sha256::digest(b"definition").unwrap();
        let other = beamcraft_model::digest::Sha256::digest(b"other").unwrap();
        let publish = Footprint::for_define(digest);
        assert!(
            publish
                .writes()
                .iter()
                .all(|e| e.target == Target::Definition(digest))
        );
        // A plate that pins it conflicts with the publication, one that pins another does not.
        assert!(publish.conflicts_with(&Footprint::for_create(id(5), Some(digest)).unwrap()));
        assert!(!publish.conflicts_with(&Footprint::for_create(id(5), Some(other)).unwrap()));
    }

    #[test]
    fn a_pinned_material_is_read_never_written() {
        let digest = beamcraft_model::digest::Sha256::digest(b"material").unwrap();
        let f = Footprint::for_create(id(5), Some(digest)).unwrap();
        let read = Entry {
            target: Target::Definition(digest),
            group: Group::Relationship,
        };
        assert!(f.reads().contains(&read));
        assert!(
            f.writes()
                .iter()
                .all(|e| e.target == Target::Element(id(5)))
        );
        // Two plates that pin the same definition only read it, so they stay independent.
        let other = Footprint::for_create(id(6), Some(digest)).unwrap();
        assert!(!f.conflicts_with(&other));
    }

    #[test]
    fn different_new_elements_are_independent_the_same_one_conflicts() {
        let a = Footprint::for_create(id(5), None).unwrap();
        let b = Footprint::for_create(id(6), None).unwrap();
        assert!(!a.conflicts_with(&b));
        assert!(!b.conflicts_with(&a));
        assert!(a.conflicts_with(&a));
    }

    #[test]
    fn the_declared_set_covers_a_subset_and_rejects_anything_wider() {
        let f = Footprint::for_create(id(5), None).unwrap();
        let narrower: BTreeSet<Entry> = f.writes().iter().copied().take(2).collect();
        assert!(f.covers_writes(&narrower));
        let mut wider = f.writes().clone();
        wider.insert(Entry {
            target: Target::Element(id(9)),
            group: Group::Label,
        });
        assert!(!f.covers_writes(&wider));
    }

    #[test]
    fn a_move_writes_the_placement_and_the_derived_invalidation_only() {
        let f = Footprint::for_update(id(5), false, false, false, true, None).unwrap();
        let groups: BTreeSet<_> = f.writes().iter().map(|e| e.group).collect();
        assert_eq!(groups, BTreeSet::from([Group::Placement, Group::Derived]));
        assert!(f.reads().iter().any(|e| e.group == Group::Placement));
        assert!(f.reads().iter().any(|e| e.group == Group::ShapeInputs));
        // Moving one element does not conflict with moving another, and conflicts with a rename
        // only through nothing: the label group is not written.
        let other = Footprint::for_update(id(6), false, false, false, true, None).unwrap();
        assert!(!f.conflicts_with(&other));
        let rename = Footprint::for_update(id(5), true, false, false, false, None).unwrap();
        assert!(!f.conflicts_with(&rename));
    }

    #[test]
    fn a_delete_reads_the_links_and_writes_the_whole_element() {
        let f = Footprint::for_delete(id(5)).unwrap();
        let writes: BTreeSet<_> = f.writes().iter().map(|e| e.group).collect();
        assert_eq!(writes.len(), 7);
        assert!(
            f.writes()
                .iter()
                .all(|e| e.target == Target::Element(id(5)))
        );
        assert!(f.reads().iter().any(|e| e.group == Group::Relationship));
        // It conflicts with anything that touches the same element and with nothing else.
        let update = Footprint::for_update(id(5), true, false, false, false, None).unwrap();
        assert!(f.conflicts_with(&update));
        assert!(!f.conflicts_with(&Footprint::for_delete(id(6)).unwrap()));
        // A copy of the element reads its lifecycle, so it cannot run against the delete.
        assert!(f.conflicts_with(&Footprint::for_copy(id(5), id(7), None, None).unwrap()));
    }

    #[test]
    fn a_mark_change_writes_only_the_target_mark_and_reads_it_back() {
        let f = Footprint::for_mark(id(5)).unwrap();
        assert_eq!(
            f.writes().iter().copied().collect::<Vec<_>>(),
            vec![Entry {
                target: Target::Element(id(5)),
                group: Group::Mark,
            }]
        );
        assert_eq!(f.reads().len(), 2);
        // Groups are the conflict unit: two marks of different elements are independent, and a
        // rename of the same element touches the label group, not the mark group.
        assert!(!f.conflicts_with(&Footprint::for_mark(id(6)).unwrap()));
        let rename = Footprint::for_update(id(5), true, false, false, false, None).unwrap();
        assert!(!f.conflicts_with(&rename));
        assert!(!rename.conflicts_with(&f));
        // The same element's mark group conflicts with itself, and a delete conflicts through
        // the lifecycle read.
        assert!(f.conflicts_with(&Footprint::for_mark(id(5)).unwrap()));
        assert!(f.conflicts_with(&Footprint::for_delete(id(5)).unwrap()));
        assert_eq!(Group::Mark.as_str(), "mark");
    }

    #[test]
    fn group_tokens_are_the_f04_names() {
        assert_eq!(Group::ShapeInputs.as_str(), "shape_inputs");
        assert_eq!(Group::MaterialRef.as_str(), "material_ref");
    }

    #[test]
    fn a_main_part_change_writes_the_assembly_and_reads_both_ends() {
        let f = Footprint::for_main_part(id(5), id(6)).unwrap();
        assert_eq!(
            f.writes().iter().copied().collect::<Vec<_>>(),
            vec![Entry {
                target: Target::Element(id(5)),
                group: Group::Relationship,
            }]
        );
        assert_eq!(f.reads().len(), 4);
        // A concurrent detach of the part conflicts; an unrelated assembly's main does not.
        assert!(f.conflicts_with(&Footprint::for_part_of(id(6), &[]).unwrap()));
        assert!(f.conflicts_with(&Footprint::for_delete(id(6)).unwrap()));
        assert!(!f.conflicts_with(&Footprint::for_main_part(id(7), id(8)).unwrap()));
    }

    #[test]
    fn an_explode_writes_the_assembly_and_the_parts_relationships() {
        let f = Footprint::for_explode(id(5), &[id(6), id(7)]).unwrap();
        for (target, group) in [
            (id(5), Group::Lifecycle),
            (id(5), Group::Relationship),
            (id(6), Group::Relationship),
            (id(7), Group::Relationship),
        ] {
            assert!(
                f.writes().contains(&Entry {
                    target: Target::Element(target),
                    group
                }),
                "{target:?} {group:?}"
            );
        }
        // Every direct part conflicts: a concurrent attach or delete of one must lose.
        assert!(f.conflicts_with(&Footprint::for_part_of(id(6), &[]).unwrap()));
        assert!(f.conflicts_with(&Footprint::for_delete(id(6)).unwrap()));
        // Parts of a nested sub-assembly are untouched by the explode of id(5).
        assert!(!f.conflicts_with(&Footprint::for_part_of(id(8), &[]).unwrap()));
    }

    #[test]
    fn a_part_of_change_writes_only_the_part_relationship_and_reads_the_chain() {
        let f = Footprint::for_part_of(id(5), &[id(6), id(7)]).unwrap();
        assert_eq!(
            f.writes().iter().copied().collect::<Vec<_>>(),
            vec![Entry {
                target: Target::Element(id(5)),
                group: Group::Relationship,
            }]
        );
        assert_eq!(f.reads().len(), 6);
        // Re-parenting the grandparent conflicts with the attach; an unrelated part does not.
        assert!(f.conflicts_with(&Footprint::for_part_of(id(7), &[id(8)]).unwrap()));
        assert!(!f.conflicts_with(&Footprint::for_part_of(id(9), &[id(6)]).unwrap()));
        // Deleting the target assembly conflicts.
        assert!(f.conflicts_with(&Footprint::for_delete(id(6)).unwrap()));
    }

    #[test]
    fn a_level_change_writes_the_element_relationship_and_reads_the_level() {
        let f = Footprint::for_level(id(5), Some(id(6))).unwrap();
        assert_eq!(
            f.writes().iter().copied().collect::<Vec<_>>(),
            vec![Entry {
                target: Target::Element(id(5)),
                group: Group::Relationship,
            }]
        );
        assert!(f.reads().contains(&Entry {
            target: Target::Element(id(6)),
            group: Group::Lifecycle,
        }));
        // Deleting the level conflicts with the assignment; assigning another element does not.
        assert!(f.conflicts_with(&Footprint::for_delete(id(6)).unwrap()));
        assert!(!f.conflicts_with(&Footprint::for_level(id(7), Some(id(6))).unwrap()));
        // An unassignment reads no level.
        let clear = Footprint::for_level(id(5), None).unwrap();
        assert_eq!(clear.reads().len(), 2);
        // A level change next to a rename is the union of both.
        let rename = Footprint::for_update(id(5), true, false, false, false, None).unwrap();
        let both = rename.clone().union(f.clone()).unwrap();
        assert_eq!(both.writes().len(), 2);
        assert!(both.covers_writes(f.writes()) && both.covers_writes(rename.writes()));
    }
}
