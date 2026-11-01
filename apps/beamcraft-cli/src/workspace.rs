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

use crate::publication::commit_verified;
use beamcraft_engine::scope::WorkspaceScope;
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, ExpectedBase, LayerEntry, LayerRole, Revision, RevisionDraft,
    RevisionError, RevisionStore, Uuid7,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use session::Refusal;
use session::pipeline::{RevisionSummary, host_uuid, integrity, summarize};
use std::path::{Path, PathBuf};

mod state;
pub use state::WorkspaceState;
pub mod compensation;
pub mod conflicts;
mod pending;
pub mod rebase;
pub mod rebase_receipt;
pub mod resolve;

const META_FILE: &str = "workspace.json";
const META_MAGIC: &str = "beamcraft-workspace 1";
const META_MAGIC_REBASED: &str = "beamcraft-workspace 2";
const META_MAGIC_SCOPED: &str = "beamcraft-workspace 3";
const META_CAP: u64 = 4096;

pub(crate) fn unusable() -> Refusal {
    Refusal {
        rule_id: None,
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
    /// Latest local rebase candidate. Publication cannot end before it. Zero keeps v1 bytes;
    /// this is separate from the already-published prefix and never marks unsent edits published.
    pub rebase_sequence: u64,
    /// The authorized section/zone write scope (U19); `None` for the unscoped 0.1 behaviour.
    /// Changed only by the explicit authorization of `create-workspace`, never by a claim.
    pub scope: Option<WorkspaceScope>,
}

impl WorkspaceMeta {
    fn magic(&self) -> &'static str {
        if self.scope.is_some() {
            META_MAGIC_SCOPED
        } else if self.rebase_sequence != 0 {
            META_MAGIC_REBASED
        } else {
            META_MAGIC
        }
    }

    fn encode(&self) -> String {
        let mut out = format!(
            "{}\nworkspace_id={}\nuser_id={}\nbase_revision={}\nbase_manifest={}\nsynced_sequence={}\n",
            self.magic(),
            self.workspace_id.as_str(),
            self.user_id.as_str(),
            self.base_revision.as_str(),
            self.base_manifest,
            self.synced_sequence
        );
        if self.rebase_sequence != 0 {
            out.push_str(&format!("rebase_sequence={}\n", self.rebase_sequence));
        }
        if let Some(scope) = &self.scope {
            out.push_str(&format!(
                "scope_code={}\nscope_section={}\n",
                scope.code, scope.section
            ));
            if !scope.zones.is_empty() {
                let zones = scope.zones.iter().cloned().collect::<Vec<_>>().join(",");
                out.push_str(&format!("scope_zones={zones}\n"));
            }
            out.push_str(&format!("scope_shared={}\n", scope.shared));
        }
        out
    }

    fn decode(text: &str) -> Result<Self, Refusal> {
        let rows: Vec<&str> = text.lines().collect();
        let magic = rows.first().copied();
        let scoped = magic == Some(META_MAGIC_SCOPED);
        if magic != Some(META_MAGIC) && magic != Some(META_MAGIC_REBASED) && !scoped {
            return Err(unusable());
        }
        let mut at = 1usize;
        let mut field = |key: &str| -> Result<&str, Refusal> {
            let line = rows.get(at).copied().ok_or_else(unusable)?;
            // A non-matching row is left in place: the optional fields below decide by key.
            let value = line
                .strip_prefix(key)
                .and_then(|l| l.strip_prefix('='))
                .ok_or_else(unusable)?;
            at += 1;
            Ok(value)
        };
        let workspace_id = host_uuid(field("workspace_id")?)?;
        let user_id = host_uuid(field("user_id")?)?;
        let base_revision = host_uuid(field("base_revision")?)?;
        let base_manifest = field("base_manifest")?.parse().map_err(|_| unusable())?;
        let synced_sequence = field("synced_sequence")?.parse().map_err(|_| unusable())?;
        let rebase_sequence = match field("rebase_sequence") {
            Ok(sequence) => {
                let sequence = sequence.parse::<u64>().map_err(|_| unusable())?;
                if sequence == 0 || sequence > beamcraft_host::revision::MAX_REVISIONS {
                    return Err(unusable());
                }
                sequence
            }
            Err(_) => 0,
        };
        let scope = if scoped {
            let code = field("scope_code")?.to_owned();
            let section = field("scope_section")?.to_owned();
            let mut zones = std::collections::BTreeSet::new();
            for zone in field("scope_zones").unwrap_or_default().split(',') {
                if !zone.is_empty() {
                    zones.insert(zone.to_owned());
                }
            }
            let shared = match field("scope_shared") {
                Ok("true") => true,
                Ok("false") => false,
                _ => return Err(unusable()),
            };
            Some(WorkspaceScope {
                code,
                section,
                zones,
                shared,
            })
        } else {
            None
        };
        if at != rows.len() {
            return Err(unusable());
        }
        let meta = Self {
            workspace_id,
            user_id,
            base_revision,
            base_manifest,
            synced_sequence,
            rebase_sequence,
            scope,
        };
        if meta.encode() != text {
            return Err(unusable());
        }
        Ok(meta)
    }
}

fn meta_path(dir: &Path) -> PathBuf {
    dir.join(META_FILE)
}

/// Recovers any pending rebase, then reads and validates workspace metadata.
pub fn read_meta(dir: &Path) -> Result<WorkspaceMeta, Refusal> {
    WorkspaceState::open(dir)?
        .meta()
        .cloned()
        .ok_or_else(unusable)
}

/// Pure read of the workspace metadata: no locks, no recovery, `None` when absent. The U19
/// routing derives the authorized scope from it for every command without taking a lease.
pub fn peek_meta(dir: &Path) -> Result<Option<WorkspaceMeta>, Refusal> {
    state::read(dir)
}

pub(crate) fn write_meta(
    dir: &Path,
    expected: Option<&WorkspaceMeta>,
    next: &WorkspaceMeta,
) -> Result<(), Refusal> {
    WorkspaceState::open(dir)?.compare_and_swap(expected, next)?;
    Ok(())
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
    /// The section/zone scope to authorize (U19). Validated against the published store's
    /// layout before anything is written; a workspace without one cannot edit a sectioned
    /// store, and a sectioned scope on a classic store is refused.
    pub scope: Option<WorkspaceScope>,
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
    /// Unresolved semantic conflicts between the unsent work and accepted history (C04);
    /// empty when none stand. Publication and packaging refuse while this is nonempty.
    pub pending_conflicts: Vec<beamcraft_engine::rebase::Conflict>,
}

fn head_of(
    store: &RevisionStore,
) -> Result<Option<beamcraft_host::revision::RevisionInfo>, Refusal> {
    store.head().map_err(Refusal::from)
}

/// Resolves the exact accepted pin independently of the local published-operation prefix.
/// The current-head path avoids a history search; `sequence_of` bounds a historical lookup by
/// the host's MAX_REVISIONS cap. An unavailable or compacted pin never falls back to local USD.
pub(crate) fn published_base(
    published: &RevisionStore,
    meta: &WorkspaceMeta,
) -> Result<Revision, Refusal> {
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
) -> Result<WorkspaceStatus, Refusal> {
    let revision = match published.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => {
            return Err(Refusal {
                rule_id: None,
                code: ErrorCode::NotFound,
                message: "there is no published revision to fork",
            });
        }
        Err(error) => return Err(error.into()),
    };
    let content = revision.manifest().content();
    // U19: the scope is authorized once, here, against the published layout — never claimed
    // by a later command. A scope on a classic store is refused, and both the section and the
    // shared layer must already exist in the store.
    if let Some(layout) = &request.scope {
        let classic = content
            .entries()
            .iter()
            .any(|e| e.name() == session::pipeline::AUTHORED_LAYER);
        if classic {
            return Err(Refusal {
                rule_id: None,
                code: ErrorCode::InvalidArgument,
                message: "this store is not organized into section scopes",
            });
        }
        crate::scope::authorize(layout, &revision)?;
    }
    let exists = meta_path(dir).exists();
    let meta = if exists {
        let existing = read_meta(dir)?;
        if existing.workspace_id != request.workspace_id || existing.user_id != request.user_id {
            return Err(Refusal {
                rule_id: None,
                code: ErrorCode::DuplicateId,
                message: "the directory is already another workspace",
            });
        }
        if existing.scope != request.scope {
            return Err(Refusal {
                rule_id: None,
                code: ErrorCode::DuplicateId,
                message: "the directory is already a workspace with another authorized scope",
            });
        }
        existing
    } else {
        if head_of(workspace)?.is_some() {
            return Err(Refusal {
                rule_id: None,
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
            rebase_sequence: 0,
            scope: request.scope.clone(),
        };
        write_meta(dir, None, &meta)?;
        meta
    };
    if head_of(workspace)?.is_none() {
        let mut entries = Vec::new();
        for entry in content.entries() {
            let bytes = revision.layer_bytes(entry.name())?;
            workspace.blobs().publish(&bytes).map_err(|e| Refusal {
                rule_id: None,
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
) -> Result<WorkspaceStatus, Refusal> {
    let state = WorkspaceState::open(dir)?;
    let meta = state.meta().cloned().ok_or_else(unusable)?;
    let draft = head_of(workspace)?.ok_or_else(|| integrity("the workspace has no revision"))?;
    if meta.rebase_sequence > draft.sequence() {
        return Err(integrity(
            "workspace rebase boundary is beyond its draft head",
        ));
    }
    let pending_conflicts = conflicts::active(workspace, dir, &meta)?;
    let head =
        head_of(published)?.ok_or_else(|| integrity("the published store has no revision"))?;
    Ok(WorkspaceStatus {
        unsent_revisions: draft.sequence().saturating_sub(meta.synced_sequence),
        base_is_current: head.revision() == &meta.base_revision,
        draft_head: summarize(&draft),
        published_head: summarize(&head),
        pending_conflicts,
        meta,
    })
}

/// The still-unresolved semantic conflicts (C04) that gate publication and packaging.
pub fn pending_conflicts(
    workspace: &RevisionStore,
    dir: &Path,
) -> Result<Vec<beamcraft_engine::rebase::Conflict>, Refusal> {
    let meta = read_meta(dir)?;
    conflicts::active(workspace, dir, &meta)
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

/// The trusted-context re-derivation of the authorized scope at publication (`DV-SCOPE-001`):
/// the unsent work may change only the authorized layers, the derived layer and external
/// sources, and the root composition must be intact (`DV-SCOPE-003`). The metadata scope is
/// the recorded authorization; the manifests decide what actually changed.
fn publish_in_scope(
    draft: &Revision,
    base: &Revision,
    layout: &WorkspaceScope,
) -> Result<(), Refusal> {
    let section = layout.section_layer().map_err(Refusal::from)?;
    let grids = layout.grids_layer().map_err(Refusal::from)?;
    let mut authorized = std::collections::BTreeSet::from([section]);
    if layout.shared {
        authorized.insert(grids);
    }
    let refused = |rule_id: Option<&'static str>, code, message| Refusal {
        rule_id,
        code,
        message,
    };
    let digest_of = |revision: &Revision| {
        revision
            .manifest()
            .content()
            .entries()
            .iter()
            .map(|e| (e.name().to_owned(), (e.digest(), e.role())))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let draft_entries = digest_of(draft);
    let base_entries = digest_of(base);
    let root = draft.manifest().content().root_layer().to_owned();
    let outside = |name: &str, role: LayerRole| {
        // The derived layer follows the owning authored inputs and the root carries the units
        // and schema pins; the root's composition is checked separately below.
        name != session::pipeline::DERIVED_LAYER
            && name != root
            && role != LayerRole::External
            && !authorized.contains(name)
    };
    for (name, (digest, role)) in &base_entries {
        if draft_entries.get(name) != Some(&(*digest, *role)) && outside(name, *role) {
            return Err(refused(
                Some(beamcraft_engine::scope::RULE_SCOPE_AUTHORIZED),
                ErrorCode::PermissionDenied,
                "the unsent work changes layers outside the authorized scope",
            ));
        }
    }
    for (name, (_, role)) in &draft_entries {
        if !base_entries.contains_key(name) && outside(name, *role) {
            return Err(refused(
                Some(beamcraft_engine::scope::RULE_SCOPE_AUTHORIZED),
                ErrorCode::PermissionDenied,
                "the unsent work adds a layer outside the authorized scope",
            ));
        }
    }
    let changed_root =
        base_entries.get(&root).map(|(d, _)| *d) != draft_entries.get(&root).map(|(d, _)| *d);
    if changed_root {
        let base_root = base.layer_bytes(&root)?;
        let draft_root = draft.layer_bytes(&root)?;
        if beamcraft_layer_engine::authored::layer_sublayers(&base_root)?
            != beamcraft_layer_engine::authored::layer_sublayers(&draft_root)?
        {
            return Err(refused(
                Some(beamcraft_engine::scope::RULE_PRESERVE_FOREIGN),
                ErrorCode::ForeignReadOnly,
                "the root layer composition changed",
            ));
        }
    }
    Ok(())
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
) -> Result<Published, Refusal> {
    let mut state = WorkspaceState::open(dir)?;
    let meta = state.meta().cloned().ok_or_else(unusable)?;
    let draft = workspace.open()?;
    let draft_sequence = draft.info().sequence();
    if meta.rebase_sequence > draft_sequence {
        return Err(integrity(
            "workspace rebase boundary is beyond its draft head",
        ));
    }
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
            ..meta.clone()
        };
        state.compare_and_swap(Some(&meta), &next)?;
        drop(state);
        return Ok(Published {
            replayed: true,
            revision: summarize(head.info()),
            status: status(published, workspace, dir)?,
        });
    }
    if draft_sequence <= meta.synced_sequence {
        return Err(Refusal {
            rule_id: None,
            code: ErrorCode::InvalidArgument,
            message: "the workspace has nothing to publish",
        });
    }
    // C04: a recorded semantic conflict refuses every publication until the draft is resolved.
    if !conflicts::active(workspace, dir, &meta)?.is_empty() {
        return Err(conflicts::refusal());
    }
    if head.info().revision() != &meta.base_revision {
        return Err(Refusal {
            rule_id: None,
            code: ErrorCode::BaseRevisionMismatch,
            message: "the published head moved since the workspace was forked",
        });
    }
    // U19: the publisher derives the authorized scope again from the verified manifests and
    // refuses work outside it (DV-SCOPE-001/003).
    if let Some(layout) = &meta.scope {
        publish_in_scope(&draft, &head, layout)?;
    }
    let operations =
        crate::command_evidence::interval(workspace, meta.synced_sequence, draft_sequence)?;
    let source = crate::command_evidence::aggregate(workspace, meta.synced_sequence, &operations)?;
    if source.head.manifest_digest() != draft.info().manifest_digest() {
        return Err(integrity(
            "publication source differs from the verified draft head",
        ));
    }
    // A fully renounced draft carries no modelling work: its content equals its accepted base
    // and a publication of it would ship nothing, so refuse before anything is written.
    if source
        .footprint
        .as_ref()
        .is_some_and(|work| work.reads().is_empty() && work.writes().is_empty())
    {
        return Err(Refusal {
            rule_id: None,
            code: ErrorCode::InvalidArgument,
            message: "the unsent work was renounced; there is nothing to publish",
        });
    }
    let content = draft.manifest().content();
    let published_content = head.manifest().content();
    let mut entries = Vec::new();
    for entry in content.entries() {
        let bytes = draft.layer_bytes(entry.name())?;
        published.blobs().publish(&bytes).map_err(|e| Refusal {
            rule_id: None,
            code: e.code(),
            message: "layer blob could not be stored",
        })?;
        entries.push(LayerEntry::from_bytes(entry.name(), entry.role(), &bytes)?);
    }
    let generation = published_content
        .workspace_generation()
        .checked_add(1)
        .ok_or(Refusal {
            rule_id: None,
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
    let committed = crate::publication::commit_verified_with_evidence(
        published,
        &next,
        &ExpectedBase::Revision(meta.base_revision.clone()),
        evidence,
        hook,
    )?;
    state.compare_and_swap(
        Some(&meta),
        &WorkspaceMeta {
            base_revision: committed.info.revision().clone(),
            base_manifest: committed.info.manifest_digest(),
            synced_sequence: draft_sequence,
            ..meta.clone()
        },
    )?;
    drop(state);
    Ok(Published {
        replayed: committed.outcome == CommitOutcome::Replayed,
        revision: summarize(&committed.info),
        status: status(published, workspace, dir)?,
    })
}
