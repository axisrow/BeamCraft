// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V17 personal representation set commands.
use beamcraft_cli::{attributes, plate::PlateError, representations, saved_views};
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{error::ErrorCode, id::ElementId};
use std::{collections::BTreeMap, ffi::OsString, path::Path};
fn invalid(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
        rule_id: None,
    }
}
pub fn handles(c: &OsString) -> bool {
    [
        "save-representation-set",
        "list-representation-sets",
        "inspect-representation-set",
        "delete-representation-set",
    ]
    .iter()
    .any(|s| c == s)
}
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}
fn read_rule(path: &str) -> Result<Json, PlateError> {
    let b = beamcraft_cli::source_file::read_file(path, 65536).map_err(|code| PlateError {
        code,
        message: "representation rule file is not usable",
        rule_id: None,
    })?;
    beamcraft_json::decode(&b, &Limits::DEFAULT)
        .map_err(|_| invalid("representation rule file is invalid JSON"))
}
fn run(args: &[OsString]) -> Result<Json, PlateError> {
    let args = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument must be bounded UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 24 {
        return Err(invalid("options must be bounded pairs"));
    }
    let allowed: &[&str] = match *command {
        "save-representation-set" => &[
            "--workspace-dir",
            "--id",
            "--name",
            "--rules",
            "--expected-views",
        ],
        "list-representation-sets" => &["--workspace-dir"],
        "inspect-representation-set" => &["--workspace-dir", "--id"],
        "delete-representation-set" => &["--workspace-dir", "--id", "--expected-views"],
        _ => return Err(invalid("unknown representation command")),
    };
    let mut options = BTreeMap::new();
    for [k, v] in tail.as_chunks::<2>().0 {
        if !allowed.contains(k) || options.insert(*k, *v).is_some() {
            return Err(invalid("unknown or repeated representation option"));
        }
    }
    let given = |k: &str| options.get(k).copied();
    let need =
        |k: &str| given(k).ok_or_else(|| invalid("required representation option is missing"));
    let dir = Path::new(need("--workspace-dir")?);
    let mut views = saved_views::read(dir)?;
    if let Some(expected) = given("--expected-views")
        && expected
            != views
                .version
                .map_or_else(|| "none".into(), |d| d.to_string())
    {
        return Err(PlateError {
            code: ErrorCode::StaleWorkspace,
            message: "personal views version differs",
            rule_id: None,
        });
    }
    if *command == "list-representation-sets" {
        return Ok(Json::object([
            ("outcome", Json::text("representation_sets")),
            (
                "representation_sets",
                Json::Array(
                    representations::list(&views)?
                        .iter()
                        .map(representations::json)
                        .collect(),
                ),
            ),
        ]));
    }
    let id = match given("--id") {
        Some(id) => ElementId::parse(id).map_err(crate::plate_cmd::id_error)?,
        None if *command == "save-representation-set" => ElementId::from_uuid(
            beamcraft_host::ids::IdSource::system()
                .next_id()
                .map_err(|_| invalid("representation identity unavailable"))?,
        ),
        _ => return Err(invalid("representation id is required")),
    };
    if *command == "inspect-representation-set" {
        return Ok(Json::object([
            ("outcome", Json::text("representation_set")),
            (
                "representation_set",
                representations::json(&representations::saved(&views, id)?),
            ),
        ]));
    }
    if *command == "delete-representation-set" {
        representations::delete(&mut views, id)?;
        let version = saved_views::write(dir, &views)?;
        return Ok(Json::object([
            ("outcome", Json::text("representation_set_deleted")),
            ("version", Json::text(version.to_string())),
        ]));
    }
    let store = RevisionStore::new(BlobStore::new(dir).map_err(|e| PlateError {
        code: e.code(),
        message: "workspace root is not usable",
        rule_id: None,
    })?);
    let attrs = attributes::read(&store)?;
    let representation = representations::decode(
        id,
        need("--name")?.to_owned(),
        &read_rule(need("--rules")?)?,
    )?;
    representations::save(&mut views, &representation, &attrs)?;
    let stored = representations::saved(&views, id)?;
    let version = saved_views::write(dir, &views)?;
    Ok(Json::object([
        ("outcome", Json::text("representation_set_saved")),
        ("version", Json::text(version.to_string())),
        ("representation_set", representations::json(&stored)),
    ]))
}
