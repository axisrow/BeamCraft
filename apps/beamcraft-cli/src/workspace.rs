// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P06 adapter: a personal workspace as a draft beside the published store.
//!
//! A workspace is a directory that is itself a revision store (its own blobs, head log and
//! outcome ledger) plus one small metadata file, `workspace.json`, that records the workspace id,
//! the owning user and the published revision the draft was forked from. Every edit command works
//! on it through `--store DIR` exactly as on the published store, so a draft is never connected to
//! the published root: nothing reaches the published head until `publish`.
//!
//! * `create` forks the published head: the same layer bytes become the first revision of the
//!   draft under the workspace's own id.
//! * `status` compares the draft head with its base and with the published head.
//! * `publish` is a fast-forward only: it commits the draft head's layers as the next published
//!   revision when the published head is still the base, through the same verifier as every other
//!   commit. A moved published head is `BASE_REVISION_MISMATCH`; there is no merge.
//!
//! Honesty limits: no authentication or permission model (the user id is a recorded label), no
//! merge or rebase, a draft keeps one linear history, and `publish` publishes the draft head as one
//! revision. Two workspaces of the same user are two directories and do not see each other.

use crate::plate::{PlateError, RevisionSummary, commit_verified, host_uuid, integrity, summarize};
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, ExpectedBase, LayerEntry, Revision, RevisionDraft, RevisionError,
    RevisionStore, Uuid7,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const META_FILE: &str = "workspace.json";
const META_MAGIC: &str = "beamcraft-workspace 1";
const META_CAP: u64 = 4096;

fn unusable() -> PlateError {
    PlateError {
        code: ErrorCode::BackendUnavailable,
        message: "workspace metadata is not usable",
    }
}

/// What a workspace remembers about its fork point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMeta {
    /// Identity of the workspace.
    pub workspace_id: Uuid7,
    /// The user the workspace belongs to (a recorded label; there is no authentication yet).
    pub user_id: Uuid7,
    /// The published revision the draft is based on.
    pub base_revision: Uuid7,
    /// Manifest digest of that revision.
    pub base_manifest: ContentDigest,
    /// Draft head sequence that is already published (or was forked from the published head).
    pub synced_sequence: u64,
}

impl WorkspaceMeta {
    fn encode(&self) -> String {
        format!(
            "{META_MAGIC}\nworkspace_id={}\nuser_id={}\nbase_revision={}\nbase_manifest={}\nsynced_sequence={}\n",
            self.workspace_id.as_str(),
            self.user_id.as_str(),
            self.base_revision.as_str(),
            self.base_manifest,
            self.synced_sequence
        )
    }

    fn decode(text: &str) -> Result<Self, PlateError> {
        let mut lines = text.lines();
        if lines.next() != Some(META_MAGIC) {
            return Err(unusable());
        }
        let mut field = |key: &str| -> Result<&str, PlateError> {
            lines
                .next()
                .and_then(|l| l.strip_prefix(key))
                .and_then(|l| l.strip_prefix('='))
                .ok_or_else(unusable)
        };
        let workspace_id = host_uuid(field("workspace_id")?)?;
        let user_id = host_uuid(field("user_id")?)?;
        let base_revision = host_uuid(field("base_revision")?)?;
        let base_manifest = field("base_manifest")?.parse().map_err(|_| unusable())?;
        let synced_sequence = field("synced_sequence")?.parse().map_err(|_| unusable())?;
        if lines.next().is_some() {
            return Err(unusable());
        }
        Ok(Self {
            workspace_id,
            user_id,
            base_revision,
            base_manifest,
            synced_sequence,
        })
    }
}

fn meta_path(dir: &Path) -> PathBuf {
    dir.join(META_FILE)
}

/// Reads and validates the metadata of a workspace directory.
pub fn read_meta(dir: &Path) -> Result<WorkspaceMeta, PlateError> {
    let path = meta_path(dir);
    // A link is never followed: the metadata is a regular file inside the workspace.
    let metadata = fs::symlink_metadata(&path).map_err(|_| unusable())?;
    if !metadata.is_file() || metadata.len() > META_CAP {
        return Err(unusable());
    }
    let text = fs::read_to_string(&path).map_err(|_| unusable())?;
    WorkspaceMeta::decode(&text)
}

/// Replaces the metadata atomically: a temporary file, synced, then renamed over the old one.
pub(crate) fn write_meta(dir: &Path, meta: &WorkspaceMeta) -> Result<(), PlateError> {
    let temporary = dir.join(format!("{META_FILE}.tmp"));
    // Exclusive creation: a leftover or planted entry (even a link) is removed as an entry, never
    // written through.
    if fs::symlink_metadata(&temporary).is_ok() {
        fs::remove_file(&temporary).map_err(|_| unusable())?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| unusable())?;
    file.write_all(meta.encode().as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| unusable())?;
    fs::rename(&temporary, meta_path(dir)).map_err(|_| unusable())
}

/// Identities of one `create` call.
#[derive(Debug, Clone)]
pub struct CreateWorkspace {
    /// The workspace identity.
    pub workspace_id: Uuid7,
    /// The owning user.
    pub user_id: Uuid7,
    /// Identity of the draft's first revision.
    pub revision_id: Uuid7,
    /// Operation that creates it.
    pub op_id: Uuid7,
}

/// Where a workspace stands against its base and the published head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceStatus {
    /// The recorded metadata.
    pub meta: WorkspaceMeta,
    /// The draft head.
    pub draft_head: RevisionSummary,
    /// Draft revisions not yet published.
    pub unsent_revisions: u64,
    /// The published head.
    pub published_head: RevisionSummary,
    /// Whether the published head is still the base (`false`: another change was published).
    pub base_is_current: bool,
}

fn head_of(
    store: &RevisionStore,
) -> Result<Option<beamcraft_host::revision::RevisionInfo>, PlateError> {
    store.head().map_err(PlateError::from)
}

/// Resolves the exact accepted pin independently of the local published-operation prefix.
/// The current-head path avoids a history search; `sequence_of` bounds a historical lookup by
/// the host's MAX_REVISIONS cap. An unavailable or compacted pin never falls back to local USD.
pub(crate) fn published_base(
    published: &RevisionStore,
    meta: &WorkspaceMeta,
) -> Result<Revision, PlateError> {
    let head = published
        .head()?
        .ok_or_else(|| integrity("the published store has no revision"))?;
    let info = if head.revision() == &meta.base_revision {
        head
    } else {
        let sequence = published
            .sequence_of(&meta.base_revision)?
            .ok_or_else(|| integrity("the accepted workspace base is unavailable"))?;
        published.info_at(sequence)?
    };
    if info.manifest_digest() != meta.base_manifest {
        return Err(integrity(
            "the accepted workspace base pin disagrees with history",
        ));
    }
    Ok(published.open_at(info.sequence())?)
}

/// Forks the published head into a new workspace directory.
///
/// Fails with `NOT_FOUND` when nothing is published, and with `DUPLICATE_ID` when the directory
/// already is a different workspace. Running it again with the same identities finishes an
/// interrupted creation and otherwise changes nothing.
pub fn create_workspace(
    published: &RevisionStore,
    workspace: &RevisionStore,
    dir: &Path,
    request: &CreateWorkspace,
    hook: &mut dyn CommitHook,
) -> Result<WorkspaceStatus, PlateError> {
    let revision = match published.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => {
            return Err(PlateError {
                code: ErrorCode::NotFound,
                message: "there is no published revision to fork",
            });
        }
        Err(error) => return Err(error.into()),
    };
    let content = revision.manifest().content();
    let exists = meta_path(dir).exists();
    let meta = if exists {
        let existing = read_meta(dir)?;
        if existing.workspace_id != request.workspace_id || existing.user_id != request.user_id {
            return Err(PlateError {
                code: ErrorCode::DuplicateId,
                message: "the directory is already another workspace",
            });
        }
        existing
    } else {
        if head_of(workspace)?.is_some() {
            return Err(PlateError {
                code: ErrorCode::DuplicateId,
                message: "the directory already holds revisions",
            });
        }
        let meta = WorkspaceMeta {
            workspace_id: request.workspace_id.clone(),
            user_id: request.user_id.clone(),
            base_revision: revision.info().revision().clone(),
            base_manifest: revision.info().manifest_digest(),
            synced_sequence: 1,
        };
        write_meta(dir, &meta)?;
        meta
    };
    if head_of(workspace)?.is_none() {
        let mut entries = Vec::new();
        for entry in content.entries() {
            let bytes = revision.layer_bytes(entry.name())?;
            workspace.blobs().publish(&bytes).map_err(|e| PlateError {
                code: e.code(),
                message: "layer blob could not be stored",
            })?;
            entries.push(LayerEntry::from_bytes(entry.name(), entry.role(), &bytes)?);
        }
        let draft = RevisionDraft {
            project: content.project().clone(),
            workspace: meta.workspace_id.clone(),
            revision: request.revision_id.clone(),
            op: request.op_id.clone(),
            workspace_generation: 1,
            schema: content.schema(),
            root_layer: content.root_layer().to_owned(),
            entries,
        };
        commit_verified(workspace, &draft, &ExpectedBase::Initial, hook)?;
    }
    status(published, workspace, dir)
}

/// Reads the workspace status: the metadata, the draft head and the published head.
pub fn status(
    published: &RevisionStore,
    workspace: &RevisionStore,
    dir: &Path,
) -> Result<WorkspaceStatus, PlateError> {
    let meta = read_meta(dir)?;
    let draft = head_of(workspace)?.ok_or_else(|| integrity("the workspace has no revision"))?;
    let head =
        head_of(published)?.ok_or_else(|| integrity("the published store has no revision"))?;
    Ok(WorkspaceStatus {
        unsent_revisions: draft.sequence().saturating_sub(meta.synced_sequence),
        base_is_current: head.revision() == &meta.base_revision,
        draft_head: summarize(&draft),
        published_head: summarize(&head),
        meta,
    })
}

/// Result of `publish`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// True when the draft head was already the published head and nothing was written.
    pub replayed: bool,
    /// The published revision that holds the draft's content.
    pub revision: RevisionSummary,
    /// The workspace status after the publish.
    pub status: WorkspaceStatus,
}

/// Publishes the draft head as the next published revision, fast-forward only.
///
/// The draft head's layers are committed to the published store with the draft head's revision id
/// as both the operation and the published revision id, so a retry after an interruption (even in
/// a new process) is the same request and is recognised. The published head must still
/// be the base (`BASE_REVISION_MISMATCH` otherwise) and there must be something to publish
/// (`INVALID_ARGUMENT`). On success the base moves to the new published revision.
pub fn publish_workspace(
    published: &RevisionStore,
    workspace: &RevisionStore,
    dir: &Path,
    hook: &mut dyn CommitHook,
) -> Result<Published, PlateError> {
    let meta = read_meta(dir)?;
    let draft = workspace.open()?;
    let draft_sequence = draft.info().sequence();
    let op = draft.info().revision().clone();
    let head = published.open().map_err(|e| match e {
        RevisionError::NoRevision => integrity("the published store has no revision"),
        other => other.into(),
    })?;
    // The commit reached the published store but the metadata was not updated: finish it.
    if head.manifest().content().op() == &op {
        let next = WorkspaceMeta {
            base_revision: head.info().revision().clone(),
            base_manifest: head.info().manifest_digest(),
            synced_sequence: draft_sequence,
            ..meta
        };
        write_meta(dir, &next)?;
        return Ok(Published {
            replayed: true,
            revision: summarize(head.info()),
            status: status(published, workspace, dir)?,
        });
    }
    if draft_sequence <= meta.synced_sequence {
        return Err(PlateError {
            code: ErrorCode::InvalidArgument,
            message: "the workspace has nothing to publish",
        });
    }
    if head.info().revision() != &meta.base_revision {
        return Err(PlateError {
            code: ErrorCode::BaseRevisionMismatch,
            message: "the published head moved since the workspace was forked",
        });
    }
    let content = draft.manifest().content();
    let published_content = head.manifest().content();
    let mut entries = Vec::new();
    for entry in content.entries() {
        let bytes = draft.layer_bytes(entry.name())?;
        published.blobs().publish(&bytes).map_err(|e| PlateError {
            code: e.code(),
            message: "layer blob could not be stored",
        })?;
        entries.push(LayerEntry::from_bytes(entry.name(), entry.role(), &bytes)?);
    }
    let generation = published_content
        .workspace_generation()
        .checked_add(1)
        .ok_or(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "generation exhausted",
        })?;
    let next = RevisionDraft {
        project: published_content.project().clone(),
        workspace: published_content.workspace().clone(),
        revision: op.clone(),
        op,
        workspace_generation: generation,
        schema: content.schema(),
        root_layer: content.root_layer().to_owned(),
        entries,
    };
    let operations =
        crate::command_evidence::interval(workspace, meta.synced_sequence, draft_sequence)?;
    let source = crate::command_evidence::aggregate(workspace, meta.synced_sequence, &operations)?;
    if source.head.manifest_digest() != draft.info().manifest_digest() {
        return Err(integrity(
            "publication source differs from the verified draft head",
        ));
    }
    let evidence = source
        .footprint
        .as_ref()
        .map(|footprint| {
            crate::command_evidence::store(
                published,
                &next.op,
                draft.info().manifest_digest(),
                footprint,
            )
        })
        .transpose()?;
    let committed = crate::plate::commit_verified_with_evidence(
        published,
        &next,
        &ExpectedBase::Revision(meta.base_revision.clone()),
        evidence,
        hook,
    )?;
    write_meta(
        dir,
        &WorkspaceMeta {
            base_revision: committed.info.revision().clone(),
            base_manifest: committed.info.manifest_digest(),
            synced_sequence: draft_sequence,
            ..meta
        },
    )?;
    Ok(Published {
        replayed: committed.outcome == CommitOutcome::Replayed,
        revision: summarize(&committed.info),
        status: status(published, workspace, dir)?,
    })
}
