// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V14 headless command: `query-elements`. It calls the engine evaluator through
//! `beamcraft_commands::filters` and prints one JSON object with the same error shape and exit
//! codes as the plate commands.

use beamcraft_commands::filters;
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::{Json, Limits};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use session::Refusal;
use std::collections::BTreeMap;
use std::ffi::OsString;

type Run<T> = Result<T, Refusal>;

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs one filter command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

fn read_rule(path: &str) -> Run<Json> {
    let bytes =
        beamcraft_commands::source_file::read_file(path, 65536).map_err(|code| Refusal {
            rule_id: None,
            code,
            message: "filter rule file is not usable",
        })?;
    beamcraft_json::decode(&bytes, &Limits::DEFAULT)
        .map_err(|_| invalid("filter rule file is invalid JSON"))
}

fn run(args: &[OsString]) -> Run<Json> {
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
    if *command != "query-elements" {
        return Err(invalid("unknown filter command"));
    }
    if !tail.len().is_multiple_of(2) || tail.len() > 10 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let allowed = [
        "--store",
        "--rule",
        "--limit",
        "--after",
        "--expected-model",
    ];
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !allowed.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    if !options.contains_key("--store") {
        return Err(invalid("--store is required"));
    }
    let root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(root).map_err(|e| Refusal {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let model = filters::read(&store)?;
    let pin = model.manifest.map(|d| d.to_string());
    // `none` expects a store without a revision, whose pin is absent.
    let expected = options
        .get("--expected-model")
        .map(|s| (*s != "none").then_some(*s));
    if let Some(expected) = expected
        && expected != pin.as_deref()
    {
        return Err(Refusal {
            rule_id: None,
            code: ErrorCode::StaleWorkspace,
            message: "filter model pin differs",
        });
    }
    let rule_path = options
        .get("--rule")
        .ok_or_else(|| invalid("--rule is required"))?;
    let rule = filters::decode_rule(&read_rule(rule_path)?, &model.schema)?;
    let ids = filters::query(&model, &rule)?;
    let after = options
        .get("--after")
        .map(|s| ElementId::parse(s).map_err(crate::plate_cmd::id_error))
        .transpose()?;
    let limit = options
        .get("--limit")
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|_| invalid("limit must be an integer"))?
        .unwrap_or(256);
    if !(1..=1024).contains(&limit) {
        return Err(invalid("limit must be 1..1024"));
    }
    let mut remaining = ids.into_iter().filter(|id| after.is_none_or(|a| *id > a));
    let page = remaining.by_ref().take(limit).collect::<Vec<_>>();
    let next = if remaining.next().is_some() {
        page.last()
            .map_or(Json::Null, |id| Json::text(id.to_string()))
    } else {
        Json::Null
    };
    Ok(Json::object([
        ("outcome", Json::text("element_query")),
        (
            "elements",
            Json::Array(
                page.into_iter()
                    .map(|id| Json::text(id.to_string()))
                    .collect(),
            ),
        ),
        ("next", next),
        ("model_manifest", pin.map_or(Json::Null, Json::text)),
    ]))
}
