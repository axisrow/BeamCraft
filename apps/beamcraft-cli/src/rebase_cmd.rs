// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Explicit C03 rebase; all engineering/storage decisions stay in the application API.

use crate::plate_cmd::{id_error, revision_json};
use beamcraft_cli::{
    plate::PlateError,
    workspace::{
        conflicts::conflict_json,
        read_meta,
        rebase::{RebaseRequest, RebaseResult, rebase_workspace},
    },
};
use beamcraft_host::{
    revision::{NoFault, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::{error::ErrorCode, id::OpId};
use std::{collections::BTreeMap, ffi::OsString, path::Path};

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

pub fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok(result) => result,
        Err(error) => crate::report::report(Err(error)),
    }
}

fn run(args: &[OsString]) -> Result<(Json, u8), PlateError> {
    let args = args
        .iter()
        .map(|s| {
            s.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (_, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    let allowed = [
        "--store",
        "--workspace-dir",
        "--op-id",
        "--expected-head",
        "--expected-draft",
        "--actor-id",
    ];
    if tail.len() > 12 || !tail.len().is_multiple_of(2) {
        return Err(invalid("options are bounded KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for [key, value] in tail.as_chunks::<2>().0 {
        if !allowed.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let required = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| invalid("a required rebase option is missing"))
    };
    let op_id = OpId::parse(required("--op-id")?).map_err(id_error)?;
    let expected_head = Uuid7::parse(required("--expected-head")?)?;
    let expected_draft = Uuid7::parse(required("--expected-draft")?)?;
    let actor = options
        .get("--actor-id")
        .map(|text| Uuid7::parse(text))
        .transpose()?;
    let dir = Path::new(required("--workspace-dir")?);
    let at = |path: &Path| {
        BlobStore::new(path)
            .map(RevisionStore::new)
            .map_err(|error| PlateError {
                rule_id: None,
                code: error.code(),
                message: "rebase store is not usable",
            })
    };
    let published = at(Path::new(required("--store")?))?;
    let workspace = at(dir)?;
    let actor = match actor {
        Some(actor) => actor,
        None => read_meta(dir)?.user_id,
    };
    match rebase_workspace(
        &published,
        &workspace,
        dir,
        &RebaseRequest {
            op_id,
            actor,
            expected_head,
            expected_draft,
        },
        &mut NoFault,
    )? {
        RebaseResult::Rebased(result) => Ok((
            Json::object([
                (
                    "outcome",
                    Json::text(if result.replayed {
                        "workspace_rebase_replayed"
                    } else {
                        "workspace_rebased"
                    }),
                ),
                ("revision", revision_json(&result.revision)),
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
            ]),
            0,
        )),
        RebaseResult::Conflicted(conflicts) => Ok((
            Json::object([
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
            ]),
            2,
        )),
    }
}
