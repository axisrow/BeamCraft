// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Read-only D02 drawing preview over one strictly inspected committed source revision.

use crate::plate_cmd::num;
use beamcraft_cli::plate::{PlateError, inspect_revision};
use beamcraft_engine::drawing::projection::{
    self, BodyProjection, DimensionProjection, Projection,
};
use beamcraft_engine::drawing::{
    DEFAULT_SCALE_DENOMINATOR, DimensionSpec, Drawing, DrawingSpec, Plane, SourceModelPin, ViewSpec,
};
use beamcraft_engine::query::feature::FeatureId;
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{
    DimensionId, DrawingId, DrawingViewId, ElementId, ProjectId, RevisionId,
};
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
    command == "drawing-preview"
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

fn run(args: &[OsString]) -> Run<Json> {
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
    Ok(Json::object([
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
    ]))
}
