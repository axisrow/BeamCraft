// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Explicit C06 compensation; all engineering/storage decisions stay in the application API.

use crate::plate_cmd::revision_json;
use beamcraft_cli::workspace::{
    compensation::{CompensationRequest, CompensationResult, compensate_workspace},
    conflicts::conflict_json,
    read_meta,
};
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
        "--store",
        "--workspace-dir",
        "--target-revision",
        "--op-id",
        "--expected-head",
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
            .ok_or_else(|| invalid("a required compensation option is missing"))
    };
    let op_id = OpId::parse(required("--op-id")?).map_err(crate::plate_cmd::context_id_error)?;
    let target_revision = Uuid7::parse(required("--target-revision")?)?;
    let expected_head = Uuid7::parse(required("--expected-head")?)?;
    let actor = options
        .get("--actor-id")
        .map(|text| Uuid7::parse(text))
        .transpose()?;
    let dir = Path::new(required("--workspace-dir")?);
    let at = |path: &Path| {
        BlobStore::new(path)
            .map(RevisionStore::new)
            .map_err(|error| Refusal {
                rule_id: None,
                code: error.code(),
                message: "compensation store is not usable",
            })
    };
    let published = at(Path::new(required("--store")?))?;
    let workspace = at(dir)?;
    let actor = match actor {
        Some(actor) => actor,
        None => read_meta(dir)?.user_id,
    };
    match compensate_workspace(
        &published,
        &workspace,
        dir,
        &CompensationRequest {
            op_id,
            actor,
            target_revision,
            expected_head,
        },
        &mut NoFault,
    )? {
        CompensationResult::Compensated(result) => Ok((
            Json::object([
                (
                    "outcome",
                    Json::text(if result.replayed {
                        "compensation_replayed"
                    } else {
                        "compensated"
                    }),
                ),
                ("revision", revision_json(&result.revision)),
                (
                    "target_revision",
                    Json::text(result.target.revision.as_str()),
                ),
                (
                    "accepted_revision",
                    Json::text(result.accepted.revision.as_str()),
                ),
            ]),
            0,
        )),
        CompensationResult::Conflicted(conflicts) => Ok((
            Json::object([
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
            ]),
            2,
        )),
    }
}
