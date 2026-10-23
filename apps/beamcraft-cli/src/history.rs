// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P07 adapter: durable undo and redo of a workspace draft.
//!
//! One accepted command is one revision of the draft (U06/U07), which is exactly one undo unit.
//! Undo does not edit anything: it commits a **new** revision whose layers are byte for byte the
//! layers of the state before the undone command, so identity, relationships, mesh and stored
//! quantities come back together or not at all, and the log stays linear and verified. Redo
//! commits the layers of the state that was undone.
//!
//! The stacks are never stored. They are derived from the revision log plus one append-only file,
//! `undo.log`, that lists the revisions that were produced by an undo or a redo (an intent is
//! synced to the file **before** its commit, and a revision is classified by its identity):
//!
//! ```text
//! for each revision after the first
//!   undo or redo (listed in undo.log)  -> move one entry between the stacks
//!   any other revision (an edit)       -> push the previous state on the undo stack, clear redo
//! ```
//!
//! So an edit made by any command (`update-element`, `copy-element`, ...) is an undo unit without
//! that command knowing about undo, a new edit after an undo discards the redo stack, and a
//! process restart or a crash between the intent and the commit leaves a consistent state: an
//! intent without a revision is ignored, and repeating it with the same `op_id` finishes it.
//!
//! Limits: snapshot undo stops at the latest rebase candidate (or the fork point), preserving
//! the combined accepted basis; later edits remain undo/redo units. History is read from the log
//! (bounded to [`MAX_REVISIONS`] revisions and [`LOG_CAP`] bytes); undo does not touch the
//! published store (a later publish sends the draft head as it is).

use crate::plate::{PlateError, RevisionSummary, commit_verified, host_uuid, integrity, summarize};
use crate::workspace::WorkspaceState;
use beamcraft_host::revision::{
    CommitHook, ExpectedBase, LayerEntry, RevisionDraft, RevisionInfo, RevisionStore, Uuid7,
};
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const LOG_FILE: &str = "undo.log";
/// Largest accepted `undo.log`.
pub const LOG_CAP: u64 = 256 * 1024;
/// Most draft revisions one history derivation reads.
pub const MAX_REVISIONS: u64 = 4096;

fn unusable() -> PlateError {
    PlateError {
        code: ErrorCode::BackendUnavailable,
        message: "the undo log is not usable",
    }
}

/// Which way a step moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Restore the state before the last unit.
    Undo,
    /// Restore the state that was undone.
    Redo,
}

impl Direction {
    fn word(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Redo => "redo",
        }
    }
}

#[derive(Debug, Clone)]
struct Intent {
    direction: Direction,
    revision: Uuid7,
    op: Uuid7,
    base: Uuid7,
}

fn log_path(dir: &Path) -> PathBuf {
    dir.join(LOG_FILE)
}

fn read_log(dir: &Path) -> Result<Vec<Intent>, PlateError> {
    let path = log_path(dir);
    // A link is never followed: the log is a regular file inside the workspace.
    let length = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => return Err(unusable()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(unusable()),
    };
    if length > LOG_CAP {
        return Err(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "the undo log is full",
        });
    }
    let text = fs::read_to_string(&path).map_err(|_| unusable())?;
    text.lines()
        .map(|line| {
            let mut words = line.split(' ');
            let direction = match words.next() {
                Some("undo") => Direction::Undo,
                Some("redo") => Direction::Redo,
                _ => return Err(unusable()),
            };
            let mut next = || words.next().ok_or_else(unusable).and_then(host_uuid);
            let (revision, op, base) = (next()?, next()?, next()?);
            if words.next().is_some() {
                return Err(unusable());
            }
            Ok(Intent {
                direction,
                revision,
                op,
                base,
            })
        })
        .collect()
}

/// Appends one intent and syncs it. The caller commits the revision afterwards.
fn append_intent(dir: &Path, intent: &Intent) -> Result<(), PlateError> {
    let line = format!(
        "{} {} {} {}\n",
        intent.direction.word(),
        intent.revision.as_str(),
        intent.op.as_str(),
        intent.base.as_str()
    );
    let path = log_path(dir);
    let used = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => return Err(unusable()),
        Err(_) => 0,
    };
    let line_length = u64::try_from(line.len()).map_err(|_| unusable())?;
    if used.saturating_add(line_length) > LOG_CAP {
        return Err(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "the undo log is full",
        });
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|_| unusable())?;
    file.write_all(line.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| unusable())
}

/// The undo and redo stacks of a draft, derived from its revision log.
#[derive(Debug, Clone)]
pub struct Timeline {
    /// The draft head.
    pub head: RevisionInfo,
    /// Log sequences of the states an undo restores, the next one last.
    pub undo: Vec<u64>,
    /// Log sequences of the states a redo restores, the next one last.
    pub redo: Vec<u64>,
    revisions: Vec<Uuid7>,
}

fn head_of(store: &RevisionStore) -> Result<RevisionInfo, PlateError> {
    store
        .head()?
        .ok_or_else(|| integrity("the workspace has no revision"))
}

impl Timeline {
    /// The draft head as a summary.
    pub fn head_summary(&self) -> RevisionSummary {
        summarize(&self.head)
    }
}

/// Derives the undo and redo stacks of the draft in `dir`.
pub fn timeline(workspace: &RevisionStore, dir: &Path) -> Result<Timeline, PlateError> {
    let state = WorkspaceState::open(dir)?;
    let meta = state.meta().ok_or_else(crate::workspace::unusable)?;
    let head = head_of(workspace)?;
    if meta.rebase_sequence > head.sequence() {
        return Err(integrity(
            "workspace rebase boundary is beyond its draft head",
        ));
    }
    if head.sequence() > MAX_REVISIONS {
        return Err(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "the draft history is too long to derive",
        });
    }
    let kinds: BTreeMap<String, Direction> = read_log(dir)?
        .into_iter()
        .map(|i| (i.revision.as_str().to_owned(), i.direction))
        .collect();
    let (mut undo, mut redo) = (Vec::new(), Vec::new());
    let mut revisions = Vec::new();
    for sequence in 1..=head.sequence() {
        let info = workspace.info_at(sequence)?;
        let before = sequence.saturating_sub(1);
        if sequence == meta.rebase_sequence {
            undo.clear();
            redo.clear();
        } else if sequence > 1 {
            match kinds.get(info.revision().as_str()) {
                Some(Direction::Undo) => {
                    undo.pop()
                        .ok_or_else(|| integrity("an undo has nothing to restore"))?;
                    redo.push(before);
                }
                Some(Direction::Redo) => {
                    redo.pop()
                        .ok_or_else(|| integrity("a redo has nothing to restore"))?;
                    undo.push(before);
                }
                None => {
                    undo.push(before);
                    redo.clear();
                }
            }
        }
        revisions.push(info.revision().clone());
    }
    Ok(Timeline {
        head,
        undo,
        redo,
        revisions,
    })
}

/// Identities of one undo or redo call.
#[derive(Debug, Clone)]
pub struct Step {
    /// Identity of the revision the step commits.
    pub revision_id: Uuid7,
    /// The operation; repeating it replays the step.
    pub op_id: Uuid7,
}

/// Result of an undo or redo.
#[derive(Debug, Clone)]
pub struct Stepped {
    /// True when the step was already committed and nothing was written.
    pub replayed: bool,
    /// The draft revision that holds the restored state.
    pub revision: RevisionSummary,
    /// The log sequence whose state was restored (`None` for a replay).
    pub restored_sequence: Option<u64>,
    /// Undo units left after the step.
    pub undo_units: usize,
    /// Redo units left after the step.
    pub redo_units: usize,
}

/// Undoes or redoes one unit of the draft by committing the restored layers as a new revision.
///
/// `INVALID_ARGUMENT` when there is nothing to undo or redo; the same `op_id` with the other
/// direction (or recorded against an older head) is `OP_ID_REUSE`. The commit is expected against
/// the draft head, so a concurrent edit loses with `BASE_REVISION_MISMATCH`.
pub fn step(
    workspace: &RevisionStore,
    dir: &Path,
    direction: Direction,
    request: &Step,
    hook: &mut dyn CommitHook,
) -> Result<Stepped, PlateError> {
    let line = timeline(workspace, dir)?;
    let known = read_log(dir)?.into_iter().find(|i| i.op == request.op_id);
    let revision_id = match &known {
        Some(intent) => {
            if intent.direction != direction {
                return Err(PlateError {
                    code: ErrorCode::OpIdReuse,
                    message: "the operation id was used for the other direction",
                });
            }
            let position = line.revisions.iter().position(|r| r == &intent.revision);
            if let Some(index) = position {
                let sequence = u64::try_from(index.saturating_add(1)).map_err(|_| unusable())?;
                let info = workspace.info_at(sequence)?;
                return Ok(Stepped {
                    replayed: true,
                    restored_sequence: None,
                    revision: summarize(&info),
                    undo_units: line.undo.len(),
                    redo_units: line.redo.len(),
                });
            }
            if &intent.base != line.head.revision() {
                return Err(PlateError {
                    code: ErrorCode::OpIdReuse,
                    message: "the operation was recorded against an older draft head",
                });
            }
            intent.revision.clone()
        }
        None => request.revision_id.clone(),
    };
    let target = match direction {
        Direction::Undo => line.undo.last(),
        Direction::Redo => line.redo.last(),
    }
    .copied()
    .ok_or(PlateError {
        code: ErrorCode::InvalidArgument,
        message: match direction {
            Direction::Undo => "there is nothing to undo",
            Direction::Redo => "there is nothing to redo",
        },
    })?;
    if known.is_none() {
        append_intent(
            dir,
            &Intent {
                direction,
                revision: revision_id.clone(),
                op: request.op_id.clone(),
                base: line.head.revision().clone(),
            },
        )?;
    }
    let restored = workspace.open_at(target)?;
    let content = restored.manifest().content();
    let mut entries = Vec::new();
    for entry in content.entries() {
        let bytes = restored.layer_bytes(entry.name())?;
        entries.push(LayerEntry::from_bytes(entry.name(), entry.role(), &bytes)?);
    }
    let head_content = line.head.manifest().content();
    let generation = head_content
        .workspace_generation()
        .checked_add(1)
        .ok_or(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "generation exhausted",
        })?;
    let draft = RevisionDraft {
        project: head_content.project().clone(),
        workspace: head_content.workspace().clone(),
        revision: revision_id,
        op: request.op_id.clone(),
        workspace_generation: generation,
        schema: content.schema(),
        root_layer: content.root_layer().to_owned(),
        entries,
    };
    let committed = commit_verified(
        workspace,
        &draft,
        &ExpectedBase::Revision(line.head.revision().clone()),
        hook,
    )?;
    let after = timeline(workspace, dir)?;
    Ok(Stepped {
        replayed: false,
        revision: summarize(&committed.info),
        restored_sequence: Some(target),
        undo_units: after.undo.len(),
        redo_units: after.redo.len(),
    })
}
