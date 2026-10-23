// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C03.1: bounded independence planning over verified command footprints.
//!
//! This module performs no merge, persistence, publication or authentication. Its caller must
//! verify ancestry and recover the actual engine-derived footprints of both operation intervals;
//! a state diff or a caller's narrower claim cannot substitute for those footprints. Missing
//! evidence must retain a conservative footprint or be refused by the application adapter.

use crate::footprint::{Entry, Footprint};
use beamcraft_model::changeset::MAX_OPERATIONS;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::OpId;
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use std::collections::{BTreeMap, BTreeSet};

/// Maximum complete conflict report. A larger report is refused, never silently truncated.
pub const MAX_REBASE_CONFLICTS: usize = 256;

/// One accepted command's identity and its verified semantic footprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    /// Full operation identity. Both intervals must contain distinct identities.
    pub id: OpId,
    /// Actual, nonempty engine-derived reads and writes, including indirect dependencies.
    pub footprint: Footprint,
}

/// One conflicting semantic entry between a draft and an intervening accepted command.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Conflict {
    /// Draft operation that must not be applied automatically.
    pub draft: OpId,
    /// Intervening accepted operation.
    pub accepted: OpId,
    /// Complete target and atomic semantic group.
    pub entry: Entry,
}

/// Pure independence decision. Neither variant changes a model or chooses a conflict winner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// No accepted write intersects a draft read/write, and no draft write intersects an
    /// accepted read. Identities retain draft order; they are not sorted by UUID.
    Independent {
        /// Ordered draft commands eligible for isolated re-evaluation by the adapter.
        operations: Vec<OpId>,
    },
    /// Complete, deterministic conflict report in draft-ID/accepted-ID/entry order.
    Conflicted {
        /// All conflicting pairs and groups, deduplicated across access directions.
        conflicts: Vec<Conflict>,
    },
}

fn limit() -> CommandError {
    CommandError::new(
        ErrorCode::LimitExceeded,
        "rebase",
        "rebase planning budget exceeded",
    )
}

/// Plans an explicit rebase of an ordered unsent interval against an ordered accepted interval.
///
/// Aggregate input is capped at 4096 operations and 4096 read/write entries before allocating
/// indexes. Conflict expansion is capped at 256 distinct pairs/groups. Every accepted operation
/// participates, including an edit later reverted: final-state equality cannot hide a dependency.
///
/// Empty footprints and duplicate operation IDs (also across intervals) are `INVALID_ARGUMENT`:
/// an empty footprint cannot stand for missing dependency evidence. The adapter must
/// resolve ledger replay/reuse and ancestry before invoking the planner. Read/read overlap is
/// permitted. No values are compared and equal final values never waive a write conflict.
pub fn plan(draft: &[Operation], accepted: &[Operation]) -> Result<Decision, CommandError> {
    if draft.len().saturating_add(accepted.len()) > MAX_OPERATIONS {
        return Err(limit());
    }
    let entries = draft.iter().chain(accepted).fold(0usize, |total, op| {
        total
            .saturating_add(op.footprint.reads().len())
            .saturating_add(op.footprint.writes().len())
    });
    if entries > MAX_FOOTPRINT_ENTRIES {
        return Err(limit());
    }
    let mut ids = BTreeSet::new();
    for op in draft.iter().chain(accepted) {
        if op.footprint.reads().is_empty() && op.footprint.writes().is_empty() {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "footprint",
                "rebase operation has no dependency evidence",
            ));
        }
        if !ids.insert(op.id) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "operations",
                "rebase intervals contain a duplicate operation identity",
            ));
        }
    }
    let mut readers: BTreeMap<Entry, BTreeSet<OpId>> = BTreeMap::new();
    let mut writers: BTreeMap<Entry, BTreeSet<OpId>> = BTreeMap::new();
    for op in accepted {
        for &entry in op.footprint.reads() {
            readers.entry(entry).or_default().insert(op.id);
        }
        for &entry in op.footprint.writes() {
            writers.entry(entry).or_default().insert(op.id);
        }
    }
    let mut conflicts = BTreeSet::new();
    for op in draft {
        for &entry in op.footprint.reads() {
            record(&mut conflicts, op.id, entry, writers.get(&entry))?;
        }
        for &entry in op.footprint.writes() {
            record(&mut conflicts, op.id, entry, readers.get(&entry))?;
            record(&mut conflicts, op.id, entry, writers.get(&entry))?;
        }
    }
    if conflicts.is_empty() {
        Ok(Decision::Independent {
            operations: draft.iter().map(|op| op.id).collect(),
        })
    } else {
        Ok(Decision::Conflicted {
            conflicts: conflicts.into_iter().collect(),
        })
    }
}

fn record(
    conflicts: &mut BTreeSet<Conflict>,
    draft: OpId,
    entry: Entry,
    accepted: Option<&BTreeSet<OpId>>,
) -> Result<(), CommandError> {
    if let Some(accepted) = accepted {
        for &accepted in accepted {
            let conflict = Conflict {
                draft,
                accepted,
                entry,
            };
            if conflicts.len() == MAX_REBASE_CONFLICTS && !conflicts.contains(&conflict) {
                return Err(limit());
            }
            conflicts.insert(conflict);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprint::{Group, Target};
    use beamcraft_model::digest::Sha256;
    use beamcraft_model::id::ElementId;

    fn element(n: usize) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
    }
    fn operation(n: usize, footprint: Footprint) -> Operation {
        Operation {
            id: OpId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap(),
            footprint,
        }
    }
    fn rename(n: usize) -> Footprint {
        Footprint::for_update(element(n), true, false, false, false, None).unwrap()
    }
    fn conflicts(decision: Decision) -> Vec<Conflict> {
        match decision {
            Decision::Conflicted { conflicts } => conflicts,
            other => panic!("expected conflicts, got {other:?}"),
        }
    }

    #[test]
    fn independent_renames_retain_the_draft_order_and_read_overlap_is_allowed() {
        let draft = [operation(20, rename(2)), operation(10, rename(3))];
        let accepted = [operation(1, rename(1))];
        assert_eq!(
            plan(&draft, &accepted).unwrap(),
            Decision::Independent {
                operations: vec![draft[0].id, draft[1].id]
            }
        );
        let read_only = operation(
            30,
            Footprint::for_update(element(1), false, false, false, false, None).unwrap(),
        );
        assert!(matches!(
            plan(&[read_only], &accepted).unwrap(),
            Decision::Independent { .. }
        ));
        assert_eq!(
            plan(&[], &accepted).unwrap(),
            Decision::Independent { operations: vec![] }
        );
    }

    #[test]
    fn a_move_and_a_rename_of_one_part_use_disjoint_semantic_groups() {
        let draft = [operation(1, rename(1))];
        let accepted = [operation(
            2,
            Footprint::for_update(element(1), false, false, false, true, None).unwrap(),
        )];
        assert!(matches!(
            plan(&draft, &accepted).unwrap(),
            Decision::Independent { .. }
        ));
        assert!(matches!(
            plan(&accepted, &draft).unwrap(),
            Decision::Independent { .. }
        ));
    }

    #[test]
    fn a_write_read_write_collision_is_reported_once_per_group() {
        let draft = [operation(1, rename(1))];
        let accepted = [operation(2, rename(1))];
        let entries = conflicts(plan(&draft, &accepted).unwrap());
        assert_eq!(
            entries,
            vec![Conflict {
                draft: draft[0].id,
                accepted: accepted[0].id,
                entry: Entry {
                    target: Target::Element(element(1)),
                    group: Group::Label
                }
            }]
        );
    }

    #[test]
    fn shape_inputs_are_atomic_and_every_intervening_operation_participates() {
        let footprint = Footprint::for_update(element(1), false, false, true, false, None).unwrap();
        let draft = [operation(1, footprint.clone())];
        // These may be an edit followed by its inverse; the planner must retain both dependencies.
        let accepted = [operation(3, footprint.clone()), operation(2, footprint)];
        let found = conflicts(plan(&draft, &accepted).unwrap());
        assert_eq!(found.len(), 4);
        assert_eq!(
            found.iter().map(|c| c.accepted).collect::<BTreeSet<_>>(),
            accepted.iter().map(|o| o.id).collect()
        );
        assert!(
            found
                .iter()
                .all(|c| matches!(c.entry.group, Group::ShapeInputs | Group::Derived))
        );
        let reverse = [accepted[1].clone(), accepted[0].clone()];
        assert_eq!(
            plan(&draft, &accepted).unwrap(),
            plan(&draft, &reverse).unwrap()
        );
    }

    #[test]
    fn unchanged_copy_sources_and_linked_part_lifecycles_still_conflict() {
        let draft = [operation(
            1,
            Footprint::for_copy(element(1), element(2), None, None).unwrap(),
        )];
        let accepted = [operation(2, rename(1))];
        assert!(
            conflicts(plan(&draft, &accepted).unwrap())
                .iter()
                .any(|c| c.entry.target == Target::Element(element(1))
                    && c.entry.group == Group::Label)
        );
        let weld = [operation(
            3,
            Footprint::for_weld(element(3), [element(1), element(2)]).unwrap(),
        )];
        let delete = [operation(4, Footprint::for_delete(element(1)).unwrap())];
        assert_eq!(conflicts(plan(&weld, &delete).unwrap()).len(), 1);
        assert_eq!(conflicts(plan(&delete, &weld).unwrap()).len(), 1);
    }

    #[test]
    fn immutable_definition_reads_are_in_the_conflict_index() {
        let digest = Sha256::digest(b"synthetic profile").unwrap();
        let draft = [operation(
            1,
            Footprint::for_create_member(element(1), digest, None).unwrap(),
        )];
        let accepted = [operation(2, Footprint::for_define(digest))];
        let found = conflicts(plan(&draft, &accepted).unwrap());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].entry.target, Target::Definition(digest));
    }

    #[test]
    fn duplicate_operation_identities_are_refused_without_deciding_replay() {
        let op = operation(1, rename(1));
        assert_eq!(
            plan(&[op.clone(), op.clone()], &[]).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            plan(std::slice::from_ref(&op), std::slice::from_ref(&op))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn conflict_expansion_is_complete_at_the_cap_and_refused_one_above() {
        let draft = [operation(1000, rename(1))];
        let mut accepted: Vec<_> = (1..=MAX_REBASE_CONFLICTS)
            .map(|n| operation(n, rename(1)))
            .collect();
        assert_eq!(
            conflicts(plan(&draft, &accepted).unwrap()).len(),
            MAX_REBASE_CONFLICTS
        );
        accepted.push(operation(MAX_REBASE_CONFLICTS + 1, rename(1)));
        assert_eq!(
            plan(&draft, &accepted).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn aggregate_input_budgets_are_checked_before_indexing() {
        let mut operations: Vec<_> = (0..MAX_OPERATIONS)
            .map(|n| {
                operation(
                    n,
                    Footprint::for_update(element(1), false, false, false, false, None).unwrap(),
                )
            })
            .collect();
        assert!(matches!(
            plan(&operations, &[]).unwrap(),
            Decision::Independent { .. }
        ));
        operations.push(operation(MAX_OPERATIONS, Footprint::default()));
        assert_eq!(
            plan(&operations, &[]).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        let creates: Vec<_> = (0..MAX_FOOTPRINT_ENTRIES / 8 + 1)
            .map(|n| operation(n, Footprint::for_create(element(n), None).unwrap()))
            .collect();
        assert!(matches!(
            plan(&creates[..creates.len() - 1], &[]).unwrap(),
            Decision::Independent { .. }
        ));
        assert_eq!(
            plan(&creates, &[]).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn missing_dependency_evidence_never_means_independent() {
        let missing = operation(1, Footprint::default());
        assert_eq!(
            plan(&[missing], &[]).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
}
