// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C01 headless commands: `create-changeset`, `inspect-changeset` and `apply-changeset`. They call
//! `beamcraft_cli::changeset` and print one JSON object with the same error shape and exit codes
//! as the plate commands.

use crate::plate_cmd::revision_json;
use beamcraft_cli::changeset::{
    CreateChangeset, StoredChangeset, apply_changeset, create_changeset, read_changeset,
};
use beamcraft_cli::plate::PlateError;
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::changeset::{Access, DependencyTarget};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ActorId, OpId, RevisionId};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

type Run<T> = Result<T, PlateError>;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs one changeset command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    let mut candidates = Vec::new();
    crate::report::report_with(run(args, &mut candidates), candidates)
}

fn store_at(path: &Path, message: &'static str) -> Run<RevisionStore> {
    let blobs = BlobStore::new(path).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message,
    })?;
    Ok(RevisionStore::new(blobs))
}

fn run(args: &[OsString], candidates: &mut Vec<Json>) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    let allowed: &[&str] = match *command {
        "create-changeset" => &[
            "--store",
            "--workspace-dir",
            "--changeset-id",
            "--revision-id",
            "--actor-id",
        ],
        "inspect-changeset" => &["--workspace-dir", "--changeset"],
        "apply-changeset" => &["--store", "--workspace-dir", "--changeset", "--actor-id"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 20 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !allowed.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let given = |key: &str| options.get(key).copied();
    let dir =
        Path::new(given("--workspace-dir").ok_or_else(|| invalid("--workspace-dir is required"))?);
    let workspace = store_at(dir, "workspace directory is not usable")?;
    let digest = || -> Run<ContentDigest> {
        given("--changeset")
            .ok_or_else(|| invalid("--changeset is required"))?
            .parse()
            .map_err(|_| invalid("a digest is not 64 lower-case hexadecimal characters"))
    };
    if *command == "inspect-changeset" {
        let stored = read_changeset(&workspace, &digest()?)?;
        return Ok(Json::object([
            ("outcome", Json::text("changeset")),
            ("changeset", changeset_json(&stored)),
        ]));
    }
    let published_root = given("--store").ok_or_else(|| invalid("--store is required"))?;
    let same = matches!(
        (std::fs::canonicalize(dir), std::fs::canonicalize(published_root)),
        (Ok(a), Ok(b)) if a == b
    );
    if same {
        return Err(invalid("the workspace directory is the published store"));
    }
    let published = store_at(Path::new(published_root), "store root is not usable")?;
    let actor = given("--actor-id")
        .map(|t| ActorId::parse(t).map_err(crate::plate_cmd::context_id_error))
        .transpose()?;
    if *command == "create-changeset" {
        let mut source = IdSource::system();
        let mut fresh = || {
            source.next_id().map_err(|_| PlateError {
                rule_id: None,
                code: ErrorCode::BackendUnavailable,
                message: "no identity could be generated",
            })
        };
        let changeset_id = match given("--changeset-id") {
            Some(text) => OpId::parse(text).map_err(crate::plate_cmd::context_id_error)?,
            None => OpId::from_uuid(fresh()?),
        };
        let revision_id = match given("--revision-id") {
            Some(text) => RevisionId::parse(text).map_err(crate::plate_cmd::context_id_error)?,
            None => RevisionId::from_uuid(fresh()?),
        };
        let stored = match create_changeset(
            &published,
            &workspace,
            dir,
            &CreateChangeset {
                changeset_id,
                revision_id,
                actor,
            },
        ) {
            Ok(stored) => stored,
            Err(error) if error.code == ErrorCode::DependencyConflict => {
                // C04: the refusal names every pending conflict in its diagnostics.
                candidates.extend(
                    beamcraft_cli::workspace::pending_conflicts(&workspace, dir)?
                        .iter()
                        .map(beamcraft_cli::workspace::conflicts::conflict_json),
                );
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        return Ok(Json::object([
            ("outcome", Json::text("changeset_created")),
            ("changeset", changeset_json(&stored)),
        ]));
    }
    // The trusted caller context; there is no authentication before C02.
    let actor = actor.ok_or_else(|| invalid("--actor-id is required"))?;
    let applied = apply_changeset(
        &published,
        &workspace,
        dir,
        &digest()?,
        &actor,
        &mut NoFault,
    )?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if applied.replayed {
                "changeset_replayed"
            } else {
                "changeset_applied"
            }),
        ),
        ("digest", Json::text(applied.digest.to_string())),
        ("workspace_synced", Json::Bool(applied.workspace_synced)),
        ("revision", revision_json(&applied.revision)),
    ]))
}

fn changeset_json(stored: &StoredChangeset) -> Json {
    let c = &stored.changeset;
    let digest_text = |d: &ContentDigest| Json::text(d.to_string());
    Json::object([
        ("digest", digest_text(&stored.digest)),
        ("changeset_id", Json::text(c.changeset_id.to_string())),
        ("project_id", Json::text(c.project.to_string())),
        ("workspace_id", Json::text(c.workspace.to_string())),
        ("actor_id", Json::text(c.actor.to_string())),
        ("base_revision", Json::text(c.base_revision.to_string())),
        ("base_manifest", digest_text(&c.base_manifest)),
        ("revision_id", Json::text(c.revision.to_string())),
        (
            "operations",
            Json::Array(
                c.operations
                    .iter()
                    .map(|o| {
                        Json::object([
                            ("op_id", Json::text(o.op_id.to_string())),
                            ("result", digest_text(&o.result)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "dependencies",
            Json::Array(
                c.dependencies
                    .iter()
                    .map(|d| {
                        let (kind, target) = match d.target {
                            DependencyTarget::Element(id) => ("element", id.to_string()),
                            DependencyTarget::Definition(digest) => {
                                ("definition", digest.to_string())
                            }
                            DependencyTarget::AttrSchema => (
                                "schema",
                                beamcraft_layer_engine::authored::ATTR_SCHEMA_IDENTITY.to_string(),
                            ),
                        };
                        Json::object([
                            ("target_kind", Json::text(kind)),
                            ("target", Json::text(target)),
                            ("group", Json::text(d.group.as_str())),
                            (
                                "access",
                                Json::text(match d.access {
                                    Access::Read => "read",
                                    Access::Write => "write",
                                }),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "layers",
            Json::Array(
                c.layers
                    .iter()
                    .map(|l| {
                        Json::object([
                            ("name", Json::text(l.name.clone())),
                            ("base", l.base.as_ref().map_or(Json::Null, digest_text)),
                            ("digest", digest_text(&l.digest)),
                            ("size", Json::text(l.size.to_string())),
                            ("changed", Json::Bool(l.changed())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("root_layer", Json::text(c.root_layer.clone())),
        (
            "assets",
            Json::Array(c.assets.iter().map(|a| digest_text(&a.digest)).collect()),
        ),
    ])
}
