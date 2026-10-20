// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A09 marks adapter: `AssignMark` and `ApproveMark`, committed as the next revision, plus a
//! read of the marks of the head. The A08 numbering preview ([`crate::numbering`]) proposes
//! series and writes nothing; this adapter stores and approves what a person assigns. See
//! [`crate::edit`] for the withdrawal of an approval under a shape-input edit and
//! `docs/design/a09-marks.md` for the state machine.
//!
//! A mark is a human-authored string on a plate, beam, column, assembly or weld with its
//! approval state: `assigned` is chosen, `approved` freezes the value. The record lives with its
//! source as `bim:mark` and `bim:markStatus` on the element prim, so only the marked element is
//! written. A copy does not inherit the mark; deleting the element removes it.
//!
//! Replay of an `op_id` is decided from the revision the host ledger recorded for it, as in
//! `weld`: the retry is a replay only when that revision left the target in exactly the
//! requested mark state, whatever the head holds now.

use crate::assembly::read_prims;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, PublishRequest, RevisionSummary, base_of,
    check_footprint, control, digest_of, host_uuid, integrity, load_base, open_base, prior_op_with,
    property, publish_revision, replay_summary, summarize, text,
};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::marks::{Mark, MarkableKind, Numbering};
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, MarkStatus, Prepare, schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Prim, Reader};
use beamcraft_model::command::{ApproveMark, AssignMark, CommandError};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

/// Identities of one A08 call beyond the command itself.
#[derive(Debug, Clone)]
pub struct NumberingContext {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
}

/// One marked element of the head.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkView {
    /// Identity.
    pub id: ElementId,
    /// The element kind, as the vocabulary token (`weld` for a connection).
    pub kind: String,
    /// Label.
    pub name: String,
    /// The mark value.
    pub mark: String,
    /// Whether a person has approved the value.
    pub status: MarkStatus,
}

/// Result of one A08 call.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkChange {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The marks of the resulting head.
    pub marks: Vec<MarkView>,
}

/// The markable kinds, the other identities and the marks of a pinned set of prims. Elements of
/// foreign or unknown kinds count as used identities and nothing else.
pub(crate) fn marks_of(prims: &[Prim]) -> Result<Numbering, PlateError> {
    let by_path: BTreeMap<&str, ElementId> = prims
        .iter()
        .filter_map(|p| {
            let id = ElementId::parse(p.element_id.as_deref()?).ok()?;
            Some((p.path.as_str(), id))
        })
        .collect();
    let mut numbering = Numbering::default();
    for prim in prims {
        let Some(id) = by_path.get(prim.path.as_str()).copied() else {
            continue;
        };
        // Only owned 0.1.0 parametric elements are interpreted, as the structural validator does;
        // newer, foreign or other-mode content stays inert and counts as a used identity only.
        let supported = !prim.sources.is_empty()
            && prim.sources.iter().all(|s| s.owned)
            && property(prim, "bim:schemaVersion").and_then(text) == Some("0.1.0")
            && property(prim, "bim:geometryMode").and_then(text) == Some("parametric");
        let markable = match property(prim, "bim:elementKind")
            .and_then(text)
            .filter(|_| supported)
        {
            Some("plate" | "beam" | "column") => MarkableKind::Part,
            Some("assembly") => MarkableKind::Assembly,
            Some("connection") => MarkableKind::Weld,
            _ => {
                numbering.insert_other(id);
                continue;
            }
        };
        numbering.insert_markable(id, markable);
        let value = property(prim, "bim:mark").and_then(text);
        let status = property(prim, "bim:markStatus").and_then(text);
        match (value, status) {
            (None, None) => {}
            (Some(value), Some(status)) => {
                let status = MarkStatus::parse(status)
                    .ok_or_else(|| integrity("mark status is not readable"))?;
                numbering.insert_mark(
                    id,
                    Mark {
                        value: value.to_owned(),
                        status,
                    },
                );
            }
            // The structural validator refuses a half-authored record before it is read here.
            _ => return Err(integrity("mark and status are not authored together")),
        }
    }
    Ok(numbering)
}

/// The kind token and label of every element of a pinned set of prims, for the `marks` list.
fn names_and_kinds(prims: &[Prim]) -> (BTreeMap<ElementId, String>, BTreeMap<ElementId, String>) {
    let mut kinds = BTreeMap::new();
    let mut names = BTreeMap::new();
    for prim in prims {
        let Some(id) = prim
            .element_id
            .as_deref()
            .and_then(|s| ElementId::parse(s).ok())
        else {
            continue;
        };
        let kind = match property(prim, "bim:elementKind").and_then(text) {
            Some(kind @ ("plate" | "beam" | "column" | "assembly" | "level" | "reference")) => kind,
            Some("connection") => "weld",
            _ => continue,
        };
        if let Some(name) = property(prim, "bim:name").and_then(text) {
            names.insert(id, name.to_owned());
        }
        kinds.insert(id, kind.to_owned());
    }
    (kinds, names)
}

/// The views of a model's marks, in ascending identity order.
fn views_of(
    numbering: &Numbering,
    kinds: &BTreeMap<ElementId, String>,
    names: &BTreeMap<ElementId, String>,
) -> Vec<MarkView> {
    numbering
        .all()
        .filter_map(|(id, mark)| {
            Some(MarkView {
                id: *id,
                kind: kinds.get(id)?.clone(),
                name: names.get(id).cloned().unwrap_or_default(),
                mark: mark.value.clone(),
                status: mark.status,
            })
        })
        .collect()
}

fn model_of(request: &OpenRequest, control: &Control) -> Result<Numbering, PlateError> {
    // Stored bytes are verified, not interpreted: the structural rules (A08-V01 among them) run
    // before the marks are read, so a damaged record is refused rather than reported or replayed.
    validate_layers(request.clone(), schema_identity()?, control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), control)?;
    let prims = read_prims(&reader, &opened.context, control)?;
    marks_of(&prims)
}

/// The marks of the verified head (empty for a store without a revision), in ascending identity
/// order.
pub fn marks(store: &RevisionStore) -> Result<Vec<MarkView>, PlateError> {
    let control = control();
    let revision = match store.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let content = revision.manifest().content();
    let damaged = |_| integrity("head manifest identity is not a UUIDv7");
    let project = ProjectId::parse(content.project().as_str()).map_err(damaged)?;
    let workspace = WorkspaceId::parse(content.workspace().as_str()).map_err(damaged)?;
    let (_, base) = load_base(store, &project, &workspace)?;
    let request = open_base(&base)?;
    // The structural rules run before the marks are read, so a damaged record is refused rather
    // than reported; the kinds and labels come from the same verified prims.
    let numbering = model_of(&request, &control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), &control)?;
    let prims = read_prims(&reader, &opened.context, &control)?;
    let (kinds, names) = names_and_kinds(&prims);
    Ok(views_of(&numbering, &kinds, &names))
}

/// The mark of one element before and after the operation's own revision.
type MarkChangePair = (Option<Mark>, Option<Mark>);

/// What the operation committed at `sequence` did to the mark of `target`: the mark before and
/// the mark after, or `None` when that revision did not change the mark at all (including the
/// case where the element did not exist before). The before state is what tells the command
/// kinds apart on a replay: an assign, a revoke and an approval each move the status between
/// different pairs, so a different request under the same op id is reuse even when the
/// resulting state agrees. One collision remains: the thickness withdrawal of `update-element`
/// leaves the same approved-to-assigned pair a revoke leaves, so a revoke retried under the
/// withdrawal's op id replays too (the stored canonical request of F05 tells them apart).
fn changed_by(
    store: &RevisionStore,
    sequence: u64,
    target: &ElementId,
    control: &Control,
) -> Result<Option<MarkChangePair>, PlateError> {
    let at = store.open_at(sequence)?;
    let after = model_of(&open_base(&base_of(&at)?)?, control)?
        .mark(target)
        .cloned();
    let before = if sequence > 1 {
        let previous = store.open_at(sequence - 1)?;
        model_of(&open_base(&base_of(&previous)?)?, control)?
            .mark(target)
            .cloned()
    } else {
        None
    };
    Ok((before != after).then_some((before, after)))
}

/// Assigns or replaces the mark of one element, or revokes its approval, and publishes the next
/// revision, or replays the operation.
///
/// Steps, each failing closed: admit the command; open and verify the head; decide replay from
/// the host ledger and the operation's own revision; decide with the engine; author with U02;
/// check the layer engine's writes against the declared footprint; commit through U06/U07 with
/// the strict verifier.
pub fn assign(
    store: &RevisionStore,
    context: &NumberingContext,
    command: &AssignMark,
    hook: &mut dyn CommitHook,
) -> Result<MarkChange, PlateError> {
    command.validate_arguments()?;
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let base_request = open_base(&base)?;
    let model = model_of(&base_request, &control)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let op_host = host_uuid(&context.op_id.to_string())?;
    // The ledger proves that the operation committed; what it left on the target is read from
    // its own revision, never from the head, so a later change of the same element cannot turn
    // an unrelated request into a replay. The host ledger stores no request bytes (the stored
    // canonical request of F05 replaces this), so the command kind is recognised from the
    // before-and-after signature of the operation's own revision: an assign leaves `(value,
    // assigned)` over an absent or different assigned mark, a revoke leaves `(value, assigned)`
    // over the approved one. A different request under the same op id is OP_ID_REUSE, whatever
    // the resulting state agrees with — with one boundary: the thickness withdrawal of
    // `update-element` leaves the same pair as a revoke, so a revoke retried under the
    // withdrawal's op id answers replayed (the two are told apart only by the stored canonical
    // request of F05).
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    let accepted = match (&prior, &recorded) {
        (PriorOp::Accepted { .. }, Some((_, sequence, _))) => {
            changed_by(store, *sequence, &command.target_id, &control)?
        }
        _ => None,
    };
    let same = match &command.mark {
        Some(value) => matches!(
            accepted,
            Some((
                before,
                Some(Mark {
                    value: after,
                    status: MarkStatus::Assigned,
                })
            )) if before.as_ref().is_none_or(|b| b.value != *value) && after == *value
        ),
        // A revoke is recognised by the status pair alone: it keeps whatever value it found.
        None => matches!(
            accepted,
            Some((
                Some(Mark {
                    status: MarkStatus::Approved,
                    ..
                }),
                Some(Mark {
                    status: MarkStatus::Assigned,
                    ..
                })
            ))
        ),
    };
    match prior {
        PriorOp::Accepted { .. } if same => {
            return Ok(MarkChange {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                marks: marks(store)?,
            });
        }
        PriorOp::Accepted { .. } => {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        PriorOp::Unseen => {}
    }
    let decision = model.assign(command)?;
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        base_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::Mark {
                id: command.target_id.to_string(),
                value: command.mark.clone(),
                status: Some(MarkStatus::Assigned),
            }],
        },
        &control,
    )?;
    check_footprint(&prepared, &decision.footprint)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &context.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(MarkChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(decision.footprint),
        changed_layers,
        marks: marks(store)?,
    })
}

/// Approves the mark of one element and publishes the next revision, or replays the operation.
/// The element must carry a mark that is not approved yet; the engine decides. The same flow as
/// [`assign`], with the approval status as the only authored field.
pub fn approve(
    store: &RevisionStore,
    context: &NumberingContext,
    command: &ApproveMark,
    hook: &mut dyn CommitHook,
) -> Result<MarkChange, PlateError> {
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let base_request = open_base(&base)?;
    let model = model_of(&base_request, &control)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let op_host = host_uuid(&context.op_id.to_string())?;
    // The host ledger stores no request bytes, so the command kind is recognised from the
    // before-and-after signature of the operation's own revision: an approval moves the mark
    // from `assigned` to `approved`. An assign or a revoke under the same op id is
    // OP_ID_REUSE even when the resulting state agrees.
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    let accepted = match (&prior, &recorded) {
        (PriorOp::Accepted { .. }, Some((_, sequence, _))) => {
            changed_by(store, *sequence, &command.target_id, &control)?
        }
        _ => None,
    };
    let same = matches!(
        accepted,
        Some((
            Some(Mark {
                status: MarkStatus::Assigned,
                ..
            }),
            Some(Mark {
                status: MarkStatus::Approved,
                ..
            })
        ))
    );
    match prior {
        PriorOp::Accepted { .. } if same => {
            return Ok(MarkChange {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                marks: marks(store)?,
            });
        }
        PriorOp::Accepted { .. } => {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        PriorOp::Unseen => {}
    }
    let decision = model.approve(command.target_id)?;
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        base_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::Mark {
                id: command.target_id.to_string(),
                value: None,
                status: Some(MarkStatus::Approved),
            }],
        },
        &control,
    )?;
    check_footprint(&prepared, &decision.footprint)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &context.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(MarkChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(decision.footprint),
        changed_layers,
        marks: marks(store)?,
    })
}
