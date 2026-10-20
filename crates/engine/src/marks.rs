// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A09 approved element marks: the engine's decisions on `AssignMark` and `ApproveMark` over
//! the marks of a model. The A08 numbering preview ([`crate::numbering`]) proposes series and
//! stores nothing; this half stores and approves what a person assigns.
//!
//! A mark is a human-authored string on a fabricable element (`plate`, `beam`, `column`,
//! `assembly`, `connection`); one value per element, one namespace per model. `assigned` is a
//! chosen value; `approved` freezes it. The value is immutable while the approval stands, and a
//! shape-input edit withdraws the approval (the value is kept, the status falls back to
//! `assigned`) — the issue's "re-approve" branch; the revoke command is the explicit door back.
//!
//! The engine never touches USD and never generates a value. The adapter reads the marks and the
//! markable kinds of the pinned model into a [`Numbering`]; the decision here is pure, bounded
//! and deterministic. The layer engine re-checks the same rules on the authored candidate, so a
//! defect in either place fails closed (docs/design/a09-marks.md).

use crate::footprint::Footprint;
use beamcraft_model::command::{AssignMark, CommandError, MarkStatus};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::{BTreeMap, BTreeSet};

/// The mark of an element and its approval state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// The value outputs reference.
    pub value: String,
    /// Whether a person has approved the value.
    pub status: MarkStatus,
}

/// The kind of a markable element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkableKind {
    /// A plate, beam or column.
    Part,
    /// An assembly.
    Assembly,
    /// A weld (a `connection` element).
    Weld,
}

/// The marks and markable kinds of a model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Numbering {
    marks: BTreeMap<ElementId, Mark>,
    markable: BTreeMap<ElementId, MarkableKind>,
    others: BTreeSet<ElementId>,
}

/// An accepted mark assignment, replacement, revocation or approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// The element whose mark changed.
    pub target: ElementId,
    /// The footprint of the change.
    pub footprint: Footprint,
}

fn error(code: ErrorCode, field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(code, field, message)
}

impl Numbering {
    /// Records the mark of an element.
    pub fn insert_mark(&mut self, id: ElementId, mark: Mark) {
        self.marks.insert(id, mark);
    }

    /// Records a markable element. A repeated identity keeps the last kind.
    pub fn insert_markable(&mut self, id: ElementId, kind: MarkableKind) {
        self.markable.insert(id, kind);
    }

    /// Records an element that cannot carry a mark, so its identity counts as used.
    pub fn insert_other(&mut self, id: ElementId) {
        self.others.insert(id);
    }

    /// Whether any element has this identity.
    pub fn contains(&self, id: &ElementId) -> bool {
        self.marks.contains_key(id) || self.markable.contains_key(id) || self.others.contains(id)
    }

    /// The mark of an element.
    pub fn mark(&self, id: &ElementId) -> Option<&Mark> {
        self.marks.get(id)
    }

    /// Every mark, in ascending identity order.
    pub fn all(&self) -> impl Iterator<Item = (&ElementId, &Mark)> + '_ {
        self.marks.iter()
    }

    /// Whether a shape-input edit of `id` withdraws an approval: only an `approved` mark is
    /// withdrawn, and the value is kept. Called by the update path before authoring the batch.
    pub fn shape_edit_withdraws(&self, id: &ElementId) -> bool {
        self.marks
            .get(id)
            .is_some_and(|m| m.status == MarkStatus::Approved)
    }

    /// Decides `AssignMark`: a value assigns or replaces the mark of an unapproved element;
    /// `None` revokes the approval of an approved one. Errors leave nothing applied.
    pub fn assign(&self, command: &AssignMark) -> Result<Decision, CommandError> {
        command.validate_arguments()?;
        let target = command.target_id;
        if !self.contains(&target) {
            return Err(error(
                ErrorCode::NotFound,
                "target_id",
                "the element is not in the head revision",
            ));
        }
        if !self.markable.contains_key(&target) {
            return Err(error(
                ErrorCode::UnsupportedProperty,
                "target_id",
                "only a plate, beam, column, assembly or weld carries a mark",
            ));
        }
        let current = self.marks.get(&target);
        match (&command.mark, current) {
            // A revoke is the explicit door back from an approval; nothing else lifts one.
            (
                None,
                Some(Mark {
                    status: MarkStatus::Approved,
                    ..
                }),
            ) => {}
            (None, _) => {
                return Err(error(
                    ErrorCode::InvalidArgument,
                    "mark",
                    "the element has no approved mark to revoke",
                ));
            }
            (Some(value), Some(current)) if current.status == MarkStatus::Approved => {
                if *value == current.value {
                    return Err(error(
                        ErrorCode::InvalidArgument,
                        "mark",
                        "the element already carries this mark",
                    ));
                }
                return Err(error(
                    ErrorCode::DependencyConflict,
                    "mark",
                    "the mark is approved and cannot change; revoke the approval first",
                ));
            }
            (Some(value), Some(current)) if *value == current.value => {
                return Err(error(
                    ErrorCode::InvalidArgument,
                    "mark",
                    "the element already carries this mark",
                ));
            }
            (Some(value), _) => {
                if self.marks.values().any(|m| m.value == *value) {
                    return Err(error(
                        ErrorCode::DuplicateId,
                        "mark",
                        "the mark is already in use",
                    ));
                }
            }
        }
        Ok(Decision {
            target,
            footprint: Footprint::for_mark(target)?,
        })
    }

    /// Decides `ApproveMark`: the element must exist, carry a mark and not be approved yet.
    /// Errors leave nothing applied.
    pub fn approve(&self, target: ElementId) -> Result<Decision, CommandError> {
        if !self.contains(&target) {
            return Err(error(
                ErrorCode::NotFound,
                "target_id",
                "the element is not in the head revision",
            ));
        }
        match self.marks.get(&target) {
            None => Err(error(
                ErrorCode::InvalidArgument,
                "target_id",
                "the element carries no mark",
            )),
            Some(Mark {
                status: MarkStatus::Approved,
                ..
            }) => Err(error(
                ErrorCode::InvalidArgument,
                "target_id",
                "the mark is already approved",
            )),
            Some(_) => Ok(Decision {
                target,
                footprint: Footprint::for_mark(target)?,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000004600{n:02x}")).unwrap()
    }

    fn assign(target: u8, mark: Option<&str>) -> AssignMark {
        AssignMark {
            target_id: id(target),
            mark: mark.map(str::to_owned),
        }
    }

    fn model() -> Numbering {
        let mut m = Numbering::default();
        m.insert_markable(id(1), MarkableKind::Part);
        m.insert_markable(id(2), MarkableKind::Part);
        m.insert_markable(id(3), MarkableKind::Assembly);
        m.insert_markable(id(4), MarkableKind::Weld);
        m.insert_other(id(8));
        m.insert_mark(
            id(5),
            Mark {
                value: "P/1".into(),
                status: MarkStatus::Assigned,
            },
        );
        m.insert_markable(id(5), MarkableKind::Part);
        m.insert_mark(
            id(6),
            Mark {
                value: "A/1".into(),
                status: MarkStatus::Approved,
            },
        );
        m.insert_markable(id(6), MarkableKind::Assembly);
        m
    }

    #[test]
    fn a_mark_assigns_replaces_and_stays_unique() {
        let m = model();
        let code = |r: Result<Decision, CommandError>| r.unwrap_err().code;
        assert_eq!(m.assign(&assign(1, Some("B/1"))).unwrap().target, id(1));
        // A value another element already carries is a duplicate, whatever the kind.
        assert_eq!(
            code(m.assign(&assign(2, Some("P/1")))),
            ErrorCode::DuplicateId
        );
        assert_eq!(
            code(m.assign(&assign(2, Some("A/1")))),
            ErrorCode::DuplicateId
        );
        assert_eq!(
            code(m.assign(&assign(4, Some("A/1")))),
            ErrorCode::DuplicateId
        );
        // A replacement on an assigned mark is free; on an approved mark it is refused.
        assert_eq!(m.assign(&assign(5, Some("P/2"))).unwrap().target, id(5));
        assert_eq!(
            code(m.assign(&assign(6, Some("A/2")))),
            ErrorCode::DependencyConflict
        );
        assert_eq!(
            code(m.assign(&assign(6, Some("A/1")))),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn only_markable_elements_carry_marks() {
        let m = model();
        let code = |r: Result<Decision, CommandError>| r.unwrap_err().code;
        assert_eq!(code(m.assign(&assign(9, Some("X")))), ErrorCode::NotFound);
        assert_eq!(
            code(m.assign(&assign(8, Some("X")))),
            ErrorCode::UnsupportedProperty
        );
        assert_eq!(code(m.approve(id(9))), ErrorCode::NotFound);
    }

    #[test]
    fn an_approval_freezes_the_value_and_revoke_is_the_door_back() {
        let m = model();
        assert_eq!(m.approve(id(5)).unwrap().target, id(5));
        assert_eq!(
            m.approve(id(6)).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            m.approve(id(1)).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let code = |r: Result<Decision, CommandError>| r.unwrap_err().code;
        assert_eq!(
            code(m.assign(&AssignMark {
                target_id: id(5),
                mark: None
            })),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            m.assign(&AssignMark {
                target_id: id(6),
                mark: None
            })
            .unwrap()
            .target,
            id(6)
        );
    }

    #[test]
    fn a_shape_edit_withdraws_only_an_approved_mark() {
        let m = model();
        assert!(!m.shape_edit_withdraws(&id(1)));
        assert!(!m.shape_edit_withdraws(&id(5)));
        assert!(m.shape_edit_withdraws(&id(6)));
    }

    #[test]
    fn every_decision_writes_only_the_target_mark() {
        let m = model();
        for d in [
            m.assign(&assign(1, Some("B/1"))).unwrap(),
            m.approve(id(5)).unwrap(),
            m.assign(&AssignMark {
                target_id: id(6),
                mark: None,
            })
            .unwrap(),
        ] {
            let writes: Vec<_> = d.footprint.writes().iter().collect();
            assert_eq!(writes.len(), 1);
            assert_eq!(
                writes[0].target,
                crate::footprint::Target::Element(d.target)
            );
            assert_eq!(writes[0].group, crate::footprint::Group::Mark);
        }
        let listed: Vec<_> = m.all().map(|(i, mark)| (*i, mark.value.clone())).collect();
        assert_eq!(listed, vec![(id(5), "P/1".into()), (id(6), "A/1".into())]);
    }
}
