// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The in-memory draft batch behind the CLI session (`I424`): commands between `batch-open`
//! and `batch-commit` admit their prepared layer sets here and commit nothing. The draft
//! holds the base revision pinned at `batch-open`, the accumulated layer set and one
//! recorded request per admitted command; the batch's single commit publishes the final
//! layer set as one revision, and a refusal abandons the draft whole. Nothing here touches
//! the disk: an interrupted batch leaves no trace, so recovery has nothing new to recover.

use crate::Refusal;
use crate::pipeline::{Base, digest_of, integrity};
use beamcraft_engine::footprint::Footprint;
use beamcraft_host::revision::{BaseRef, ExpectedBase};
use beamcraft_layer_engine::Asset;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId};
use std::cell::RefCell;
use std::collections::BTreeSet;

/// What one admitted command's preview claims: the generation and sequence of the revision it
/// would create, and the pinned base that revision would carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    /// Workspace generation of the drafted revision.
    pub generation: u64,
    /// Head-log sequence the drafted revision would take.
    pub sequence: u64,
    /// The pinned base as a manifest reference; `None` for a first-revision store.
    pub base: Option<BaseRef>,
}

/// One command admitted into the open draft: its operation id (the per-command replay key),
/// the digest of its exact request, and the semantic footprint its admission declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftCommand {
    /// Full operation identity of the command.
    pub op: OpId,
    /// Digest of the admitted request; a different request under this id is `OP_ID_REUSE`.
    pub fingerprint: ContentDigest,
    /// The footprint the command's admission declared.
    pub footprint: Footprint,
}

/// The state of one open draft batch.
#[derive(Debug, Clone)]
pub struct Draft {
    /// The verified base the batch pinned at `batch-open`; its `expected` is what the batch's
    /// commit must still find at the root, and its identity fields are the batch revision's.
    pub base: Base,
    /// Head-log sequence of the pinned base; the preview of the first admitted command
    /// claims the sequence after it.
    pub base_sequence: u64,
    /// The accumulated layer set: the base's layers with every admitted command's prepared
    /// output (owned layers plus retained foreign sources) applied.
    pub layers: Vec<Asset>,
    /// The workspace generation the next admitted command's stage runs at: the base's
    /// generation plus the number of admitted commands.
    pub generation: u64,
    /// The admitted commands, in batch order.
    pub commands: Vec<DraftCommand>,
    /// Union of the admitted commands' footprints; the batch's commit pins it as evidence
    /// and refuses beyond the footprint budget (`4096` entries, never truncated).
    pub union: Footprint,
}

impl Draft {
    /// Opens a draft over a verified base.
    pub fn open(base: Base, base_sequence: u64) -> Self {
        Self {
            layers: base.assets.clone(),
            base_sequence,
            generation: base.generation,
            commands: Vec::new(),
            union: Footprint::default(),
            base,
        }
    }

    /// The identity digest of the current draft state: a digest over the ordered layer set,
    /// stable across runs of the same command list, so a drafted stage is reproducible.
    pub fn virtual_pin(&self) -> Result<ContentDigest, Refusal> {
        let mut text = Vec::new();
        text.extend_from_slice(b"beamcraft-draft-base 1");
        for asset in &self.layers {
            text.extend_from_slice(asset.name.as_bytes());
            text.extend_from_slice(asset.digest.as_bytes());
        }
        digest_of(&text)
    }

    /// The base a command inside the batch authors over: the draft's accumulated layers, at
    /// the draft's generation, expected to replace the pinned base.
    pub fn base_for(&self, claim: Option<ProjectId>) -> Result<Base, Refusal> {
        if claim.is_some_and(|c| c.to_string() != self.base.project) {
            return Err(Refusal::new(
                ErrorCode::InvalidArgument,
                "the project id differs from the project recorded in the store",
            ));
        }
        Ok(Base {
            project: self.base.project.clone(),
            workspace: self.base.workspace.clone(),
            digest: self.virtual_pin()?,
            generation: self.generation,
            assets: self.layers.clone(),
            external: self.external(),
            expected: self.base.expected.clone(),
        })
    }

    /// The foreign layer names the draft carries, from the pinned base.
    pub fn external(&self) -> BTreeSet<String> {
        self.base.external.clone()
    }

    /// Admits one prepared command output into the draft: records the request, applies the
    /// prepared layer set and returns the `(generation, sequence)` a preview of this command
    /// should claim. A duplicate operation id inside one batch is refused (the identical
    /// request replays only as a whole, by the batch's own operation id); a union footprint
    /// beyond the budget is refused, never truncated.
    pub fn admit(
        &mut self,
        layers: &[Asset],
        retained: &[Asset],
        op: &str,
        fingerprint: ContentDigest,
        footprint: &Footprint,
    ) -> Result<Admitted, Refusal> {
        let op_id = OpId::parse(op).map_err(|_| integrity("operation identity is invalid"))?;
        if self.commands.iter().any(|recorded| recorded.op == op_id) {
            return Err(Refusal::new(
                ErrorCode::OpIdReuse,
                "the operation id was already admitted into this batch",
            ));
        }
        let union = self
            .union
            .clone()
            .union(footprint.clone())
            .map_err(Refusal::from)?;
        self.union = union;
        self.commands.push(DraftCommand {
            op: op_id,
            fingerprint,
            footprint: footprint.clone(),
        });
        self.layers = layers.iter().chain(retained.iter()).cloned().collect();
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Refusal::new(ErrorCode::LimitExceeded, "generation exhausted"))?;
        let sequence = self
            .base_sequence
            .checked_add(
                u64::try_from(self.commands.len())
                    .map_err(|_| Refusal::new(ErrorCode::LimitExceeded, "batch is too long"))?,
            )
            .ok_or_else(|| Refusal::new(ErrorCode::LimitExceeded, "sequence exhausted"))?;
        let base = match &self.base.expected {
            ExpectedBase::Revision(id) => Some(BaseRef {
                revision: id.clone(),
                manifest: self.base.digest,
            }),
            ExpectedBase::Initial => None,
        };
        Ok(Admitted {
            generation: self.generation,
            sequence,
            base,
        })
    }
}

thread_local! {
    static OPEN: RefCell<Option<Draft>> = const { RefCell::new(None) };
}

/// Held while a draft batch is open on the current thread. There is never more than one:
/// `hold` refuses while a batch is open, and dropping the guard abandons whatever is left.
#[must_use = "the batch ends when the guard is dropped"]
pub struct BatchGuard {
    _private: (),
}

impl Drop for BatchGuard {
    fn drop(&mut self) {
        OPEN.with(|slot| *slot.borrow_mut() = None);
    }
}

/// Opens a draft batch on the current thread; refuses while one is already open.
pub fn hold(base: Base, base_sequence: u64) -> Result<BatchGuard, Refusal> {
    OPEN.with(|slot| {
        let mut open = slot.borrow_mut();
        if open.is_some() {
            return Err(Refusal::new(
                ErrorCode::InvalidArgument,
                "a batch is already open",
            ));
        }
        *open = Some(Draft::open(base, base_sequence));
        Ok(BatchGuard { _private: () })
    })
}

/// True while a draft batch is open: publication admits into it instead of committing.
pub fn is_open() -> bool {
    OPEN.with(|slot| slot.borrow().is_some())
}

/// Runs `f` with the open draft; `None` when no batch is open.
pub fn with<R>(f: impl FnOnce(&mut Draft) -> R) -> Option<R> {
    OPEN.with(|slot| {
        let mut open = slot.borrow_mut();
        open.as_mut().map(f)
    })
}

/// Ends the batch and returns its state; both the commit and an abandonment end it this way.
/// The guard, if still held, keeps clearing an already-empty slot when it drops.
pub fn take() -> Option<Draft> {
    OPEN.with(|slot| slot.borrow_mut().take())
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_engine::footprint::{Entry, Group, Target};
    use beamcraft_host::revision::ExpectedBase;
    use beamcraft_model::id::ElementId;

    fn base() -> Base {
        let asset = |name: &str, text: &str| Asset {
            name: name.into(),
            digest: digest_of(text.as_bytes()).unwrap(),
            bytes: text.as_bytes().to_vec(),
        };
        Base {
            project: "01900000-0000-7000-8000-000000000001".into(),
            workspace: "01900000-0000-7000-8000-000000000002".into(),
            digest: digest_of(b"test-base").unwrap(),
            generation: 4,
            assets: vec![
                asset("authored.usda", "#usda 1.0\n"),
                asset("derived.usda", "#usda 1.0\n"),
            ],
            external: BTreeSet::new(),
            expected: ExpectedBase::Initial,
        }
    }

    fn footprint(element: u128) -> Footprint {
        let id = ElementId::parse(&format!("01900000-0000-7000-8000-{element:012x}")).unwrap();
        let entry = Entry {
            target: Target::Element(id),
            group: Group::Placement,
        };
        Footprint::from_entries(&[entry.clone()], &[entry]).unwrap()
    }

    fn prepared(generation: u32) -> (Vec<Asset>, Vec<Asset>) {
        let asset = |name: &str, text: String| Asset {
            name: name.into(),
            digest: digest_of(text.as_bytes()).unwrap(),
            bytes: text.into_bytes(),
        };
        (
            vec![asset(
                "authored.usda",
                format!("#usda 1.0\n# generation {generation}\n"),
            )],
            Vec::new(),
        )
    }

    #[test]
    fn hold_refuses_a_second_batch_and_take_ends_it() {
        let guard = hold(base(), 7).unwrap();
        assert!(is_open());
        assert!(hold(base(), 7).is_err());
        let draft = take().unwrap();
        assert_eq!(draft.base_sequence, 7);
        assert_eq!(draft.generation, 4);
        drop(guard);
        assert!(!is_open());
        assert!(take().is_none());
    }

    #[test]
    fn dropping_the_guard_abandons_the_draft() {
        let guard = hold(base(), 1).unwrap();
        assert!(is_open());
        drop(guard);
        assert!(!is_open());
        assert!(take().is_none());
    }

    #[test]
    fn admit_accumulates_layers_and_previews_advance() {
        let guard = hold(base(), 3).unwrap();
        let (layers, retained) = prepared(1);
        let admitted = with(|draft| {
            draft.admit(
                &layers,
                &retained,
                "01900000-0000-7000-8000-00000000000a",
                digest_of(b"r1").unwrap(),
                &footprint(1),
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(admitted.generation, 5);
        assert_eq!(admitted.sequence, 4);
        assert!(admitted.base.is_none());
        with(|draft| {
            assert_eq!(draft.layers.len(), 1);
            assert_eq!(draft.commands.len(), 1);
            assert_eq!(draft.union.reads().len(), 1);
        })
        .unwrap();
        let (layers, retained) = prepared(2);
        with(|draft| {
            draft
                .admit(
                    &layers,
                    &retained,
                    "01900000-0000-7000-8000-00000000000b",
                    digest_of(b"r2").unwrap(),
                    &footprint(1),
                )
                .unwrap();
            assert_eq!(draft.commands.len(), 2);
            // The same element read and written twice unions to one entry.
            assert_eq!(draft.union.reads().len(), 1);
            assert_eq!(draft.generation, 6);
        })
        .unwrap();
        drop(guard);
    }

    #[test]
    fn a_duplicate_operation_id_inside_one_batch_is_refused() {
        let guard = hold(base(), 1).unwrap();
        let (layers, retained) = prepared(1);
        let op = "01900000-0000-7000-8000-00000000000a";
        with(|draft| {
            draft
                .admit(
                    &layers,
                    &retained,
                    op,
                    digest_of(b"r1").unwrap(),
                    &footprint(1),
                )
                .unwrap();
            let refused = draft.admit(
                &layers,
                &retained,
                op,
                digest_of(b"r1").unwrap(),
                &footprint(1),
            );
            assert_eq!(refused.unwrap_err().code, ErrorCode::OpIdReuse);
        })
        .unwrap();
        drop(guard);
    }

    #[test]
    fn the_virtual_pin_follows_the_layer_set() {
        let guard = hold(base(), 1).unwrap();
        let pin_before = with(|draft| draft.virtual_pin()).unwrap().unwrap();
        let (layers, retained) = prepared(1);
        with(|draft| {
            draft
                .admit(
                    &layers,
                    &retained,
                    "01900000-0000-7000-8000-00000000000a",
                    digest_of(b"r1").unwrap(),
                    &footprint(1),
                )
                .unwrap();
        })
        .unwrap();
        let pin_after = with(|draft| draft.virtual_pin()).unwrap().unwrap();
        assert_ne!(pin_before, pin_after);
        drop(guard);
    }

    #[test]
    fn base_for_carries_the_draft_state_and_refuses_a_foreign_claim() {
        let guard = hold(base(), 1).unwrap();
        let claimed = ProjectId::parse("01900000-0000-7000-8000-000000000001").unwrap();
        let other = ProjectId::parse("01900000-0000-7000-8000-000000000009").unwrap();
        let (layers, retained) = prepared(1);
        with(|draft| {
            draft
                .admit(
                    &layers,
                    &retained,
                    "01900000-0000-7000-8000-00000000000a",
                    digest_of(b"r1").unwrap(),
                    &footprint(1),
                )
                .unwrap();
            let base = draft.base_for(None).unwrap();
            assert_eq!(base.generation, 5);
            assert_eq!(base.assets.len(), 1);
            assert_eq!(base.project, claimed.to_string());
            assert_eq!(
                draft.base_for(Some(claimed)).unwrap().project,
                claimed.to_string()
            );
            let refused = draft.base_for(Some(other));
            assert_eq!(refused.unwrap_err().code, ErrorCode::InvalidArgument);
        })
        .unwrap();
        drop(guard);
    }
}
