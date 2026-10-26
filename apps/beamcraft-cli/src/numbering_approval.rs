// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A09 adapter: `ApproveNumbering` committed as the next revision, plus a read of the approval
//! stored in the head.
//!
//! The approved mark of a plate, beam, column or assembly is authored data of the element: three
//! optional properties of its prim in the authored layer (`steel:numbering:series`,
//! `steel:numbering:number`, `steel:numbering:basis`), written through the layer engine's
//! `Change::Mark` in one atomic revision. The engine ([`beamcraft_engine::numbering_approval`])
//! evaluates the preview again against the facts of the head and refuses a stale token, an
//! overlap of series and an empty model; this adapter only reads the facts, authors the marks,
//! checks the layer engine's write set against the declared footprint and commits through the
//! same verified path as every other command. The revision is an undo unit like any other
//! (`undo-workspace` restores the earlier marks with the rest of the state).
//!
//! Approving what is already approved writes nothing: when every mark and every approval record
//! stored in the head equals what the approval would write, the result is `unchanged` and no
//! revision is created. A retry of the same `op_id` with the same settings and token is a replay,
//! decided from the revision the host ledger recorded for it.
//!
//! Interface for readers of the mark (IFC export E02, BOM E03): read the three properties from the
//! element prim; the mark is `series` followed by `number`, with a `-` between them for an
//! assembly ([`beamcraft_engine::numbering_approval::Mark::text`]). A mark is current only while
//! [`numbering_status`] reports `current`.

use crate::assembly::read_prims;
use crate::numbering::state;
use crate::plate::{
    AUTHORED_LAYER, DERIVED_LAYER, PlateError, RevisionSummary, base_of, check_footprint, control,
    digest_of, host_uuid, integrity, load_base, open_base, prior_op_with, property, replay_summary,
    summarize, text,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::numbering::GroupKind;
use beamcraft_engine::numbering_approval::{
    ApproveNumbering, Assignment, Basis, Mark, Report, StoredMark, approve, assess,
};
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, MARK_BASIS, MARK_NUMBER, MARK_SERIES, Mark as LayerMark, Prepare,
    schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Reader, Value};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

/// Identities of one A09 call beyond the command itself.
#[derive(Debug, Clone)]
pub struct ApprovalContext {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet (an approval of such a store is `NOT_FOUND`).
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
}

/// Result of [`approve_numbering`].
#[derive(Debug, Clone, PartialEq)]
pub struct Approved {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// True when the head already held exactly this approval and nothing was written.
    pub unchanged: bool,
    /// The revision that holds the approval (the head for `unchanged`).
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay or an unchanged approval.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty when nothing was written).
    pub changed_layers: Vec<String>,
    /// Groups approved; `None` for a replay.
    pub groups: Option<usize>,
    /// Elements that carry a mark afterwards; `None` for a replay.
    pub marks: Option<usize>,
}

/// One stored mark with what it marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkView {
    /// The element.
    pub id: ElementId,
    /// Whether it is a part and of which kind, or an assembly; `None` for an element the numbering
    /// does not read as a part or an assembly.
    pub kind: Option<GroupKind>,
    /// The stored mark.
    pub mark: Mark,
}

/// The approval stored in the head, judged against the model.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusView {
    /// The revision it was read from.
    pub revision_id: String,
    /// Current, stale or not approved, with the reason.
    pub report: Report,
    /// Every stored mark in element-id order.
    pub marks: Vec<MarkView>,
}

fn damaged() -> PlateError {
    integrity("a numbering mark record is not readable")
}

fn refuse(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError::new(code, message)
}

/// The marks stored on the supported parts and assemblies of one pinned stage, in element-id
/// order. With `validate` the structural rules of the layer engine (the mark rules among them)
/// run first, so a damaged record is refused and never read as "unnumbered"; the head has been
/// validated by the reader of the facts already.
fn marks_of(
    request: &OpenRequest,
    validate: bool,
    control: &Control,
) -> Result<Vec<StoredMark>, PlateError> {
    if validate {
        validate_layers(request.clone(), schema_identity()?, control)?;
    }
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), control)?;
    let prims = read_prims(&reader, &opened.context, control)?;
    let mut marks = Vec::new();
    for prim in &prims {
        let Some(id) = prim
            .element_id
            .as_deref()
            .and_then(|i| ElementId::parse(i).ok())
        else {
            continue;
        };
        // Only owned 0.1.0 parametric elements are interpreted; anything else stays inert.
        let supported = !prim.sources.is_empty()
            && prim.sources.iter().all(|s| s.owned)
            && property(prim, "bim:schemaVersion").and_then(text) == Some("0.1.0")
            && property(prim, "bim:geometryMode").and_then(text) == Some("parametric")
            && matches!(
                property(prim, "bim:elementKind").and_then(text),
                Some("plate" | "beam" | "column" | "assembly")
            );
        if !supported {
            continue;
        }
        let series = property(prim, MARK_SERIES);
        let number = property(prim, MARK_NUMBER);
        let basis = property(prim, MARK_BASIS);
        if series.is_none() && number.is_none() && basis.is_none() {
            continue;
        }
        let number = match number {
            Some(Value::Int(n)) => u64::try_from(*n).ok(),
            Some(Value::Uint(n)) => Some(*n),
            _ => None,
        };
        let (Some(series), Some(number), Some(basis)) =
            (series.and_then(text), number, basis.and_then(text))
        else {
            return Err(damaged());
        };
        marks.push(StoredMark {
            id,
            mark: Mark {
                series: series.to_owned(),
                number,
            },
            basis: basis.to_owned(),
        });
    }
    marks.sort_by_key(|m| m.id);
    Ok(marks)
}

/// The revision id of the verified head and the marks stored on it.
/// The revision id of the verified head and the marks stored on it. Public for the readers of
/// the mark named in the module docs (the IFC and BOM exports).
pub fn head_marks(store: &RevisionStore) -> Result<(String, Vec<StoredMark>), PlateError> {
    let revision = store.open()?;
    let base = base_of(&revision)?;
    let marks = marks_of(&open_base(&base)?, false, &control())?;
    Ok((revision.info().revision().as_str().to_owned(), marks))
}

/// The approval stored in the head and every stored mark, judged against the facts of the head.
/// Nothing is written.
pub fn numbering_status(store: &RevisionStore) -> Result<StatusView, PlateError> {
    let facts = state(store)?;
    let report = assess(&facts.parts, &facts.assemblies, &facts.stored)
        .map_err(|e| refuse(e.code(), e.message))?;
    let kinds: BTreeMap<ElementId, GroupKind> = facts
        .parts
        .iter()
        .map(|p| (p.id, GroupKind::Part(p.kind)))
        .chain(facts.assemblies.iter().map(|a| (a.id, GroupKind::Assembly)))
        .collect();
    Ok(StatusView {
        revision_id: facts.revision_id,
        report,
        marks: facts
            .stored
            .into_iter()
            .map(|m| MarkView {
                kind: kinds.get(&m.id).copied(),
                id: m.id,
                mark: m.mark,
            })
            .collect(),
    })
}

fn reuse() -> PlateError {
    CommandError::new(
        ErrorCode::OpIdReuse,
        "op_id",
        "operation id was accepted with different content",
    )
    .into()
}

fn layer_mark(assignment: &Assignment, basis: &str) -> Result<Change, PlateError> {
    Ok(Change::Mark {
        id: assignment.id.to_string(),
        mark: LayerMark {
            series: assignment.mark.series.clone(),
            number: u32::try_from(assignment.mark.number)
                .map_err(|_| refuse(ErrorCode::LimitExceeded, "a mark number is too large"))?,
            basis: basis.to_owned(),
        },
    })
}

/// Whether the revision the ledger recorded for an operation is the one this approval wrote: its
/// marks all carry this approval's record and the revision before it did not already hold them.
fn written_by(
    store: &RevisionStore,
    sequence: u64,
    basis: &str,
    control: &Control,
) -> Result<bool, PlateError> {
    let at = marks_of(
        &open_base(&base_of(&store.open_at(sequence)?)?)?,
        true,
        control,
    )?;
    if at.is_empty() || at.iter().any(|m| m.basis != basis) {
        return Ok(false);
    }
    if sequence > 1 {
        let before = marks_of(
            &open_base(&base_of(&store.open_at(sequence - 1)?)?)?,
            true,
            control,
        )?;
        if before == at {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Approves the numbering the preview with the command's settings proposed and publishes the next
/// revision, or replays the operation, or reports that the head already holds this approval.
///
/// Steps, each failing closed: open and verify the head; decide replay from the host ledger and
/// the revision it recorded; read the facts of the same head and let the engine evaluate the
/// preview again (a stale token, an overlap and an empty model are refused with nothing
/// written); decide whether anything would change; author the marks with U02; check the layer
/// engine's writes against the declared footprint; commit through U06/U07 with the strict
/// verifier.
pub fn approve_numbering(
    store: &RevisionStore,
    context: &ApprovalContext,
    command: &ApproveNumbering,
    hook: &mut dyn CommitHook,
) -> Result<Approved, PlateError> {
    command.fingerprint()?;
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let Some(head_revision) = head.as_ref() else {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "store",
            "the store has no revision to number",
        )
        .into());
    };
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let command_basis = Basis {
        settings: command.settings.clone(),
        token: command.preview_token,
    }
    .to_text();
    let op_host = host_uuid(&context.op_id.to_string())?;
    // The ledger proves that the operation committed; what it committed is read from its own
    // revision, never from the head, so a later approval is not mistaken for this one.
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    match prior {
        PriorOp::Accepted { .. } => {
            let same = match &recorded {
                Some((_, sequence, _)) => written_by(store, *sequence, &command_basis, &control)?,
                None => false,
            };
            if !same {
                return Err(reuse());
            }
            return Ok(Approved {
                replayed: true,
                unchanged: false,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                groups: None,
                marks: None,
            });
        }
        PriorOp::Unseen => {}
    }
    let facts = state(store)?;
    if facts.revision_id != head_revision.info().revision().as_str() {
        return Err(refuse(
            ErrorCode::BaseRevisionMismatch,
            "the head moved while the numbering was approved",
        ));
    }
    let approval = approve(&facts.parts, &facts.assemblies, command)
        .map_err(|e| refuse(e.code(), e.message))?;
    let basis = approval.basis.to_text();
    let unchanged = facts.stored.len() == approval.assignments.len()
        && approval.assignments.iter().all(|a| {
            facts
                .stored
                .iter()
                .any(|m| m.id == a.id && m.mark == a.mark && m.basis == basis)
        });
    if unchanged {
        return Ok(Approved {
            replayed: false,
            unchanged: true,
            revision: summarize(head_revision.info()),
            footprint: None,
            changed_layers: Vec::new(),
            groups: Some(approval.groups),
            marks: Some(approval.assignments.len()),
        });
    }
    let footprint = approval.footprint()?;
    let changes = approval
        .assignments
        .iter()
        .map(|a| layer_mark(a, &basis))
        .collect::<Result<Vec<_>, _>>()?;
    let base_request = open_base(&base)?;
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
            changes,
        },
        &control,
    )?;
    check_footprint(&prepared, &footprint)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &footprint,
            fingerprint: command.fingerprint()?,
            prepared: &prepared,
            schema,
            project: &base.project,
            workspace: &base.workspace,
            revision_id: &context.revision_id,
            op: op_host,
            base_generation: base.generation,
            expected: &base.expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(Approved {
        replayed: false,
        unchanged: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        groups: Some(approval.groups),
        marks: Some(approval.assignments.len()),
    })
}
