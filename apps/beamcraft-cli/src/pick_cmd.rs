// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V04 headless queries: `pick`, `feature-lookup` and `snap` against the verified head's pinned
//! view snapshot. They call the engine queries (`beamcraft_engine::query::{pick, feature}`) and
//! add no computation: the CLI parses the options, the engine decides and measures.
//!
//! A triangle, vertex or buffer index is never an address: the options `--triangle`,
//! `--vertex-index` and `--instance` are refused with `GEO_TRIANGLE_ADDRESS_FORBIDDEN`. The
//! queries only read: nothing is authored, and an attached foreign reference has no engine body,
//! so it is never a hit.

use crate::plate_cmd::{num, translation, triple};
use beamcraft_cli::plate::{PlateError, inspect};
use beamcraft_engine::error::{ViewCode, ViewError};
use beamcraft_engine::query::feature::{
    FeatureGeometry, FeatureKind, FeatureLookup, lookup_feature,
};
use beamcraft_engine::query::pick::{
    DEFAULT_MAX_HITS, Hit, MAX_T_MM, PickQuery, PointPick, RayPick, SnapKind, SnapQuery, pick, snap,
};
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeMap;
use std::ffi::OsString;

/// A failure of a query: an F04 code, the F06 token when the engine produced it, and a message.
struct Failure {
    code: ErrorCode,
    reason: Option<&'static str>,
    message: &'static str,
}

impl From<PlateError> for Failure {
    fn from(e: PlateError) -> Self {
        Self {
            code: e.code,
            reason: None,
            message: e.message,
        }
    }
}

impl From<ViewError> for Failure {
    fn from(e: ViewError) -> Self {
        Self {
            code: e.f04_code(),
            reason: Some(e.code().as_str()),
            message: "the query was refused",
        }
    }
}

type Run<T> = Result<T, Failure>;

fn invalid(message: &'static str) -> Failure {
    Failure {
        code: ErrorCode::InvalidArgument,
        reason: None,
        message,
    }
}

/// Whether `command` is one of the V04 queries.
pub fn handles(command: &OsString) -> bool {
    command == "pick" || command == "feature-lookup" || command == "snap"
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
                ("diagnostics", Json::Array(vec![])),
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
        "pick" => &[
            "--store",
            "--origin",
            "--direction",
            "--t-max",
            "--include-backfaces",
            "--point",
            "--targets",
            "--radius",
            "--max-hits",
        ],
        "feature-lookup" => &["--store", "--feature-id"],
        "snap" => &["--store", "--point", "--radius", "--kinds", "--max-hits"],
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
    let number = |key: &str, default: f64| -> Run<f64> {
        given(key).map_or(Ok(default), |t| {
            t.parse::<f64>()
                .map_err(|_| invalid("a number is malformed"))
        })
    };
    let max_hits = given("--max-hits").map_or(Ok(DEFAULT_MAX_HITS), |t| {
        t.parse::<u32>()
            .map_err(|_| invalid("--max-hits is a whole number"))
    })?;
    let point = |key: &str| -> Run<[f64; 3]> {
        translation(given(key).ok_or_else(|| invalid("a point is required"))?)
            .map_err(Failure::from)
    };
    // Parse everything before opening the store, so a malformed request reads nothing.
    enum Request {
        Pick(PickQuery),
        Lookup(String),
        Snap(SnapQuery),
    }
    let request = match *command {
        "pick" => {
            let targets = given("--targets")
                .ok_or_else(|| invalid("--targets is required"))?
                .split(',')
                .map(|t| {
                    FeatureKind::parse(t)
                        .ok_or_else(|| invalid("--targets are face, edge or vertex"))
                })
                .collect::<Run<Vec<_>>>()?;
            let radius_mm = number("--radius", 0.0)?;
            match (given("--origin"), given("--point")) {
                (Some(_), None) => Request::Pick(PickQuery::Ray(RayPick {
                    origin_mm: point("--origin")?,
                    direction: point("--direction")?,
                    t_max_mm: number("--t-max", MAX_T_MM)?,
                    targets,
                    radius_mm,
                    max_hits,
                    include_backfaces: match given("--include-backfaces") {
                        None | Some("false") => false,
                        Some("true") => true,
                        Some(_) => return Err(invalid("--include-backfaces is true or false")),
                    },
                })),
                (None, Some(_)) => {
                    if given("--direction").is_some()
                        || given("--t-max").is_some()
                        || given("--include-backfaces").is_some()
                    {
                        return Err(invalid("the point form takes no ray options"));
                    }
                    Request::Pick(PickQuery::Point(PointPick {
                        point_mm: point("--point")?,
                        radius_mm,
                        targets,
                        max_hits,
                    }))
                }
                _ => return Err(invalid("give either --origin and --direction, or --point")),
            }
        }
        "feature-lookup" => Request::Lookup(
            given("--feature-id")
                .ok_or_else(|| invalid("--feature-id is required"))?
                .to_owned(),
        ),
        _ => Request::Snap(SnapQuery {
            point_mm: point("--point")?,
            radius_mm: number("--radius", 0.0)?,
            kinds: given("--kinds")
                .unwrap_or("vertex,axis_end,axis")
                .split(',')
                .map(|t| {
                    SnapKind::parse(t)
                        .ok_or_else(|| invalid("--kinds are vertex, axis_end or axis"))
                })
                .collect::<Run<Vec<_>>>()?,
            max_hits,
        }),
    };
    let blobs = BlobStore::new(store_root).map_err(|e| PlateError {
        code: e.code(),
        message: "store root is not usable",
    })?;
    let inspection = inspect(&RevisionStore::new(blobs))?;
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
        Request::Pick(query) => {
            let result = pick(snapshot, &query)?;
            let mut fields = header("picked");
            fields.push((
                "hits".into(),
                Json::Array(result.hits.iter().map(hit_json).collect::<Run<_>>()?),
            ));
            fields.push(("truncated".into(), Json::Bool(result.truncated)));
            fields.push(("skipped".into(), ids_json(&result.skipped)));
            fields
        }
        Request::Lookup(text) => {
            let found = lookup_feature(snapshot, &text)?;
            let mut fields = header("feature");
            fields.extend(lookup_json(&found)?);
            fields
        }
        Request::Snap(query) => {
            let result = snap(snapshot, &query)?;
            let mut fields = header("snapped");
            let candidates = result
                .candidates
                .iter()
                .map(|c| {
                    Ok(Json::object([
                        ("element_id", Json::text(c.element_id.to_string())),
                        ("kind", Json::text(c.kind.as_str())),
                        (
                            "feature_id",
                            c.feature_id
                                .as_ref()
                                .map_or(Json::Null, |f| Json::text(f.to_string())),
                        ),
                        (
                            "axis_parameter",
                            c.axis_parameter.map_or(Ok(Json::Null), num)?,
                        ),
                        ("point_world_mm", triple(c.point_world_mm)?),
                        ("distance_mm", num(c.distance_mm)?),
                    ]))
                })
                .collect::<Result<Vec<_>, PlateError>>()?;
            fields.push(("candidates".into(), Json::Array(candidates)));
            fields.push(("truncated".into(), Json::Bool(result.truncated)));
            fields.push(("skipped".into(), ids_json(&result.skipped)));
            fields
        }
    };
    fields.push((
        "elements".into(),
        Json::uint(u64::try_from(snapshot.len()).unwrap_or(u64::MAX)),
    ));
    Ok(Json::Object(fields.into_iter().collect()))
}

fn ids_json(ids: &[beamcraft_engine::element::ElementId]) -> Json {
    Json::Array(ids.iter().map(|id| Json::text(id.to_string())).collect())
}

fn hit_json(h: &Hit) -> Run<Json> {
    Ok(Json::object([
        ("element_id", Json::text(h.element_id.to_string())),
        (
            "feature_id",
            h.feature_id
                .as_ref()
                .map_or(Json::Null, |f| Json::text(f.to_string())),
        ),
        ("feature_kind", Json::text(h.feature_kind.as_str())),
        ("t_mm", h.t_mm.map_or(Ok(Json::Null), num)?),
        ("distance_mm", num(h.distance_mm)?),
        ("point_world_mm", triple(h.point_world_mm)?),
        (
            "normal_world",
            h.normal_world.map_or(Ok(Json::Null), triple)?,
        ),
        ("back_facing", Json::Bool(h.back_facing)),
        ("boundary", Json::Bool(h.boundary)),
    ]))
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
