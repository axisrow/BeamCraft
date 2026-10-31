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
    CutSide, DEFAULT_SCALE_DENOMINATOR, DetailSpec, DimensionSpec, DraftingGeometry, DraftingSpec,
    Drawing, DrawingSpec, LayerSpec, LineType, Plane, SectionSpec, SheetFrame, SheetSpec,
    SourceModelPin, TitleBlock, ViewSpec, print, sheet, svg,
};
use beamcraft_engine::query::feature::FeatureId;
use beamcraft_host::revision::{ExpectedBase, RevisionStore};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{
    ActorId, DimensionId, DocumentVersionId, DraftingEntityId, DrawingId, DrawingViewId, ElementId,
    OpId, ProjectId, RevisionId, Uuid7,
};
use beamcraft_model::sheet::Sheet;
use std::io::Write;
use std::{collections::BTreeMap, ffi::OsString};

type Run<T> = Result<T, PlateError>;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
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
                (
                    "diagnostics",
                    Json::Array(crate::plate_cmd::domain_diagnostics(&e)),
                ),
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

/// Parses one exactly-two-component paper origin.
fn parse_origin(text: &str) -> Run<[f64; 2]> {
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

/// Single-value options plus the repeatable `--drafting`, `--layer`, `--placed-view`,
/// `--placed-section` and `--placed-detail` values, in caller order.
struct DocumentOptions<'a> {
    options: BTreeMap<&'a str, &'a str>,
    drafting: Vec<&'a str>,
    layers: Vec<&'a str>,
    views: Vec<&'a str>,
    sections: Vec<&'a str>,
    details: Vec<&'a str>,
}

fn document_options(args: &[OsString]) -> Run<DocumentOptions<'_>> {
    if args.len() > 159 {
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
    let mut drafting = Vec::new();
    let mut layers = Vec::new();
    let mut views = Vec::new();
    let mut sections = Vec::new();
    let mut details = Vec::new();
    for [key, value] in args.as_chunks::<2>().0 {
        match *key {
            "--drafting" => drafting.push(*value),
            "--layer" => layers.push(*value),
            "--placed-view" => views.push(*value),
            "--placed-section" => sections.push(*value),
            "--placed-detail" => details.push(*value),
            _ if matches!(
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
                    | "--sheet-size"
                    | "--sheet-frame"
                    | "--sheet-project"
                    | "--sheet-author"
                    | "--sheet-remove"
                    | "--op-id"
                    | "--revision-id"
                    | "--actor-id"
                    | "--version-id"
                    | "--model-store"
                    | "--out"
                    | "--format"
                    | "--sheet"
            ) && options.insert(*key, *value).is_none() => {}
            _ => return Err(invalid("unknown or repeated drawing document option")),
        }
    }
    Ok(DocumentOptions {
        options,
        drafting,
        layers,
        views,
        sections,
        details,
    })
}

/// Parses repeatable `--drafting UUID:kind:x,y:layer:linetype[:payload]` values into drawn
/// objects. The layer and stroke segments are required; the sixth segment is the string payload
/// (`text`, `pattern` or `symbol`) the kind requires. Engine admission still owns every bound;
/// this only builds the typed values.
fn drafting_specs(values: &[&str]) -> Run<Vec<DraftingSpec>> {
    values
        .iter()
        .map(|value| {
            // The first five segments never contain a colon, so the split is exact even for a
            // text payload holding colons: the payload is the whole remainder.
            let mut parts = value.splitn(6, ':');
            let id = parts.next().ok_or_else(|| {
                invalid("drafting spec requires UUID:kind:x,y:layer:linetype[:payload]")
            })?;
            let kind = parts.next().ok_or_else(|| {
                invalid("drafting spec requires UUID:kind:x,y:layer:linetype[:payload]")
            })?;
            let coords = parts.next().ok_or_else(|| {
                invalid("drafting spec requires UUID:kind:x,y:layer:linetype[:payload]")
            })?;
            let layer = parts.next().ok_or_else(|| {
                invalid("drafting spec requires UUID:kind:x,y:layer:linetype[:payload]")
            })?;
            let line_type = parts.next().ok_or_else(|| {
                invalid("drafting spec requires UUID:kind:x,y:layer:linetype[:payload]")
            })?;
            let payload = parts.next();
            let id = DraftingEntityId::parse(id)
                .map_err(|_| invalid("drafting identity must be a full canonical UUIDv7"))?;
            let layer = DraftingEntityId::parse(layer)
                .map_err(|_| invalid("drafting layer must be a full canonical UUIDv7"))?;
            let line_type = LineType::parse(line_type).ok_or_else(|| {
                invalid("drafting line type must be continuous, dashed, dotted or dashdot")
            })?;
            let coords = coords
                .split(',')
                .map(|number| {
                    number
                        .parse::<f64>()
                        .map_err(|_| invalid("invalid drafting coordinate"))
                })
                .collect::<Run<Vec<f64>>>()?;
            let geometry =
                DraftingGeometry::from_parts(kind, &coords, payload).ok_or_else(|| {
                    invalid(
                        "drafting kind must be line, polyline, arc, circle, rectangle, text, \
                         hatch, symbol or free_dimension with matching coordinates and payload",
                    )
                })?;
            Ok(DraftingSpec {
                id,
                geometry,
                layer,
                line_type,
            })
        })
        .collect()
}

/// Parses repeatable `--layer UUID:name:visible` values into document layers, where the visible
/// flag is `on` or `off`. Engine admission still owns every bound; this only builds the values.
fn layer_specs(values: &[&str]) -> Run<Vec<LayerSpec>> {
    values
        .iter()
        .map(|value| {
            let mut parts = value.splitn(3, ':');
            let id = parts
                .next()
                .ok_or_else(|| invalid("layer requires UUID:name:on|off"))?;
            let name = parts
                .next()
                .ok_or_else(|| invalid("layer requires UUID:name:on|off"))?;
            let visible = parts
                .next()
                .ok_or_else(|| invalid("layer requires UUID:name:on|off"))?;
            Ok(LayerSpec {
                id: DraftingEntityId::parse(id)
                    .map_err(|_| invalid("layer identity must be a full canonical UUIDv7"))?,
                name: name.to_owned(),
                visible: match visible {
                    "on" => true,
                    "off" => false,
                    _ => return Err(invalid("layer visibility must be on or off")),
                },
            })
        })
        .collect()
}

/// Parses repeatable `--placed-view UUID:plane:scale:x,y` values into placed views. Engine
/// admission still owns every bound; this only builds the typed values.
fn placed_view_specs(values: &[&str]) -> Run<Vec<ViewSpec>> {
    values
        .iter()
        .map(|value| {
            let mut parts = value.splitn(4, ':');
            let id = parts
                .next()
                .ok_or_else(|| invalid("placed view requires UUID:plane:scale:x,y"))?;
            let plane = parts
                .next()
                .ok_or_else(|| invalid("placed view requires UUID:plane:scale:x,y"))?;
            let scale = parts
                .next()
                .ok_or_else(|| invalid("placed view requires UUID:plane:scale:x,y"))?;
            let origin = parts
                .next()
                .ok_or_else(|| invalid("placed view requires UUID:plane:scale:x,y"))?;
            Ok(ViewSpec {
                id: DrawingViewId::parse(id)
                    .map_err(|_| invalid("placed view identity must be a full canonical UUIDv7"))?,
                plane: Plane::parse(plane)
                    .ok_or_else(|| invalid("placed view plane must be front, top or right"))?,
                scale_denominator: scale
                    .parse::<f64>()
                    .map_err(|_| invalid("invalid placed view scale"))?,
                paper_origin_mm: parse_origin(origin)?,
            })
        })
        .collect()
}

/// Parses repeatable `--placed-section UUID:plane:cut:side:scale:x,y` values into section
/// views. Engine admission still owns every bound; this only builds the typed values.
fn placed_section_specs(values: &[&str]) -> Run<Vec<SectionSpec>> {
    values
        .iter()
        .map(|value| {
            let mut parts = value.splitn(6, ':');
            let fail = || invalid("placed section requires UUID:plane:cut:side:scale:x,y");
            let id = parts.next().ok_or_else(fail)?;
            let plane = parts.next().ok_or_else(fail)?;
            let cut = parts.next().ok_or_else(fail)?;
            let side = parts.next().ok_or_else(fail)?;
            let scale = parts.next().ok_or_else(fail)?;
            let origin = parts.next().ok_or_else(fail)?;
            Ok(SectionSpec {
                id: DrawingViewId::parse(id).map_err(|_| {
                    invalid("placed section identity must be a full canonical UUIDv7")
                })?,
                plane: Plane::parse(plane)
                    .ok_or_else(|| invalid("placed section plane must be front, top or right"))?,
                cut_mm: cut
                    .parse::<f64>()
                    .map_err(|_| invalid("invalid placed section cut"))?,
                side: CutSide::parse(side)
                    .ok_or_else(|| invalid("placed section side must be positive or negative"))?,
                scale_denominator: scale
                    .parse::<f64>()
                    .map_err(|_| invalid("invalid placed section scale"))?,
                paper_origin_mm: parse_origin(origin)?,
            })
        })
        .collect()
}

/// Parses repeatable `--placed-detail UUID:parent:center:radius:scale:x,y` values into detail
/// views, where center is `x,y` in the parent's paper millimetres. Engine admission still owns
/// every bound; this only builds the typed values.
fn placed_detail_specs(values: &[&str]) -> Run<Vec<DetailSpec>> {
    values
        .iter()
        .map(|value| {
            let mut parts = value.splitn(6, ':');
            let fail = || invalid("placed detail requires UUID:parent:x,y:radius:scale:x,y");
            let id = parts.next().ok_or_else(fail)?;
            let parent = parts.next().ok_or_else(fail)?;
            let center = parts.next().ok_or_else(fail)?;
            let radius = parts.next().ok_or_else(fail)?;
            let scale = parts.next().ok_or_else(fail)?;
            let origin = parts.next().ok_or_else(fail)?;
            Ok(DetailSpec {
                id: DrawingViewId::parse(id).map_err(|_| {
                    invalid("placed detail identity must be a full canonical UUIDv7")
                })?,
                parent: DrawingViewId::parse(parent)
                    .map_err(|_| invalid("placed detail parent must be a full canonical UUIDv7"))?,
                center_paper_mm: parse_origin(center)?,
                radius_mm: radius
                    .parse::<f64>()
                    .map_err(|_| invalid("invalid placed detail radius"))?,
                scale_denominator: scale
                    .parse::<f64>()
                    .map_err(|_| invalid("invalid placed detail scale"))?,
                paper_origin_mm: parse_origin(origin)?,
            })
        })
        .collect()
}
/// Builds the sheet intent from `--sheet-size` and its companion options. A `--sheet-size`
/// requires the project and author fields; every bound is re-checked by engine admission.
fn document_sheet(options: &BTreeMap<&str, &str>) -> Run<Option<SheetSpec>> {
    let Some(size) = options.get("--sheet-size") else {
        return Ok(None);
    };
    let required = |key: &str| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| invalid("required drawing option is missing"))
    };
    let size = Sheet::parse(size).ok_or_else(|| invalid("sheet must be a4, a3, a2, a1 or a0"))?;
    let inset = options.get("--sheet-frame").map_or(Ok(10.0), |value| {
        value
            .parse::<f64>()
            .map_err(|_| invalid("invalid sheet frame inset"))
    })?;
    Ok(Some(SheetSpec {
        size,
        frame: SheetFrame { inset_mm: inset },
        title: TitleBlock {
            project: required("--sheet-project")?.to_owned(),
            author: required("--sheet-author")?.to_owned(),
        },
    }))
}

fn document_drawing(options: &BTreeMap<&str, &str>, source: SourceModelPin) -> Run<Drawing> {
    let required = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| invalid("required drawing option is missing"))
    };
    let id_error = |_| PlateError {
        rule_id: None,
        code: ErrorCode::InvalidId,
        message: "drawing and member identities must be full canonical UUIDv7",
    };
    let id = DrawingId::parse(required("--drawing-id")?).map_err(id_error)?;
    let view_id = DrawingViewId::parse(required("--view-id")?).map_err(id_error)?;
    let dimension_id = DimensionId::parse(required("--dimension-id")?).map_err(id_error)?;
    let member = ElementId::parse(required("--id")?).map_err(id_error)?;
    let anchor = |key| {
        FeatureId::parse(required(key)?).map_err(|error| PlateError {
            rule_id: None,
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
    let origin = options
        .get("--origin")
        .map_or(Ok([0.0, 0.0]), |text| parse_origin(text))?;
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
        drafting: Vec::new(),
        views: Vec::new(),
        sections: Vec::new(),
        details: Vec::new(),
        sheet: document_sheet(options)?,
        layers: Vec::new(),
    })
    .map_err(Into::into)
}

fn document_json(revision: &beamcraft_cli::drawing::document::DocumentRevision) -> Run<Json> {
    Ok(Json::object([
        (
            "revision_id",
            Json::text(revision.info.revision().to_string()),
        ),
        ("sequence", Json::text(revision.info.sequence().to_string())),
        (
            "drawing_id",
            Json::text(revision.drawing.spec().id.to_string()),
        ),
        ("name", Json::text(revision.drawing.spec().name.clone())),
        (
            "view",
            Json::object([
                (
                    "id",
                    Json::text(revision.drawing.spec().view.id.to_string()),
                ),
                (
                    "plane",
                    Json::text(revision.drawing.spec().view.plane.as_str()),
                ),
                (
                    "scale_denominator",
                    num(revision.drawing.spec().view.scale_denominator)?,
                ),
                (
                    "paper_origin_mm",
                    point(revision.drawing.spec().view.paper_origin_mm)?,
                ),
            ]),
        ),
        (
            "sheet",
            match revision.drawing.sheet() {
                None => Json::Null,
                Some(sheet) => Json::object([
                    ("size", Json::text(sheet.size.as_str())),
                    ("frame_inset_mm", num(sheet.frame.inset_mm)?),
                    ("project", Json::text(sheet.title.project.clone())),
                    ("author", Json::text(sheet.title.author.clone())),
                ]),
            },
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
        (
            "placed_views",
            Json::Array(
                revision
                    .drawing
                    .views()
                    .iter()
                    .map(|view| {
                        Ok(Json::object([
                            ("id", Json::text(view.id.to_string())),
                            ("plane", Json::text(view.plane.as_str())),
                            ("scale_denominator", num(view.scale_denominator)?),
                            ("paper_origin_mm", point(view.paper_origin_mm)?),
                        ]))
                    })
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
        (
            "placed_sections",
            Json::Array(
                revision
                    .drawing
                    .sections()
                    .iter()
                    .map(|section| {
                        Ok(Json::object([
                            ("id", Json::text(section.id.to_string())),
                            ("plane", Json::text(section.plane.as_str())),
                            ("cut_mm", num(section.cut_mm)?),
                            ("side", Json::text(section.side.as_str())),
                            ("scale_denominator", num(section.scale_denominator)?),
                            ("paper_origin_mm", point(section.paper_origin_mm)?),
                        ]))
                    })
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
        (
            "placed_details",
            Json::Array(
                revision
                    .drawing
                    .details()
                    .iter()
                    .map(|detail| {
                        Ok(Json::object([
                            ("id", Json::text(detail.id.to_string())),
                            ("parent", Json::text(detail.parent.to_string())),
                            ("center_paper_mm", point(detail.center_paper_mm)?),
                            ("radius_mm", num(detail.radius_mm)?),
                            ("scale_denominator", num(detail.scale_denominator)?),
                            ("paper_origin_mm", point(detail.paper_origin_mm)?),
                        ]))
                    })
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
        (
            "layers",
            Json::Array(
                revision
                    .drawing
                    .layers()
                    .iter()
                    .map(|layer| {
                        Ok(Json::object([
                            ("id", Json::text(layer.id.to_string())),
                            ("name", Json::text(&layer.name)),
                            ("visible", Json::Bool(layer.visible)),
                        ]))
                    })
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
        // The paper-visible subset of the drawn objects: hiding a layer changes this list and
        // never the stored document.
        (
            "visible_drafting",
            Json::Array(
                revision
                    .drawing
                    .visible_drafting()
                    .map(|entity| Json::text(entity.id.to_string()))
                    .collect(),
            ),
        ),
        (
            "drafting",
            Json::Array(
                revision
                    .drawing
                    .drafting()
                    .iter()
                    .map(|entity| {
                        let geometry_mm = Json::Array(
                            entity
                                .geometry
                                .clone()
                                .coordinates()
                                .iter()
                                .map(|value| num(*value))
                                .collect::<Run<Vec<_>>>()?,
                        );
                        let mut fields = vec![
                            ("id", Json::text(entity.id.to_string())),
                            ("kind", Json::text(entity.geometry.kind())),
                            ("layer", Json::text(entity.layer.to_string())),
                            ("line_type", Json::text(entity.line_type.as_str())),
                            ("geometry_mm", geometry_mm),
                        ];
                        if let Some((name, value)) = entity.geometry.string_payload() {
                            let name = name.strip_prefix("drawing:").unwrap_or(name);
                            fields.push((name, Json::text(value)));
                        }
                        Ok(Json::Object(
                            fields
                                .into_iter()
                                .map(|(key, value)| (key.into(), value))
                                .collect(),
                        ))
                    })
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
    ]))
}

fn run_document(args: &[OsString]) -> Run<Json> {
    let command = args
        .first()
        .and_then(|value| value.to_str())
        .ok_or_else(|| invalid("drawing command is missing"))?;
    let parsed = document_options(args)?;
    let options = &parsed.options;
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
            let drawing = document_drawing(options, source.pin())?;
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
            document_json(&revision)
        }
        "edit-drafting" => {
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
            let layers = layer_specs(&parsed.layers)?;
            let drafting = drafting_specs(&parsed.drafting)?;
            // Replaying the exact accepted operation stays safe after later revisions. The
            // fingerprint binds the incoming layers and drafting too: a retry that drops or
            // changes either is a different request, never a replay.
            if let Some(previous) = documents.query(&op)? {
                let candidate = previous.drawing.clone().with_drafting(layers, drafting)?;
                let request = request_fingerprint(
                    &candidate,
                    State::Draft,
                    None,
                    previous.latest_published,
                    actor,
                    &expected,
                )?;
                if previous.request == request {
                    return document_json(&previous);
                }
                return Err(PlateError {
                    rule_id: None,
                    code: ErrorCode::OpIdReuse,
                    message: "document operation id was already used for another request",
                });
            }
            let current = documents.head()?;
            // Drawn objects belong to the document only: no model store is read, the retained
            // source archive and every identity carry over unchanged.
            let edited = current.drawing.clone().with_drafting(layers, drafting)?;
            let (_, revised) = documents.commit(
                &expected,
                DocumentWrite {
                    revision,
                    operation: op,
                    actor,
                    drawing: &edited,
                    source: &current.source,
                    state: State::Draft,
                    version: None,
                    latest_published: current.latest_published,
                },
            )?;
            Ok(document_json(&revised)?)
        }
        "edit-views" => {
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
            let views = placed_view_specs(&parsed.views)?;
            let sections = placed_section_specs(&parsed.sections)?;
            let details = placed_detail_specs(&parsed.details)?;
            // Same replay discipline as edit-drafting: the fingerprint binds the incoming
            // placement of all three collections, so a retry that drops or changes one is a
            // different request.
            let spliced = |drawing: &Drawing| {
                drawing
                    .clone()
                    .with_views(views.clone())?
                    .with_sections(sections.clone())?
                    .with_details(details.clone())
            };
            if let Some(previous) = documents.query(&op)? {
                let candidate = spliced(&previous.drawing)?;
                let request = request_fingerprint(
                    &candidate,
                    State::Draft,
                    None,
                    previous.latest_published,
                    actor,
                    &expected,
                )?;
                if previous.request == request {
                    return document_json(&previous);
                }
                return Err(PlateError {
                    rule_id: None,
                    code: ErrorCode::OpIdReuse,
                    message: "document operation id was already used for another request",
                });
            }
            let current = documents.head()?;
            // Placed views belong to the document only: no model store is read, the retained
            // source archive and every identity carry over unchanged.
            let edited = spliced(&current.drawing)?;
            let (_, revised) = documents.commit(
                &expected,
                DocumentWrite {
                    revision,
                    operation: op,
                    actor,
                    drawing: &edited,
                    source: &current.source,
                    state: State::Draft,
                    version: None,
                    latest_published: current.latest_published,
                },
            )?;
            Ok(document_json(&revised)?)
        }
        "edit-sheet" => {
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
            if options
                .get("--sheet-remove")
                .is_some_and(|value| *value != "true")
            {
                return Err(invalid("--sheet-remove must be true"));
            }
            let remove = options.contains_key("--sheet-remove");
            if remove && options.contains_key("--sheet-size") {
                return Err(invalid("choose either --sheet-remove true or a new sheet"));
            }
            let sheet = if remove {
                None
            } else {
                document_sheet(options)?
            };
            // Same replay discipline as edit-views: the fingerprint binds the incoming sheet,
            // so a retry that drops or changes it is a different request.
            if let Some(previous) = documents.query(&op)? {
                let candidate = match sheet.clone() {
                    Some(value) => previous.drawing.clone().with_sheet(value)?,
                    None if remove => previous.drawing.clone().without_sheet(),
                    None => previous.drawing.clone(),
                };
                let request = request_fingerprint(
                    &candidate,
                    State::Draft,
                    None,
                    previous.latest_published,
                    actor,
                    &expected,
                )?;
                if previous.request == request {
                    return document_json(&previous);
                }
                return Err(PlateError {
                    rule_id: None,
                    code: ErrorCode::OpIdReuse,
                    message: "document operation id was already used for another request",
                });
            }
            let current = documents.head()?;
            let edited = match sheet {
                Some(value) => current.drawing.clone().with_sheet(value)?,
                None if remove => current.drawing.clone().without_sheet(),
                None => {
                    return Err(invalid(
                        "edit-sheet requires --sheet-size with its fields or --sheet-remove true",
                    ));
                }
            };
            let (_, revised) = documents.commit(
                &expected,
                DocumentWrite {
                    revision,
                    operation: op,
                    actor,
                    drawing: &edited,
                    source: &current.source,
                    state: State::Draft,
                    version: None,
                    latest_published: current.latest_published,
                },
            )?;
            Ok(document_json(&revised)?)
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
                    return document_json(&previous);
                }
                return Err(PlateError {
                    rule_id: None,
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
            document_json(&published)
        }
        "export-drawing" => {
            let documents = DocumentStore::open(std::path::Path::new(required("--document-dir")?))?;
            let version = DocumentVersionId::parse(required("--version-id")?)
                .map_err(|_| invalid("document version id must be UUIDv7"))?;
            let destination = required("--out")?;
            // One exact published version, reopened from its own immutable archived source.
            // The live model is never read and never changes; the document head is only read.
            let published = documents
                .history()?
                .into_iter()
                .find(|revision| {
                    revision.state == State::Published && revision.version == Some(version)
                })
                .ok_or(PlateError {
                    rule_id: None,
                    code: ErrorCode::NotFound,
                    message: "no published document version carries this id",
                })?;
            let snapshot = published.source.inspect()?.snapshot;
            let projection = projection::resolve(&published.drawing, &snapshot)?;
            let format = options.get("--format").copied().unwrap_or("svg");
            let bytes = match format {
                // A sheet document renders its whole sheet; a loose document keeps the
                // earlier tight-bounds export. Both stay bound to the exact version.
                "svg" => match published.drawing.sheet() {
                    Some(_) => {
                        let mut bodies = Vec::with_capacity(published.drawing.views().len());
                        for view in published.drawing.views() {
                            bodies.push(projection::resolve_view(
                                &published.drawing,
                                view,
                                &snapshot,
                            )?);
                        }
                        sheet::export_svg(&published.drawing, version, &projection, &bodies)?
                    }
                    None => svg::export(&published.drawing, version, &projection)?,
                },
                "pdf" => {
                    let sheet = print::Sheet::parse(
                        options
                            .get("--sheet")
                            .copied()
                            .or_else(|| published.drawing.sheet().map(|sheet| sheet.size.as_str()))
                            .unwrap_or("a4"),
                    )
                    .ok_or_else(|| invalid("sheet must be a4, a3, a2, a1 or a0"))?;
                    print::export(&published.drawing, version, &projection, sheet)?
                }
                _ => return Err(invalid("format must be svg or pdf")),
            };
            // Reserve the destination only after the exact version verified and rendered; an
            // existing file is never opened for writing or removed.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)
                .map_err(|_| invalid("drawing export destination exists or cannot be created"))?;
            if file
                .write_all(&bytes)
                .and_then(|()| file.sync_all())
                .is_err()
            {
                drop(file);
                let _ = std::fs::remove_file(destination);
                return Err(PlateError {
                    rule_id: None,
                    code: ErrorCode::BackendUnavailable,
                    message: "drawing export could not be written",
                });
            }
            Ok(Json::object([
                ("outcome", Json::text("drawing_export")),
                ("format", Json::text(format)),
                (
                    "drawing_id",
                    Json::text(published.drawing.spec().id.to_string()),
                ),
                ("version_id", Json::text(version.to_string())),
                (
                    "source_revision_id",
                    Json::text(published.source.pin().revision.to_string()),
                ),
                (
                    "source_manifest",
                    Json::text(published.source.pin().manifest.to_string()),
                ),
                ("bytes", num(bytes.len() as f64)?),
            ]))
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
            let mut fields = match document_json(&current)? {
                Json::Object(fields) => fields,
                _ => {
                    return Err(PlateError {
                        rule_id: None,
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
                            .collect::<Run<Vec<_>>>()?,
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
        rule_id: None,
        code: ErrorCode::InvalidId,
        message: "drawing and member identities must be full canonical UUIDv7",
    };
    let id = DrawingId::parse(required("--drawing-id")?).map_err(id_error)?;
    let view_id = DrawingViewId::parse(required("--view-id")?).map_err(id_error)?;
    let dimension_id = DimensionId::parse(required("--dimension-id")?).map_err(id_error)?;
    let member = ElementId::parse(required("--id")?).map_err(id_error)?;
    let anchor = |key| {
        FeatureId::parse(required(key)?).map_err(|e| PlateError {
            rule_id: None,
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
    let origin = options
        .get("--origin")
        .map_or(Ok([0.0, 0.0]), |text| parse_origin(text))?;
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
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let revision = RevisionStore::new(blobs).open()?;
    let content = revision.manifest().content();
    let source = SourceModelPin {
        project: ProjectId::parse(content.project().as_str())
            .map_err(crate::plate_cmd::context_id_error)?,
        revision: RevisionId::parse(revision.info().revision().as_str())
            .map_err(crate::plate_cmd::context_id_error)?,
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
        drafting: Vec::new(),
        views: Vec::new(),
        sections: Vec::new(),
        details: Vec::new(),
        sheet: None,
        layers: Vec::new(),
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
                rule_id: None,
                code: ErrorCode::BackendUnavailable,
                message: "drawing intent could not be written",
            });
        }
    }
    Ok(result)
}
