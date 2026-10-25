// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Read-only D02 drawing preview over one strictly inspected committed source revision.

use crate::plate_cmd::num;
use beamcraft_cli::drawing::archive::SourceArchive;
use beamcraft_cli::drawing::document::store::request_fingerprint;
use beamcraft_cli::drawing::document::{DocumentStore, DocumentWrite, State};
use beamcraft_cli::plate::{PlateError, inspect_revision};
use beamcraft_engine::drawing::projection::{
    self, BodyProjection, DimensionProjection, Projection,
};
use beamcraft_engine::drawing::{
    DEFAULT_SCALE_DENOMINATOR, DimensionSpec, Drawing, DrawingSpec, Plane, SourceModelPin, ViewSpec,
};
use beamcraft_engine::query::feature::FeatureId;
use beamcraft_host::revision::{ExpectedBase, RevisionStore};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{
    ActorId, DimensionId, DocumentVersionId, DrawingId, DrawingViewId, ElementId, OpId, ProjectId,
    RevisionId, Uuid7,
};
use std::io::Write;
use std::{collections::BTreeMap, ffi::OsString};

type Run<T> = Result<T, PlateError>;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Whether the command is the D02 read-only preview.
pub fn handles(command: &OsString) -> bool {
    matches!(
        command.to_str(),
        Some(
            "preview-drawing"
                | "create-drawing"
                | "refresh-drawing"
                | "publish-drawing"
                | "inspect-drawing"
                | "inspect-drawing-versions"
        )
    )
}

/// Runs the headless adapter; successful unresolved geometry is explicit in the JSON result.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok(value) => (value, 0),
        Err(e) => (
            Json::object([
                ("outcome", Json::text("rejected")),
                ("code", Json::text(e.code.as_str())),
                ("message", Json::text(e.message)),
                ("observed_context", Json::Null),
                ("diagnostics", Json::Array(vec![])),
            ]),
            if e.code == ErrorCode::BackendUnavailable {
                3
            } else {
                2
            },
        ),
    }
}

fn point([x, y]: [f64; 2]) -> Run<Json> {
    Ok(Json::Array(vec![num(x)?, num(y)?]))
}
fn segment([a, b]: [[f64; 2]; 2]) -> Run<Json> {
    Ok(Json::Array(vec![point(a)?, point(b)?]))
}

fn geometry_json(result: Projection) -> Run<(Json, Json)> {
    let body = match result.body {
        BodyProjection::Unresolved(reason) => Json::object([
            ("state", Json::text("unresolved")),
            ("reason", Json::text(reason.as_str())),
            ("edges", Json::Array(vec![])),
        ]),
        BodyProjection::Resolved(edges) => Json::object([
            ("state", Json::text("resolved")),
            ("reason", Json::Null),
            (
                "edges",
                Json::Array(
                    edges
                        .into_iter()
                        .map(|edge| {
                            Ok(Json::object([
                                ("feature_id", Json::text(edge.feature.to_string())),
                                ("paper_mm", segment(edge.paper_mm)?),
                            ]))
                        })
                        .collect::<Run<_>>()?,
                ),
            ),
        ]),
    };
    let dimension = match result.dimension {
        DimensionProjection::Unresolved(reason) => Json::object([
            ("state", Json::text("unresolved")),
            ("reason", Json::text(reason.as_str())),
            ("projected_length_mm", Json::Null),
            ("label_mm", Json::Null),
            ("anchors_paper_mm", Json::Null),
            ("line_paper_mm", Json::Null),
        ]),
        DimensionProjection::Resolved(d) => Json::object([
            ("state", Json::text("resolved")),
            ("reason", Json::Null),
            ("projected_length_mm", num(d.projected_length_mm)?),
            ("label_mm", Json::text(d.label_mm)),
            ("anchors_paper_mm", segment(d.anchors_paper_mm)?),
            ("line_paper_mm", segment(d.line_paper_mm)?),
        ]),
    };
    Ok((body, dimension))
}

fn document_options(args: &[OsString]) -> Run<BTreeMap<&str, &str>> {
    if args.len() > 39 {
        return Err(invalid("too many drawing document options"));
    }
    let args = args
        .iter()
        .skip(1)
        .map(|arg| {
            arg.to_str()
                .filter(|text| text.len() <= 4096)
                .ok_or_else(|| invalid("argument must be bounded UTF-8"))
        })
        .collect::<Run<Vec<_>>>()?;
    if !args.len().is_multiple_of(2) {
        return Err(invalid("options must be KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for [key, value] in args.as_chunks::<2>().0 {
        if !matches!(
            *key,
            "--store"
                | "--document-dir"
                | "--expected-head"
                | "--id"
                | "--drawing-id"
                | "--view-id"
                | "--dimension-id"
                | "--anchor-a"
                | "--anchor-b"
                | "--name"
                | "--plane"
                | "--scale"
                | "--origin"
                | "--offset"
                | "--decimals"
                | "--op-id"
                | "--revision-id"
                | "--actor-id"
                | "--version-id"
                | "--model-store"
        ) || options.insert(*key, *value).is_some()
        {
            return Err(invalid("unknown or repeated drawing document option"));
        }
    }
    Ok(options)
}

fn document_drawing(options: &BTreeMap<&str, &str>, source: SourceModelPin) -> Run<Drawing> {
    let required = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| invalid("required drawing option is missing"))
    };
    let id_error = |_| PlateError {
        code: ErrorCode::InvalidId,
        message: "drawing and member identities must be full canonical UUIDv7",
    };
    let id = DrawingId::parse(required("--drawing-id")?).map_err(id_error)?;
    let view_id = DrawingViewId::parse(required("--view-id")?).map_err(id_error)?;
    let dimension_id = DimensionId::parse(required("--dimension-id")?).map_err(id_error)?;
    let member = ElementId::parse(required("--id")?).map_err(id_error)?;
    let anchor = |key| {
        FeatureId::parse(required(key)?).map_err(|error| PlateError {
            code: error.f04_code(),
            message: "anchor must be a full semantic FeatureId, never an index",
        })
    };
    let number = |key, default| {
        options.get(key).map_or(Ok(default), |value| {
            value
                .parse::<f64>()
                .map_err(|_| invalid("invalid drawing number"))
        })
    };
    let origin = options.get("--origin").map_or(Ok([0.0, 0.0]), |text| {
        let mut parts = text.split(',');
        let mut next = || {
            parts
                .next()
                .ok_or_else(|| invalid("origin requires X,Y"))?
                .parse::<f64>()
                .map_err(|_| invalid("invalid origin number"))
        };
        let value = [next()?, next()?];
        if parts.next().is_some() {
            return Err(invalid("origin requires exactly X,Y"));
        }
        Ok(value)
    })?;
    Drawing::admit(DrawingSpec {
        id,
        name: options
            .get("--name")
            .copied()
            .unwrap_or("Member drawing")
            .to_owned(),
        source,
        member,
        view: ViewSpec {
            id: view_id,
            plane: Plane::parse(options.get("--plane").copied().unwrap_or("front"))
                .ok_or_else(|| invalid("plane must be front, top or right"))?,
            scale_denominator: number("--scale", DEFAULT_SCALE_DENOMINATOR)?,
            paper_origin_mm: origin,
        },
        dimension: DimensionSpec {
            id: dimension_id,
            anchors: [anchor("--anchor-a")?, anchor("--anchor-b")?],
            paper_offset_mm: number("--offset", 10.0)?,
            decimal_places: options.get("--decimals").map_or(Ok(0), |value| {
                value
                    .parse::<u8>()
                    .map_err(|_| invalid("invalid decimal count"))
            })?,
        },
    })
    .map_err(Into::into)
}

fn document_json(revision: &beamcraft_cli::drawing::document::DocumentRevision) -> Json {
    Json::object([
        (
            "revision_id",
            Json::text(revision.info.revision().to_string()),
        ),
        ("sequence", Json::text(revision.info.sequence().to_string())),
        (
            "drawing_id",
            Json::text(revision.drawing.spec().id.to_string()),
        ),
        (
            "state",
            Json::text(match revision.state {
                State::Draft => "draft",
                State::Published => "published",
            }),
        ),
        (
            "version_id",
            revision
                .version
                .map_or(Json::Null, |id| Json::text(id.to_string())),
        ),
        (
            "latest_published_version_id",
            revision
                .latest_published
                .map_or(Json::Null, |id| Json::text(id.to_string())),
        ),
        (
            "source_revision_id",
            Json::text(revision.source.pin().revision.to_string()),
        ),
        (
            "source_manifest",
            Json::text(revision.source.pin().manifest.to_string()),
        ),
        ("request_digest", Json::text(revision.request.to_string())),
    ])
}

fn run_document(args: &[OsString]) -> Run<Json> {
    let command = args
        .first()
        .and_then(|value| value.to_str())
        .ok_or_else(|| invalid("drawing command is missing"))?;
    let options = document_options(args)?;
    let required = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| invalid("required drawing document option is missing"))
    };
    match command {
        "create-drawing" | "refresh-drawing" => {
            let model_path = required("--store")?;
            let document_path = required("--document-dir")?;
            let op = OpId::parse(required("--op-id")?)
                .map_err(|_| invalid("operation id must be UUIDv7"))?;
            let revision = Uuid7::parse(required("--revision-id")?)
                .map_err(|_| invalid("revision id must be UUIDv7"))?;
            let actor = ActorId::parse(required("--actor-id")?)
                .map_err(|_| invalid("actor id must be UUIDv7"))?;
            let expected = if command == "create-drawing" {
                ExpectedBase::Initial
            } else {
                ExpectedBase::Revision(
                    beamcraft_host::revision::Uuid7::parse(required("--expected-head")?)
                        .map_err(|_| invalid("expected head must be UUIDv7"))?,
                )
            };
            let source = SourceArchive::capture(std::path::Path::new(model_path))?;
            let drawing = document_drawing(&options, source.pin())?;
            let document_path = std::path::Path::new(document_path);
            let documents = if command == "create-drawing" && !document_path.exists() {
                DocumentStore::create(document_path)?
            } else {
                DocumentStore::open(document_path)?
            };
            let latest_published = if command == "create-drawing" {
                None
            } else if let Some(previous) = documents.query(&op)? {
                previous.latest_published
            } else {
                documents.head()?.latest_published
            };
            let (_, revision) = documents.commit(
                &expected,
                DocumentWrite {
                    revision,
                    operation: op,
                    actor,
                    drawing: &drawing,
                    source: &source,
                    state: State::Draft,
                    version: None,
                    latest_published,
                },
            )?;
            Ok(document_json(&revision))
        }
        "publish-drawing" => {
            let documents = DocumentStore::open(std::path::Path::new(required("--document-dir")?))?;
            let expected = ExpectedBase::Revision(
                beamcraft_host::revision::Uuid7::parse(required("--expected-head")?)
                    .map_err(|_| invalid("expected head must be UUIDv7"))?,
            );
            let op = OpId::parse(required("--op-id")?)
                .map_err(|_| invalid("operation id must be UUIDv7"))?;
            let revision = Uuid7::parse(required("--revision-id")?)
                .map_err(|_| invalid("revision id must be UUIDv7"))?;
            let actor = ActorId::parse(required("--actor-id")?)
                .map_err(|_| invalid("actor id must be UUIDv7"))?;
            let version = DocumentVersionId::parse(required("--version-id")?)
                .map_err(|_| invalid("document version id must be UUIDv7"))?;
            if let Some(previous) = documents.query(&op)? {
                let request = request_fingerprint(
                    &previous.drawing,
                    State::Published,
                    Some(version),
                    Some(version),
                    actor,
                    &expected,
                )?;
                if previous.request == request {
                    return Ok(document_json(&previous));
                }
                return Err(PlateError {
                    code: ErrorCode::OpIdReuse,
                    message: "document operation id was already used for another request",
                });
            }
            let current = documents.head()?;
            let (_, published) = documents.commit(
                &expected,
                DocumentWrite {
                    revision,
                    operation: op,
                    actor,
                    drawing: &current.drawing,
                    source: &current.source,
                    state: State::Published,
                    version: Some(version),
                    latest_published: Some(version),
                },
            )?;
            Ok(document_json(&published))
        }
        "inspect-drawing" => {
            let documents = DocumentStore::open(std::path::Path::new(required("--document-dir")?))?;
            let current = documents.head()?;
            let freshness = match options.get("--model-store") {
                Some(model_path) => {
                    match SourceArchive::capture(std::path::Path::new(model_path)) {
                        Ok(current_model) if current_model.pin() == current.source.pin() => {
                            "current"
                        }
                        Ok(_) => "stale",
                        Err(_) => "unavailable",
                    }
                }
                None => "unknown",
            };
            let mut fields = match document_json(&current) {
                Json::Object(fields) => fields,
                _ => {
                    return Err(PlateError {
                        code: ErrorCode::IntegrityError,
                        message: "drawing inspect response is malformed",
                    });
                }
            };
            fields.insert("freshness".to_owned(), Json::text(freshness));
            Ok(Json::Object(fields))
        }
        "inspect-drawing-versions" => {
            let documents = DocumentStore::open(std::path::Path::new(required("--document-dir")?))?;
            Ok(Json::object([
                ("outcome", Json::text("drawing_versions")),
                (
                    "versions",
                    Json::Array(
                        documents
                            .history()?
                            .iter()
                            .filter(|revision| revision.state == State::Published)
                            .map(document_json)
                            .collect(),
                    ),
                ),
            ]))
        }
        _ => Err(invalid("unknown drawing command")),
    }
}

fn run(args: &[OsString]) -> Run<Json> {
    if args.first().is_some_and(|arg| arg != "preview-drawing") {
        return run_document(args);
    }
    if args.len() > 31 {
        return Err(invalid("too many drawing options"));
    }
    let args = args
        .iter()
        .skip(1)
        .map(|arg| {
            arg.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument must be bounded UTF-8"))
        })
        .collect::<Run<Vec<_>>>()?;
    if !args.len().is_multiple_of(2) {
        return Err(invalid("options must be KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for [key, value] in args.as_chunks::<2>().0 {
        if !matches!(
            *key,
            "--store"
                | "--id"
                | "--drawing-id"
                | "--view-id"
                | "--dimension-id"
                | "--anchor-a"
                | "--anchor-b"
                | "--name"
                | "--plane"
                | "--scale"
                | "--origin"
                | "--offset"
                | "--decimals"
                | "--intent-out"
        ) || options.insert(*key, *value).is_some()
        {
            return Err(invalid("unknown or repeated drawing option"));
        }
    }
    let required = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| invalid("required drawing option is missing"))
    };
    let id_error = |_| PlateError {
        code: ErrorCode::InvalidId,
        message: "drawing and member identities must be full canonical UUIDv7",
    };
    let id = DrawingId::parse(required("--drawing-id")?).map_err(id_error)?;
    let view_id = DrawingViewId::parse(required("--view-id")?).map_err(id_error)?;
    let dimension_id = DimensionId::parse(required("--dimension-id")?).map_err(id_error)?;
    let member = ElementId::parse(required("--id")?).map_err(id_error)?;
    let anchor = |key| {
        FeatureId::parse(required(key)?).map_err(|e| PlateError {
            code: e.f04_code(),
            message: "anchor must be a full semantic FeatureId, never an index",
        })
    };
    let anchors = [anchor("--anchor-a")?, anchor("--anchor-b")?];
    let number = |key, default| {
        options.get(key).map_or(Ok(default), |text| {
            text.parse::<f64>()
                .map_err(|_| invalid("invalid drawing number"))
        })
    };
    let origin = options.get("--origin").map_or(Ok([0.0, 0.0]), |text| {
        let mut parts = text.split(',');
        let mut next = || {
            parts
                .next()
                .ok_or_else(|| invalid("origin requires X,Y"))?
                .parse::<f64>()
                .map_err(|_| invalid("invalid origin number"))
        };
        let value = [next()?, next()?];
        if parts.next().is_some() {
            return Err(invalid("origin requires exactly X,Y"));
        }
        Ok(value)
    })?;
    let view = ViewSpec {
        id: view_id,
        plane: Plane::parse(options.get("--plane").copied().unwrap_or("front"))
            .ok_or_else(|| invalid("plane must be front, top or right"))?,
        scale_denominator: number("--scale", DEFAULT_SCALE_DENOMINATOR)?,
        paper_origin_mm: origin,
    };
    let dimension = DimensionSpec {
        id: dimension_id,
        anchors,
        paper_offset_mm: number("--offset", 10.0)?,
        decimal_places: options.get("--decimals").map_or(Ok(0), |text| {
            text.parse::<u8>()
                .map_err(|_| invalid("invalid decimal count"))
        })?,
    };
    let blobs = BlobStore::new(required("--store")?).map_err(|e| PlateError {
        code: e.code(),
        message: "store root is not usable",
    })?;
    let revision = RevisionStore::new(blobs).open()?;
    let content = revision.manifest().content();
    let source = SourceModelPin {
        project: ProjectId::parse(content.project().as_str()).map_err(id_error)?,
        revision: RevisionId::parse(revision.info().revision().as_str()).map_err(id_error)?,
        manifest: revision.info().manifest_digest(),
        schema: content.schema(),
        generation: content.workspace_generation(),
    };
    let drawing = Drawing::admit(DrawingSpec {
        id,
        name: options
            .get("--name")
            .copied()
            .unwrap_or("Member drawing")
            .into(),
        source,
        member,
        view,
        dimension,
    })?;
    // Admission precedes the strict kernel-derived inspection. The same opened revision supplies
    // both the complete source pin and the snapshot; a concurrent model head never mixes them.
    let inspection = inspect_revision(&revision)?;
    let (body, dimension_geometry) =
        geometry_json(projection::resolve(&drawing, &inspection.snapshot)?)?;
    let spec = drawing.spec();
    let intent = options
        .get("--intent-out")
        .map(|_| beamcraft_cli::drawing::codec::encode(&drawing))
        .transpose()?;
    let result = Json::object([
        ("outcome", Json::text("drawing_preview")),
        ("drawing_id", Json::text(id.to_string())),
        ("view_id", Json::text(view_id.to_string())),
        ("dimension_id", Json::text(dimension_id.to_string())),
        ("name", Json::text(&spec.name)),
        ("member_id", Json::text(member.to_string())),
        ("algorithm", Json::text(projection::ALGORITHM)),
        ("plane", Json::text(spec.view.plane.as_str())),
        ("scale_denominator", num(spec.view.scale_denominator)?),
        ("paper_origin_mm", point(spec.view.paper_origin_mm)?),
        ("paper_offset_mm", num(spec.dimension.paper_offset_mm)?),
        (
            "anchor_ids",
            Json::Array(
                spec.dimension
                    .anchors
                    .iter()
                    .map(|a| Json::text(a.to_string()))
                    .collect(),
            ),
        ),
        (
            "source",
            Json::object([
                ("project_id", Json::text(source.project.to_string())),
                ("revision_id", Json::text(source.revision.to_string())),
                ("manifest", Json::text(source.manifest.to_string())),
                ("schema", Json::text(source.schema.to_string())),
                ("generation", Json::text(source.generation.to_string())),
            ]),
        ),
        (
            "snapshot_token",
            Json::text(inspection.snapshot.token().as_str()),
        ),
        ("body", body),
        ("dimension", dimension_geometry),
        (
            "intent_digest",
            intent
                .as_ref()
                .map_or(Json::Null, |asset| Json::text(asset.digest.to_string())),
        ),
    ]);
    if let (Some(path), Some(asset)) = (options.get("--intent-out"), intent) {
        // Reserve after all admission, source inspection, geometry and serialization succeed.
        // An existing destination is never opened for writing or removed.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| invalid("drawing intent destination exists or cannot be created"))?;
        if file
            .write_all(&asset.bytes)
            .and_then(|()| file.sync_all())
            .is_err()
        {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(PlateError {
                code: ErrorCode::BackendUnavailable,
                message: "drawing intent could not be written",
            });
        }
    }
    Ok(result)
}
