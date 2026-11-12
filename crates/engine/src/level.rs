// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D2 levels (storeys): the engine's decision on `CreateLevel` and on the level assignment of
//! `UpdateElement` over the `bim:level` records of a model.
//!
//! A level is an element of kind `level` with its own UUIDv7, a name and an elevation. It has no
//! geometry and no placement, and the elevation moves nothing: changing or assigning a level
//! changes no element's shape key, mesh or quantity. The record lives with its source (the plate,
//! beam or column), so assigning writes only that element; the level is only read.
//!
//! The engine never touches USD. The adapter reads the element kinds, the levels and the current
//! records from the pinned model into a [`Levels`]; the decision here is pure, bounded and
//! deterministic. The layer engine re-checks the same rules on the authored candidate, so a defect
//! in either place fails closed.
//!
//! Rules: only a plate, beam or column is assigned (`UNSUPPORTED_PROPERTY` otherwise); an unknown
//! level is `DANGLING_RELATIONSHIP`; a target that is not a level is `INVALID_ARGUMENT`; an element
//! has at most one level, and assigning another replaces it (a record, not a collection).

use crate::footprint::Footprint;
use beamcraft_model::command::{CommandError, CreateLevel, UpdateLevel};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use beamcraft_model::level::CreateLevels;
use std::collections::{BTreeMap, BTreeSet};

/// The kind of an element that can be assigned to a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HolderKind {
    /// A plate.
    Plate,
    /// A beam.
    Beam,
    /// A column.
    Column,
}

/// The stored fields of a level.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelRecord {
    /// Label.
    pub name: String,
    /// Elevation in millimetres.
    pub elevation_mm: f64,
}

/// The elements, levels and level records of a model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Levels {
    holders: BTreeMap<ElementId, HolderKind>,
    levels: BTreeMap<ElementId, LevelRecord>,
    assigned: BTreeMap<ElementId, ElementId>,
    others: BTreeSet<ElementId>,
}

/// An accepted level creation: the declared footprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creation {
    /// The declared footprint.
    pub footprint: Footprint,
}

/// An accepted level update: only the fields that differ from the head.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelUpdate {
    /// The new label, when it differs.
    pub name: Option<String>,
    /// The new elevation in millimetres, when it differs.
    pub elevation_mm: Option<f64>,
    /// The declared footprint.
    pub footprint: Footprint,
}

/// An accepted assignment or unassignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// The level the element had before, if any.
    pub former: Option<ElementId>,
    /// The declared footprint.
    pub footprint: Footprint,
}

fn error(code: ErrorCode, field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(code, field, message)
}

impl Levels {
    /// Records a plate, beam or column. A repeated identity keeps the last kind.
    pub fn insert_holder(&mut self, id: ElementId, kind: HolderKind) {
        self.holders.insert(id, kind);
    }

    /// Records a level.
    pub fn insert_level(&mut self, id: ElementId, record: LevelRecord) {
        self.levels.insert(id, record);
    }

    /// Records an element of any other kind, so that its identity counts as used.
    pub fn insert_other(&mut self, id: ElementId) {
        self.others.insert(id);
    }

    /// Records the level of `holder`.
    pub fn set_level(&mut self, holder: ElementId, level: ElementId) {
        self.assigned.insert(holder, level);
    }

    /// Whether any element or level has this identity.
    pub fn contains(&self, id: &ElementId) -> bool {
        self.holders.contains_key(id) || self.levels.contains_key(id) || self.others.contains(id)
    }

    /// The kind of a plate, beam or column.
    pub fn holder(&self, id: &ElementId) -> Option<HolderKind> {
        self.holders.get(id).copied()
    }

    /// The stored fields of a level.
    pub fn level(&self, id: &ElementId) -> Option<&LevelRecord> {
        self.levels.get(id)
    }

    /// The level of an element.
    pub fn level_of(&self, holder: &ElementId) -> Option<ElementId> {
        self.assigned.get(holder).copied()
    }

    /// The elements assigned to `level`, in identity order.
    pub fn elements_of(&self, level: &ElementId) -> Vec<ElementId> {
        self.assigned
            .iter()
            .filter(|(_, l)| *l == level)
            .map(|(e, _)| *e)
            .collect()
    }

    /// Every level, in ascending identity order.
    pub fn levels(&self) -> impl Iterator<Item = (&ElementId, &LevelRecord)> + '_ {
        self.levels.iter()
    }

    /// Decides a level creation: the arguments are admitted and the identity must be unused
    /// (`DUPLICATE_ID`). Errors leave nothing applied.
    pub fn create(&self, command: &CreateLevel) -> Result<Creation, CommandError> {
        command.validate_arguments()?;
        if self.contains(&command.new_id) {
            return Err(error(
                ErrorCode::DuplicateId,
                "new_id",
                "the identity is already in use",
            ));
        }
        Ok(Creation {
            footprint: Footprint::for_create(command.new_id, None)?,
        })
    }

    /// Decides the creation of several levels in one operation: the request is expanded against
    /// the names the levels of this model use, and each level is decided as [`Levels::create`]
    /// would. Returns the footprint, their union, and the levels to create. Errors leave nothing
    /// applied.
    pub fn create_all(
        &self,
        command: &CreateLevels,
    ) -> Result<(Creation, Vec<CreateLevel>), CommandError> {
        let used = self.levels.values().map(|r| r.name.clone()).collect();
        let levels = command.levels(&used)?;
        let mut footprint = Footprint::default();
        for level in &levels {
            footprint = footprint.union(self.create(level)?.footprint)?;
        }
        Ok((Creation { footprint }, levels))
    }

    /// Decides a level update: the arguments are admitted, the target must be a level
    /// (`NOT_FOUND` for an unknown identity, `UNSUPPORTED_PROPERTY` for another kind) and at least
    /// one value must differ (`INVALID_ARGUMENT`), so an update never publishes an empty revision.
    /// Errors leave nothing applied.
    pub fn update(&self, command: &UpdateLevel) -> Result<LevelUpdate, CommandError> {
        command.validate_arguments()?;
        let Some(record) = self.levels.get(&command.target_id) else {
            return Err(if self.contains(&command.target_id) {
                error(
                    ErrorCode::UnsupportedProperty,
                    "target_id",
                    "the target is not a level",
                )
            } else {
                error(
                    ErrorCode::NotFound,
                    "target_id",
                    "the level is not in the head revision",
                )
            });
        };
        let name = command.name.clone().filter(|name| *name != record.name);
        let elevation_mm = command.elevation_mm.filter(|e| *e != record.elevation_mm);
        if name.is_none() && elevation_mm.is_none() {
            return Err(error(
                ErrorCode::InvalidArgument,
                "patch",
                "the level already has these values",
            ));
        }
        Ok(LevelUpdate {
            footprint: Footprint::for_level_update(
                command.target_id,
                name.is_some(),
                elevation_mm.is_some(),
            )?,
            name,
            elevation_mm,
        })
    }

    /// Decides the level assignment of `target`: `Some(level)` assigns, `None` removes the record.
    /// Errors leave nothing applied.
    pub fn assign(
        &self,
        target: ElementId,
        level: Option<ElementId>,
    ) -> Result<Assignment, CommandError> {
        if !self.contains(&target) {
            return Err(error(
                ErrorCode::NotFound,
                "target_id",
                "the element is not in the head revision",
            ));
        }
        if self.holder(&target).is_none() {
            return Err(error(
                ErrorCode::UnsupportedProperty,
                "level",
                "only a plate, beam or column is assigned to a level",
            ));
        }
        let former = self.level_of(&target);
        match level {
            Some(level) => {
                if !self.contains(&level) {
                    return Err(error(
                        ErrorCode::DanglingRelationship,
                        "level",
                        "the level is not in the head revision",
                    ));
                }
                if !self.levels.contains_key(&level) {
                    return Err(error(
                        ErrorCode::InvalidArgument,
                        "level",
                        "the target is not a level",
                    ));
                }
            }
            None if former.is_none() => {
                return Err(error(
                    ErrorCode::InvalidArgument,
                    "level",
                    "the element has no level to remove",
                ));
            }
            None => {}
        }
        Ok(Assignment {
            former,
            footprint: Footprint::for_level(target, level)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000004000{n:02x}")).unwrap()
    }

    fn model() -> Levels {
        let mut m = Levels::default();
        m.insert_level(
            id(1),
            LevelRecord {
                name: "L1".into(),
                elevation_mm: 0.0,
            },
        );
        m.insert_level(
            id(2),
            LevelRecord {
                name: "L2".into(),
                elevation_mm: 3000.0,
            },
        );
        m.insert_holder(id(10), HolderKind::Plate);
        m.insert_holder(id(11), HolderKind::Beam);
        m.insert_other(id(20));
        m
    }

    #[test]
    fn an_update_writes_only_the_fields_that_differ() {
        let m = model();
        let update = |target_id, name: Option<&str>, elevation_mm| UpdateLevel {
            target_id,
            name: name.map(str::to_owned),
            elevation_mm,
            move_elements: false,
        };
        let moved = m.update(&update(id(1), Some("L1"), Some(500.0))).unwrap();
        assert_eq!(moved.name, None);
        assert_eq!(moved.elevation_mm, Some(500.0));
        assert_eq!(
            moved.footprint,
            Footprint::for_level_update(id(1), false, true).unwrap()
        );
        let code = |c: UpdateLevel| m.update(&c).unwrap_err().code;
        assert_eq!(
            code(update(id(1), Some("L1"), Some(0.0))),
            ErrorCode::InvalidArgument
        );
        assert_eq!(code(update(id(1), None, None)), ErrorCode::InvalidArgument);
        assert_eq!(
            code(update(id(10), Some("x"), None)),
            ErrorCode::UnsupportedProperty
        );
        assert_eq!(code(update(id(99), Some("x"), None)), ErrorCode::NotFound);
    }

    fn create(n: u8, elevation_mm: f64) -> CreateLevel {
        CreateLevel {
            new_id: id(n),
            name: "L".into(),
            elevation_mm,
        }
    }

    #[test]
    fn a_level_is_created_with_a_free_identity_and_a_bounded_elevation() {
        let m = model();
        let created = m.create(&create(3, -3000.0)).unwrap();
        assert!(
            created
                .footprint
                .writes()
                .iter()
                .all(|e| e.target == crate::footprint::Target::Element(id(3)))
        );
        for taken in [1, 10, 20] {
            assert_eq!(
                m.create(&create(taken, 0.0)).unwrap_err().code,
                ErrorCode::DuplicateId
            );
        }
        assert_eq!(
            m.create(&create(3, f64::NAN)).unwrap_err().code,
            ErrorCode::NonFiniteNumber
        );
        assert_eq!(
            m.create(&create(3, 2e9)).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn several_levels_are_decided_together_and_one_taken_identity_refuses_all() {
        let m = model();
        let all = CreateLevels {
            new_ids: vec![id(3), id(4)],
            elevations: "0 3000".into(),
            names: String::new(),
        };
        let (created, levels) = m.create_all(&all).unwrap();
        let union = Footprint::for_create(id(3), None)
            .unwrap()
            .union(Footprint::for_create(id(4), None).unwrap())
            .unwrap();
        assert_eq!(created.footprint, union);
        // The model's levels are "L1" and "L2"; generated names start from "Level 1".
        assert_eq!(
            levels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["Level 1", "Level 2"]
        );
        let taken = CreateLevels {
            new_ids: vec![id(3), id(1)],
            ..all
        };
        assert_eq!(
            m.create_all(&taken).unwrap_err().code,
            ErrorCode::DuplicateId
        );
    }

    #[test]
    fn an_assignment_checks_the_element_kind_and_the_level() {
        let mut m = model();
        let set = m.assign(id(10), Some(id(1))).unwrap();
        assert_eq!(set.former, None);
        assert!(
            set.footprint
                .writes()
                .iter()
                .all(|e| e.target == crate::footprint::Target::Element(id(10)))
        );
        m.set_level(id(10), id(1));
        // Another level replaces the record: it is a single record, not a collection.
        assert_eq!(m.assign(id(10), Some(id(2))).unwrap().former, Some(id(1)));
        assert_eq!(m.assign(id(10), None).unwrap().former, Some(id(1)));
        let code = |r: Result<Assignment, CommandError>| r.unwrap_err().code;
        assert_eq!(code(m.assign(id(99), Some(id(1)))), ErrorCode::NotFound);
        assert_eq!(
            code(m.assign(id(20), Some(id(1)))),
            ErrorCode::UnsupportedProperty
        );
        assert_eq!(
            code(m.assign(id(1), Some(id(2)))),
            ErrorCode::UnsupportedProperty
        );
        assert_eq!(
            code(m.assign(id(10), Some(id(99)))),
            ErrorCode::DanglingRelationship
        );
        assert_eq!(
            code(m.assign(id(10), Some(id(11)))),
            ErrorCode::InvalidArgument
        );
        assert_eq!(code(m.assign(id(11), None)), ErrorCode::InvalidArgument);
    }

    #[test]
    fn levels_and_their_elements_list_in_identity_order() {
        let mut m = model();
        m.set_level(id(11), id(2));
        m.set_level(id(10), id(2));
        assert_eq!(m.elements_of(&id(2)), vec![id(10), id(11)]);
        assert!(m.elements_of(&id(1)).is_empty());
        let listed: Vec<_> = m.levels().map(|(i, r)| (*i, r.name.clone())).collect();
        assert_eq!(listed, vec![(id(1), "L1".into()), (id(2), "L2".into())]);
        assert_eq!(m.level(&id(2)).map(|r| r.elevation_mm), Some(3000.0));
        assert_eq!(m.level_of(&id(11)), Some(id(2)));
    }
}
