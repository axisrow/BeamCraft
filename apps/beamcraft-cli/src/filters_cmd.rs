// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V14 paged object queries and named personal filter sets.
use beamcraft_cli::{
    attributes,
    filters::{self, SavedFilter},
    plate::PlateError,
    saved_views,
};
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
        "query-elements",
        "save-filter",
        "list-filters",
        "inspect-filter",
        "delete-filter",
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
        message: "filter rule file is not usable",
        rule_id: None,
    })?;
    beamcraft_json::decode(&b, &Limits::DEFAULT)
        .map_err(|_| invalid("filter rule file is invalid JSON"))
}
fn filter_json(f: &SavedFilter) -> Json {
    Json::object([
        ("id", Json::text(f.id.to_string())),
        ("name", Json::text(&f.name)),
        ("rule", f.rule.clone()),
    ])
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
        "query-elements" => &[
            "--store",
            "--rule",
            "--filter",
            "--limit",
            "--after",
            "--expected-model",
        ],
        "save-filter" => &[
            "--workspace-dir",
            "--id",
            "--name",
            "--rule",
            "--expected-views",
        ],
        "list-filters" => &["--workspace-dir"],
        "inspect-filter" => &["--workspace-dir", "--id"],
        "delete-filter" => &["--workspace-dir", "--id", "--expected-views"],
        _ => return Err(invalid("unknown filter command")),
    };
    let mut options = BTreeMap::new();
    for [k, v] in tail.as_chunks::<2>().0 {
        if !allowed.contains(k) || options.insert(*k, *v).is_some() {
            return Err(invalid("unknown or repeated filter option"));
        }
    }
    let given = |k: &str| options.get(k).copied();
    let need = |k: &str| given(k).ok_or_else(|| invalid("required filter option is missing"));
    if *command == "query-elements" {
        let root = need("--store")?;
        let store = RevisionStore::new(BlobStore::new(root).map_err(|e| PlateError {
            code: e.code(),
            message: "store root is not usable",
            rule_id: None,
        })?);
        let attrs = attributes::read(&store)?;
        let pin = attrs
            .revision
            .as_ref()
            .map(|r| r.manifest_digest.to_string());
        if let Some(expected) = given("--expected-model")
            && Some(expected) != pin.as_deref()
        {
            return Err(PlateError {
                code: ErrorCode::StaleWorkspace,
                message: "filter model pin differs",
                rule_id: None,
            });
        }
        let ids = match (given("--rule"), given("--filter")) {
            (Some(path), None) => {
                let rule = filters::decode_rule(&read_rule(path)?, &attrs)?;
                filters::dataset(&attrs)?.query(&rule)?
            }
            (None, Some(id)) => {
                let id = ElementId::parse(id).map_err(crate::plate_cmd::id_error)?;
                filters::matching(&saved_views::read(Path::new(root))?, id, &attrs)?
            }
            _ => return Err(invalid("give exactly one of --rule or --filter")),
        };
        let after = given("--after")
            .map(ElementId::parse)
            .transpose()
            .map_err(crate::plate_cmd::id_error)?;
        let limit = given("--limit")
            .unwrap_or("256")
            .parse::<usize>()
            .map_err(|_| invalid("limit must be an integer"))?;
        if !(1..=1024).contains(&limit) {
            return Err(invalid("limit must be 1..1024"));
        }
        let mut remaining = ids.into_iter().filter(|id| after.is_none_or(|a| *id > a));
        let page = remaining.by_ref().take(limit).collect::<Vec<_>>();
        let more = remaining.next().is_some();
        let next = if more {
            page.last()
                .map_or(Json::Null, |id| Json::text(id.to_string()))
        } else {
            Json::Null
        };
        return Ok(Json::object([
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
        ]));
    }
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
    if *command == "list-filters" {
        return Ok(Json::object([
            ("outcome", Json::text("filters")),
            (
                "filters",
                Json::Array(filters::list(&views)?.iter().map(filter_json).collect()),
            ),
        ]));
    }
    let id = match given("--id") {
        Some(id) => ElementId::parse(id).map_err(crate::plate_cmd::id_error)?,
        None if *command == "save-filter" => ElementId::from_uuid(
            beamcraft_host::ids::IdSource::system()
                .next_id()
                .map_err(|_| invalid("filter identity unavailable"))?,
        ),
        _ => return Err(invalid("filter id is required")),
    };
    if *command == "inspect-filter" {
        return Ok(Json::object([
            ("outcome", Json::text("filter")),
            ("filter", filter_json(&filters::saved(&views, id)?)),
        ]));
    }
    if *command == "delete-filter" {
        filters::delete(&mut views, id)?;
        let version = saved_views::write(dir, &views)?;
        return Ok(Json::object([
            ("outcome", Json::text("filter_deleted")),
            ("version", Json::text(version.to_string())),
        ]));
    }
    let store = RevisionStore::new(BlobStore::new(dir).map_err(|e| PlateError {
        code: e.code(),
        message: "workspace root is not usable",
        rule_id: None,
    })?);
    let attrs = attributes::read(&store)?;
    let filter = SavedFilter {
        id,
        name: need("--name")?.to_owned(),
        rule: read_rule(need("--rule")?)?,
    };
    filters::save(&mut views, filter, &attrs)?;
    let stored = filters::saved(&views, id)?;
    let version = saved_views::write(dir, &views)?;
    Ok(Json::object([
        ("outcome", Json::text("filter_saved")),
        ("version", Json::text(version.to_string())),
        ("filter", filter_json(&stored)),
    ]))
}
