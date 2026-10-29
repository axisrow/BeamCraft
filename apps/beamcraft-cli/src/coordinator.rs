// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C02 adapter: the handler a local coordinator ([`beamcraft_host::coordinator`]) runs for every
//! authenticated request, one at a time.
//!
//! Operations:
//!
//! - `head`: the published head's revision id (or `null` for an empty store).
//! - `compensate` with `workspace`, `op_id`, `target_revision` and `expected_head`: the C06
//!   compensation runs with exactly the local `compensate-workspace` semantics and the
//!   **authenticated actor** as the only trusted actor, so a token compensates only its own
//!   actor's workspace (`PERMISSION_DENIED` otherwise, nothing written). The answer carries
//!   the typed local result (`compensated` / `compensation_replayed`, or a rejected
//!   `DEPENDENCY_CONFLICT` with the bounded report naming every intersecting accepted
//!   operation) plus the current head for information; a compensation only commits the
//!   workspace's draft, and the client still needs an explicit `publish` to move the head.
//!   The same request identity replays across this wire and the local CLI.
//! - `rebase` with `workspace`, `op_id`, `expected_head` and `expected_draft`: the C03 rebase
//!   runs with exactly the local `rebase-workspace` semantics and the **authenticated actor**
//!   as the only trusted actor, so a token rebases only its own actor's workspace
//!   (`PERMISSION_DENIED` otherwise). The answer carries the typed local result
//!   (`workspace_rebased` / `workspace_rebase_replayed`, or a rejected
//!   `DEPENDENCY_CONFLICT` with the bounded report) plus the current head for information; a
//!   rebase never moves the published head, and the client still needs an explicit `publish`
//!   to move it. The same request identity replays across this wire and the local CLI.
//! - `publish` with `workspace` (a directory name under the coordinator's workspaces root),
//!   `changeset` (the digest `create-changeset` printed) and `expected_head` (the revision the
//!   client believes is the published head). The expected head must be the changeset's base;
//!   then C01's receipt runs with the **authenticated actor** as the trusted caller, so a token
//!   publishes only its own actor's changesets (`PERMISSION_DENIED` otherwise). Publication is
//!   fast-forward only: when the head is no longer the expected one the answer is
//!   `BASE_REVISION_MISMATCH` with the current head, and nothing is written. The same payload
//!   replays. The current head in an answer is re-read after the decision and marked `known` or
//!   `unavailable`; a failed re-read never changes the decision itself.
//!
//! The queue makes publications of one coordinator strictly sequential, so two concurrent
//! publications from the same base accept exactly one; a commit to the published store from
//! outside the coordinator is still caught by the store's own expected-base check (U06), so an
//! accepted head is never overwritten either way.
//!
//! A workspace is named, never given as a path: the name must be one directory component and its
//! canonical path must stay inside the canonical workspaces root (`PERMISSION_DENIED` for a
//! symbolic link or name that leaves it).

use crate::changeset::{apply_changeset, read_changeset};
use crate::plate::PlateError;
use crate::workspace::{
    compensation::{CompensationRequest, CompensationResult, compensate_workspace},
    conflicts::conflict_json,
    rebase::{RebaseRequest, RebaseResult, rebase_workspace},
};
use beamcraft_host::coordinator::{Handler, Request, rejection};
use beamcraft_host::revision::{NoFault, RevisionError, RevisionStore, Uuid7};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::Json;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::OpId;
use std::path::{Path, PathBuf};

/// The publish handler of one coordinator.
pub struct PublishHandler {
    published: RevisionStore,
    /// The workspaces root as given: stores are opened under it, as every other command opens
    /// the paths it is given (the store refuses links on that path itself).
    root: PathBuf,
    /// Its canonical form, for the containment check only.
    canonical_root: PathBuf,
}

fn refused(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError::new(code, message)
}

impl PublishHandler {
    /// A handler publishing into `published` from workspaces under `workspaces_root`.
    pub fn new(published: RevisionStore, workspaces_root: &Path) -> Result<Self, PlateError> {
        let canonical_root = workspaces_root.canonicalize().map_err(|_| {
            refused(
                ErrorCode::NotFound,
                "the workspaces root is not a directory",
            )
        })?;
        if !canonical_root.is_dir() {
            return Err(refused(
                ErrorCode::NotFound,
                "the workspaces root is not a directory",
            ));
        }
        Ok(Self {
            published,
            root: std::path::absolute(workspaces_root).map_err(|_| {
                refused(
                    ErrorCode::NotFound,
                    "the workspaces root is not a directory",
                )
            })?,
            canonical_root,
        })
    }

    /// The directory of a named workspace, contained in the root.
    fn workspace(&self, name: &str) -> Result<PathBuf, PlateError> {
        let simple = (1..=64).contains(&name.len())
            && !name.starts_with('.')
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
        if !simple {
            return Err(refused(
                ErrorCode::InvalidArgument,
                "a workspace is named by one directory component",
            ));
        }
        let dir = self.root.join(name);
        let canonical = dir
            .canonicalize()
            .map_err(|_| refused(ErrorCode::NotFound, "the workspace does not exist"))?;
        if !canonical.starts_with(&self.canonical_root)
            || canonical == self.canonical_root
            || !canonical.is_dir()
        {
            return Err(refused(
                ErrorCode::PermissionDenied,
                "the workspace is outside the workspaces root",
            ));
        }
        // The canonical path only proves containment; on Windows it is a verbatim `\\?\` path,
        // which the store's link checks do not walk like the given one.
        Ok(dir)
    }

    fn head(&self) -> Result<Option<String>, PlateError> {
        match self.published.open() {
            Ok(revision) => Ok(Some(revision.info().revision().as_str().to_owned())),
            Err(RevisionError::NoRevision) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn dispatch(&mut self, request: Request) -> Result<Json, PlateError> {
        let text = |key: &str| -> Result<&str, PlateError> {
            match request.fields.get(key) {
                Some(Json::String(s)) => Ok(s.as_str()),
                _ => Err(refused(
                    ErrorCode::InvalidArgument,
                    "a required field is missing",
                )),
            }
        };
        let allowed: &[&str] = match request.op.as_str() {
            "head" => &[],
            "compensate" => &["workspace", "op_id", "target_revision", "expected_head"],
            "rebase" => &["workspace", "op_id", "expected_head", "expected_draft"],
            "publish" => &["workspace", "changeset", "expected_head"],
            _ => {
                return Err(refused(
                    ErrorCode::UnsupportedCommand,
                    "unknown coordinator operation",
                ));
            }
        };
        if request
            .fields
            .keys()
            .any(|k| !allowed.contains(&k.as_str()))
        {
            return Err(refused(
                ErrorCode::UnsupportedProperty,
                "unknown request field",
            ));
        }
        let head_json = |head: &Option<String>| head.clone().map_or(Json::Null, Json::text);
        // After C01 decided, the head is re-read for the client's information only: a failed
        // re-read must not turn a known publication (or refusal) into a different answer.
        let current = |this: &Self| match this.head() {
            Ok(head) => (head_json(&head), Json::text("known")),
            Err(_) => (Json::Null, Json::text("unavailable")),
        };
        if request.op == "head" {
            return Ok(Json::object([
                ("outcome", Json::text("head")),
                ("revision_id", head_json(&self.head()?)),
            ]));
        }
        if request.op == "compensate" {
            let invalid = |message: &'static str| refused(ErrorCode::InvalidArgument, message);
            let op_id = OpId::parse(text("op_id")?)
                .map_err(|_| invalid("op_id is not a usable operation identity"))?;
            let target_revision = Uuid7::parse(text("target_revision")?)?;
            let expected_head = Uuid7::parse(text("expected_head")?)?;
            let dir = self.workspace(text("workspace")?)?;
            let blobs = BlobStore::new(&dir).map_err(|e| PlateError {
                rule_id: None,
                code: e.code(),
                message: "workspace store is not usable",
            })?;
            let workspace = RevisionStore::new(blobs);
            let request = CompensationRequest {
                op_id,
                // The token is the only trusted actor; a request cannot name one (C02).
                actor: Uuid7::parse(&request.actor.to_string())
                    .map_err(|_| invalid("the authenticated actor is not a canonical UUIDv7"))?,
                target_revision,
                expected_head,
            };
            return match compensate_workspace(
                &self.published,
                &workspace,
                &dir,
                &request,
                &mut NoFault,
            ) {
                Ok(CompensationResult::Compensated(result)) => {
                    let (head, status) = current(self);
                    Ok(Json::object([
                        (
                            "outcome",
                            Json::text(if result.replayed {
                                "compensation_replayed"
                            } else {
                                "compensated"
                            }),
                        ),
                        (
                            "revision_id",
                            Json::text(result.revision.revision_id.as_str()),
                        ),
                        (
                            "target_revision",
                            Json::text(result.target.revision.as_str()),
                        ),
                        (
                            "accepted_revision",
                            Json::text(result.accepted.revision.as_str()),
                        ),
                        ("current_head", head),
                        ("current_head_status", status),
                    ]))
                }
                Ok(CompensationResult::Conflicted(conflicts)) => {
                    let (head, status) = current(self);
                    Ok(Json::object([
                        ("outcome", Json::text("rejected")),
                        ("code", Json::text(ErrorCode::DependencyConflict.as_str())),
                        (
                            "message",
                            Json::text(
                                "compensation requires the target's state to still be current; \
                                 every later intersecting accepted operation is named",
                            ),
                        ),
                        (
                            "diagnostics",
                            Json::Array(conflicts.iter().map(conflict_json).collect()),
                        ),
                        ("current_head", head),
                        ("current_head_status", status),
                    ]))
                }
                Err(e) if e.code == ErrorCode::BaseRevisionMismatch => {
                    let (head, status) = current(self);
                    Ok(Json::object([
                        ("outcome", Json::text("rejected")),
                        ("code", Json::text(e.code.as_str())),
                        ("message", Json::text(e.message)),
                        ("current_head", head),
                        ("current_head_status", status),
                    ]))
                }
                Err(e) => Err(e),
            };
        }
        if request.op == "rebase" {
            let invalid = |message: &'static str| refused(ErrorCode::InvalidArgument, message);
            let op_id = OpId::parse(text("op_id")?)
                .map_err(|_| invalid("op_id is not a usable operation identity"))?;
            let expected_head = Uuid7::parse(text("expected_head")?)?;
            let expected_draft = Uuid7::parse(text("expected_draft")?)?;
            let dir = self.workspace(text("workspace")?)?;
            let blobs = BlobStore::new(&dir).map_err(|e| PlateError {
                rule_id: None,
                code: e.code(),
                message: "workspace store is not usable",
            })?;
            let workspace = RevisionStore::new(blobs);
            let request = RebaseRequest {
                op_id,
                // The token is the only trusted actor; a request cannot name one (C02).
                actor: Uuid7::parse(&request.actor.to_string())
                    .map_err(|_| invalid("the authenticated actor is not a canonical UUIDv7"))?,
                expected_head,
                expected_draft,
            };
            return match rebase_workspace(&self.published, &workspace, &dir, &request, &mut NoFault)
            {
                Ok(RebaseResult::Rebased(result)) => {
                    let (head, status) = current(self);
                    Ok(Json::object([
                        (
                            "outcome",
                            Json::text(if result.replayed {
                                "workspace_rebase_replayed"
                            } else {
                                "workspace_rebased"
                            }),
                        ),
                        (
                            "revision_id",
                            Json::text(result.revision.revision_id.as_str()),
                        ),
                        (
                            "base_revision",
                            Json::text(result.accepted.revision.as_str()),
                        ),
                        (
                            "base_manifest",
                            Json::text(result.accepted.manifest.to_string()),
                        ),
                        (
                            "retained_operations",
                            Json::text(result.retained_operations.to_string()),
                        ),
                        ("current_head", head),
                        ("current_head_status", status),
                    ]))
                }
                Ok(RebaseResult::Conflicted(conflicts)) => {
                    let (head, status) = current(self);
                    Ok(Json::object([
                        ("outcome", Json::text("rejected")),
                        ("code", Json::text(ErrorCode::DependencyConflict.as_str())),
                        (
                            "message",
                            Json::text("rebase requires explicit resolution of semantic conflicts"),
                        ),
                        (
                            "diagnostics",
                            Json::Array(conflicts.iter().map(conflict_json).collect()),
                        ),
                        ("current_head", head),
                        ("current_head_status", status),
                    ]))
                }
                Err(e) if e.code == ErrorCode::BaseRevisionMismatch => {
                    let (head, status) = current(self);
                    Ok(Json::object([
                        ("outcome", Json::text("rejected")),
                        ("code", Json::text(e.code.as_str())),
                        ("message", Json::text(e.message)),
                        ("current_head", head),
                        ("current_head_status", status),
                    ]))
                }
                Err(e) => Err(e),
            };
        }
        let dir = self.workspace(text("workspace")?)?;
        let digest = text("changeset")?
            .parse::<ContentDigest>()
            .map_err(|_| refused(ErrorCode::InvalidArgument, "changeset is not a SHA-256"))?;
        let expected = text("expected_head")?;
        let blobs = BlobStore::new(&dir).map_err(|e| PlateError {
            rule_id: None,
            code: e.code(),
            message: "workspace store is not usable",
        })?;
        let workspace = RevisionStore::new(blobs);
        let stored = read_changeset(&workspace, &digest)?;
        if stored.changeset.base_revision.to_string() != expected {
            return Err(refused(
                ErrorCode::InvalidArgument,
                "the expected head is not the changeset's base",
            ));
        }
        match apply_changeset(
            &self.published,
            &workspace,
            &dir,
            &digest,
            &request.actor,
            &mut NoFault,
        ) {
            Ok(applied) => {
                let (head, status) = current(self);
                Ok(Json::object([
                    (
                        "outcome",
                        Json::text(if applied.replayed {
                            "changeset_replayed"
                        } else {
                            "changeset_applied"
                        }),
                    ),
                    ("revision_id", Json::text(applied.revision.revision_id)),
                    ("changeset", Json::text(applied.digest.to_string())),
                    ("workspace_synced", Json::Bool(applied.workspace_synced)),
                    ("current_head", head),
                    ("current_head_status", status),
                ]))
            }
            Err(e) if e.code == ErrorCode::BaseRevisionMismatch => {
                let (head, status) = current(self);
                Ok(Json::object([
                    ("outcome", Json::text("rejected")),
                    ("code", Json::text(e.code.as_str())),
                    ("message", Json::text(e.message)),
                    ("current_head", head),
                    ("current_head_status", status),
                ]))
            }
            Err(e) => Err(e),
        }
    }
}

impl Handler for PublishHandler {
    fn handle(&mut self, request: Request) -> Json {
        self.dispatch(request)
            .unwrap_or_else(|e| rejection(e.code, e.message))
    }
}
