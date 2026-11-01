// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Explicit C04 conflict resolution; all engineering/storage decisions stay in the application
//! API.

use crate::plate_cmd::revision_json;
use beamcraft_cli::workspace::resolve::{ResolveRequest, resolve_workspace};
use beamcraft_host::{
    revision::{NoFault, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::{error::ErrorCode, id::OpId};
use session::Refusal;
use std::{collections::BTreeMap, ffi::OsString, path::Path};

fn invalid(message: &'static str) -> Refusal {
    Refusal {
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

fn run(args: &[OsString]) -> Result<(Json, u8), Refusal> {
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
        "--workspace-dir",
        "--op-id",
        "--expected-draft",
        "--discard",
        "--actor-id",
    ];
    if tail.len() > 10 || !tail.len().is_multiple_of(2) {
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
            .ok_or_else(|| invalid("a required resolve option is missing"))
    };
    let op_id = OpId::parse(required("--op-id")?).map_err(crate::plate_cmd::context_id_error)?;
    let discarded =
        OpId::parse(required("--discard")?).map_err(crate::plate_cmd::context_id_error)?;
    let expected_draft = Uuid7::parse(required("--expected-draft")?)?;
    let dir = Path::new(required("--workspace-dir")?);
    let workspace = BlobStore::new(dir)
        .map(RevisionStore::new)
        .map_err(|error| Refusal {
            rule_id: None,
            code: error.code(),
            message: "resolve store is not usable",
        })?;
    let actor = match options.get("--actor-id") {
        Some(text) => Uuid7::parse(text)?,
        None => beamcraft_cli::workspace::read_meta(dir)?.user_id,
    };
    let result = resolve_workspace(
        &workspace,
        dir,
        &ResolveRequest {
            op_id,
            actor,
            discarded,
            expected_draft,
        },
        &mut NoFault,
    )?;
    Ok((
        Json::object([
            (
                "outcome",
                Json::text(if result.replayed {
                    "workspace_resolve_replayed"
                } else {
                    "workspace_resolved"
                }),
            ),
            ("revision", revision_json(&result.revision)),
            (
                "renounced_operations",
                Json::Array(
                    result
                        .renounced
                        .iter()
                        .map(|op| Json::text(op.to_string()))
                        .collect(),
                ),
            ),
        ]),
        0,
    ))
}
