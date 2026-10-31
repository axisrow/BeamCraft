// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Headless queries: `list-feature`, `lookup-feature` and `element-edges` against the verified
//! head's pinned view snapshot. They call the engine queries
//! (`beamcraft_engine::query::feature`) and add no computation: the CLI parses the options, the
//! engine decides and measures.
//!
//! A triangle, vertex or buffer index is never an address: the options `--triangle`,
//! `--vertex-index` and `--instance` are refused with `GEO_TRIANGLE_ADDRESS_FORBIDDEN`. The
//! queries only read: nothing is authored, and an attached foreign reference has no engine body,
//! so it is never a hit.

use crate::plate_cmd::triple;
use beamcraft_cli::plate::{PlateError, inspect};
use beamcraft_engine::error::{ViewCode, ViewError};
use beamcraft_engine::query::feature::{
    FeatureGeometry, FeatureLookup, list_features, lookup_feature,
};
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeMap;
use std::ffi::OsString;

/// A failure of a query: an F04 code, the F06 token when the engine produced it, and a message.
struct Failure {
    rule_id: Option<&'static str>,
    code: ErrorCode,
    reason: Option<&'static str>,
    message: &'static str,
}

impl From<PlateError> for Failure {
    fn from(e: PlateError) -> Self {
        Self {
            rule_id: e.rule_id,
            code: e.code,
            reason: None,
            message: e.message,
        }
    }
}

impl From<ViewError> for Failure {
    fn from(e: ViewError) -> Self {
        Self {
            rule_id: None,
            code: e.f04_code(),
            reason: Some(e.code().as_str()),
            message: "the query was refused",
        }
    }
}

type Run<T> = Result<T, Failure>;

fn invalid(message: &'static str) -> Failure {
    Failure {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        reason: None,
        message,
    }
}

/// Runs one query and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok(json) => (json, 0),
        Err(e) => (
            Json::object([
                ("outcome", Json::text("rejected")),
                ("code", Json::text(e.code.as_str())),
                ("reason", e.reason.map_or(Json::Null, Json::text)),
                ("message", Json::text(e.message)),
                ("observed_context", Json::Null),
                (
                    "diagnostics",
                    Json::Array(crate::plate_cmd::domain_diagnostics(&PlateError {
                        rule_id: e.rule_id,
                        code: e.code,
                        message: e.message,
                    })),
                ),
            ]),
            match e.code {
                ErrorCode::BackendUnavailable => 3,
                _ => 2,
            },
        ),
    }
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
    let own: &[&str] = match *command {
        "lookup-feature" => &["--store", "--feature-id"],
        "list-feature" => &["--store", "--id"],
        "element-edges" => &["--store", "--id"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        // Indices are never addresses (F06 section 5 rule 8).
        if matches!(*key, "--triangle" | "--vertex-index" | "--instance") {
            return Err(ViewError::new(ViewCode::TriangleAddressForbidden).into());
        }
        if !own.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let given = |key: &str| options.get(key).copied();
    // Parse everything before opening the store, so a malformed request reads nothing.
    enum Request {
        Lookup(String),
        List(beamcraft_model::id::ElementId),
        Edges(beamcraft_model::id::ElementId),
    }
    let request = match *command {
        "lookup-feature" => Request::Lookup(
            given("--feature-id")
                .ok_or_else(|| invalid("--feature-id is required"))?
                .to_owned(),
        ),
        _ => {
            let id = beamcraft_model::id::ElementId::parse(
                given("--id").ok_or_else(|| invalid("--id is required"))?,
            )
            .map_err(|_| Failure {
                rule_id: None,
                code: ErrorCode::InvalidId,
                reason: None,
                message: "--id must be a full canonical UUIDv7",
            })?;
            if *command == "element-edges" {
                Request::Edges(id)
            } else {
                Request::List(id)
            }
        }
    };
    let blobs = BlobStore::new(store_root).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let inspection = inspect(&store)?;
    let snapshot = &inspection.snapshot;
    let revision = Json::text(inspection.revision.revision_id.clone());
    let header = |outcome: &str| {
        vec![
            ("outcome".to_owned(), Json::text(outcome)),
            ("revision_id".to_owned(), revision.clone()),
            (
                "snapshot_token".to_owned(),
                Json::text(snapshot.token().as_str()),
            ),
        ]
    };
    let mut fields = match request {
        Request::Lookup(text) => {
            let found = lookup_feature(snapshot, &text)?;
            let mut fields = header("feature");
            fields.extend(lookup_json(&found)?);
            fields
        }
        Request::List(element) => {
            let features = list_features(snapshot, &element)?;
            let mut fields = header("features_listed");
            fields.push(("element_id".into(), Json::text(element.to_string())));
            fields.push((
                "model_manifest".into(),
                Json::text(snapshot.pin().revision().to_string()),
            ));
            fields.push((
                "workspace_generation".into(),
                Json::text(snapshot.pin().generation().to_string()),
            ));
            fields.push((
                "features".into(),
                Json::Array(
                    features
                        .into_iter()
                        .map(|feature| {
                            Json::object([
                                ("feature_id", Json::text(feature.to_string())),
                                ("kind", Json::text(feature.kind().as_str())),
                            ])
                        })
                        .collect(),
                ),
            ));
            fields
        }
        Request::Edges(id) => {
            let edges = beamcraft_engine::query::feature::element_edges(snapshot, &id)?;
            let mut fields = header("element_edges");
            fields.push(("element_id".into(), Json::text(id.to_string())));
            fields.push((
                "edges_world_mm".into(),
                Json::Array(
                    edges
                        .into_iter()
                        .map(|edge| {
                            Ok(Json::Array(
                                edge.into_iter()
                                    .map(crate::plate_cmd::triple)
                                    .collect::<Result<Vec<_>, _>>()?,
                            ))
                        })
                        .collect::<Run<Vec<_>>>()?,
                ),
            ));
            fields
        }
    };
    fields.push((
        "elements".into(),
        Json::uint(u64::try_from(snapshot.len()).unwrap_or(u64::MAX)),
    ));
    Ok(Json::Object(fields.into_iter().collect()))
}

fn geometry_json(g: &FeatureGeometry) -> Result<Json, PlateError> {
    Ok(match g {
        FeatureGeometry::Face {
            normal,
            corners,
            vertex_ids,
        } => Json::object([
            ("normal", triple(*normal)?),
            (
                "corners",
                Json::Array(
                    corners
                        .iter()
                        .map(|c| triple(*c))
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                "vertex_ids",
                Json::Array(
                    vertex_ids
                        .iter()
                        .map(|v| Json::text(v.to_string()))
                        .collect(),
                ),
            ),
        ]),
        FeatureGeometry::Surface {
            normal,
            triangles,
            vertex_ids,
        } => Json::object([
            ("normal", normal.map_or(Ok(Json::Null), triple)?),
            (
                "triangles",
                Json::Array(
                    triangles
                        .iter()
                        .map(|triangle| {
                            Ok(Json::Array(
                                triangle
                                    .iter()
                                    .map(|p| triple(*p))
                                    .collect::<Result<_, PlateError>>()?,
                            ))
                        })
                        .collect::<Result<_, PlateError>>()?,
                ),
            ),
            (
                "vertex_ids",
                Json::Array(
                    vertex_ids
                        .iter()
                        .map(|id| Json::text(id.to_string()))
                        .collect(),
                ),
            ),
        ]),
        FeatureGeometry::Edge { ends, vertex_ids } => Json::object([
            (
                "ends",
                Json::Array(ends.iter().map(|c| triple(*c)).collect::<Result<_, _>>()?),
            ),
            (
                "vertex_ids",
                Json::Array(
                    vertex_ids
                        .iter()
                        .map(|v| Json::text(v.to_string()))
                        .collect(),
                ),
            ),
        ]),
        FeatureGeometry::Vertex { position } => Json::object([("position", triple(*position)?)]),
    })
}

fn lookup_json(found: &FeatureLookup) -> Result<Vec<(String, Json)>, PlateError> {
    Ok(vec![
        ("feature_id".into(), Json::text(found.id.to_string())),
        ("feature_kind".into(), Json::text(found.id.kind().as_str())),
        (
            "element_id".into(),
            Json::text(found.id.element().to_string()),
        ),
        ("local_id".into(), Json::text(found.id.local_id())),
        ("local".into(), geometry_json(&found.local)?),
        ("world".into(), geometry_json(&found.world)?),
    ])
}
