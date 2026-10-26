// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01 headless grid creation and verified listing.
use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{check_project_option, footprint_json, id_error, num, revision_json};
use beamcraft_cli::grid::{GridContext, create_grid, grids};
use beamcraft_cli::plate::PlateError;
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::grid::{
    CreateGrid, Grid, GridFamily, GridLabelOrder, RegularGridFamily, generate_grid_labels,
    parse_grid_expression, parse_grid_labels,
};
use beamcraft_model::id::{ElementId, Uuid7};
use std::{collections::BTreeMap, ffi::OsString};
type Run<T> = Result<T, PlateError>;
fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}
fn error(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code,
        message,
    }
}
/// Whether this is a grid command.
pub fn handles(c: &OsString) -> bool {
    matches!(
        c.to_str(),
        Some(
            "create-grid"
                | "update-grid"
                | "list-grids"
                | "resolve-grid-point"
                | "resolve-grid-plane"
                | "draw-grid-line"
                | "reshape-grid-line"
                | "remove-grid-line"
                | "relabel-grid-line"
        )
    )
}
/// Run one grid command through the common JSON result protocol.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    let result = run(args);
    let diagnostics = result
        .as_ref()
        .err()
        .filter(|e| {
            matches!(
                e.message,
                "GRID_LABEL_DUPLICATE"
                    | "GRID_LABEL_COUNT"
                    | "GRID_LABEL_INVALID"
                    | "GRID_SPACING_INVALID"
                    | "GRID_EXPRESSION_INVALID"
                    | "GRID_LINE_DEGENERATE"
                    | "GRID_FAMILY_MODE"
                    | "GRID_AXIS_UNKNOWN"
                    | "GRID_IN_USE"
                    | "GRID_AXIS_ID_COUNT"
                    | "GRID_UNKNOWN"
                    | "GRID_NAME_AMBIGUOUS"
                    | "GRID_INTERSECTION_UNDEFINED"
            )
        })
        .map(|e| {
            vec![Json::object([
                ("rule", Json::text("T01-V01")),
                ("reason_code", Json::text(e.message)),
            ])]
        })
        .unwrap_or_default();
    crate::report::report_with(result, diagnostics)
}
fn pair(text: &str) -> Run<[f64; 2]> {
    let numbers = text
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.parse::<f64>()
                .map_err(|_| invalid("invalid coordinate pair"))
        })
        .collect::<Run<Vec<_>>>()?;
    match numbers.as_slice() {
        [a, b] => Ok([*a, *b]),
        _ => Err(invalid("expected two coordinates")),
    }
}
fn family(
    options: &BTreeMap<&str, &str>,
    u: bool,
    frame: &Grid,
    fresh: &mut dyn FnMut() -> Run<Uuid7>,
) -> Run<GridFamily> {
    let prefix = if u { "--u" } else { "--v" };
    let key = |suffix: &str| format!("{prefix}-{suffix}");
    let get = |suffix: &str| options.get(key(suffix).as_str()).copied();
    if get("expression").is_some()
        && ["from", "to", "label", "axis-id"]
            .iter()
            .any(|s| get(s).is_some())
    {
        return Err(error(ErrorCode::UnsupportedProperty, "GRID_FAMILY_MODE"));
    }
    let Some(expression) = get("expression") else {
        if ["from", "to", "label", "axis-id"]
            .iter()
            .any(|s| get(s).is_some())
            && [
                "labels",
                "axis-ids",
                "span",
                "scheme",
                "label-start",
                "label-step",
                "label-order",
            ]
            .iter()
            .any(|s| get(s).is_some())
        {
            return Err(error(ErrorCode::UnsupportedProperty, "GRID_FAMILY_MODE"));
        }
        if ["from", "to", "label", "axis-id"]
            .iter()
            .any(|s| get(s).is_some())
        {
            let from =
                pair(get("from").ok_or_else(|| invalid("drawn family requires from and to"))?)?;
            let to = pair(get("to").ok_or_else(|| invalid("drawn family requires from and to"))?)?;
            let label = get("label")
                .ok_or_else(|| invalid("drawn family requires a label"))?
                .to_owned();
            let axis_id = match get("axis-id") {
                Some(s) => ElementId::parse(s).map_err(id_error)?,
                None => ElementId::from_uuid(fresh()?),
            };
            return Ok(GridFamily::Drawn(vec![
                beamcraft_model::grid::DrawnGridLine {
                    axis_id,
                    label,
                    start_mm: beamcraft_engine::grid_edit::world_to_local(frame, from),
                    end_mm: beamcraft_engine::grid_edit::world_to_local(frame, to),
                },
            ]));
        }
        if [
            "labels",
            "axis-ids",
            "span",
            "scheme",
            "label-start",
            "label-step",
            "label-order",
        ]
        .iter()
        .any(|s| get(s).is_some())
        {
            return Err(invalid("family options require an expression"));
        }
        return Ok(GridFamily::Drawn(Vec::new()));
    };
    let (expression, positions_mm) = parse_grid_expression(expression).map_err(|e| {
        error(
            match e {
                beamcraft_model::grid::GridExpressionError::Limit => ErrorCode::LimitExceeded,
                beamcraft_model::grid::GridExpressionError::NonFinite => ErrorCode::NonFiniteNumber,
                _ => ErrorCode::InvalidArgument,
            },
            match e {
                beamcraft_model::grid::GridExpressionError::Spacing => "GRID_SPACING_INVALID",
                _ => "GRID_EXPRESSION_INVALID",
            },
        )
    })?;
    let count = positions_mm.len();
    if get("labels").is_some() && get("scheme").is_some() {
        return Err(invalid("choose explicit labels or a label scheme"));
    }
    let labels = match get("labels") {
        Some(s) => parse_grid_labels(s, count),
        None => generate_grid_labels(
            get("scheme").unwrap_or(if u { "numeric" } else { "alpha" }),
            count,
            get("label-start"),
            get("label-step")
                .map(|s| {
                    s.parse::<u32>()
                        .map_err(|_| invalid("invalid numeric label step"))
                })
                .transpose()?,
            match get("label-order").unwrap_or("ascending") {
                "ascending" => GridLabelOrder::Ascending,
                "descending" => GridLabelOrder::Descending,
                _ => return Err(invalid("invalid label order")),
            },
        ),
    }
    .map_err(|e| {
        invalid(match e {
            beamcraft_model::grid::GridLabelError::Duplicate => "GRID_LABEL_DUPLICATE",
            beamcraft_model::grid::GridLabelError::Count => "GRID_LABEL_COUNT",
            _ => "GRID_LABEL_INVALID",
        })
    })?;
    let axis_ids = match get("axis-ids") {
        Some(s) => s
            .split(|c: char| c == ',' || c.is_ascii_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| ElementId::parse(s).map_err(id_error))
            .collect::<Run<Vec<_>>>()?,
        None => (0..count)
            .map(|_| fresh().map(ElementId::from_uuid))
            .collect::<Run<Vec<_>>>()?,
    };
    if axis_ids.len() != count {
        return Err(invalid("axis id count does not match the expression"));
    }
    let span_mm = match get("span") {
        Some(s) => pair(s)?,
        None => [0.0, 0.0],
    };
    Ok(GridFamily::Regular(RegularGridFamily {
        expression,
        labels,
        axis_ids,
        span_mm,
        positions_mm,
    }))
}
fn set_default_span(f: &mut GridFamily, opposite: &GridFamily) {
    if let GridFamily::Regular(f) = f {
        let positions = match opposite {
            GridFamily::Regular(f) => f.positions_mm.as_slice(),
            GridFamily::Drawn(_) => &[],
        };
        f.span_mm = match (positions.first(), positions.last()) {
            (Some(a), Some(b)) => [*a - 2000.0, *b + 2000.0],
            _ => [-2000.0, 2000.0],
        };
    }
}
fn run(args: &[OsString]) -> Run<Json> {
    let args = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 16_384)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<Vec<_>>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 80 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let own: &[&str] = match *command {
        "create-grid" => &[
            "--store",
            "--name",
            "--new-id",
            "--origin",
            "--rotation",
            "--u-expression",
            "--v-expression",
            "--u-labels",
            "--v-labels",
            "--u-axis-ids",
            "--v-axis-ids",
            "--u-span",
            "--v-span",
            "--u-scheme",
            "--v-scheme",
            "--u-label-start",
            "--v-label-start",
            "--u-label-step",
            "--v-label-step",
            "--u-label-order",
            "--v-label-order",
            "--u-from",
            "--u-to",
            "--u-label",
            "--u-axis-id",
            "--v-from",
            "--v-to",
            "--v-label",
            "--v-axis-id",
        ],
        "list-grids" => &["--store"],
        "resolve-grid-point" => &["--store", "--grid", "--labels"],
        "resolve-grid-plane" => &["--store", "--grid", "--label"],
        "update-grid" => &[
            "--store",
            "--id",
            "--name",
            "--origin",
            "--rotation",
            "--u-expression",
            "--u-labels",
            "--u-new-axis-ids",
            "--u-span",
            "--v-expression",
            "--v-labels",
            "--v-new-axis-ids",
            "--v-span",
        ],
        "draw-grid-line" => &[
            "--store",
            "--id",
            "--family",
            "--new-axis-id",
            "--label",
            "--from",
            "--to",
        ],
        "reshape-grid-line" => &["--store", "--id", "--axis-id", "--from", "--to"],
        "remove-grid-line" => &["--store", "--id", "--axis-id"],
        "relabel-grid-line" => &[
            "--store",
            "--id",
            "--axis-id",
            "--label",
            "--family",
            "--labels",
        ],
        _ => return Err(invalid("unknown grid command")),
    };
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed = if *command == "list-grids" {
            *key == "--store"
        } else {
            own.contains(key) || CONTEXT_OPTIONS.contains(key)
        };
        if !allowed || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let store = RevisionStore::new(
        BlobStore::new(root).map_err(|e| error(e.code(), "store root is not usable"))?,
    );
    if *command == "list-grids" {
        return Ok(Json::object([
            ("outcome", Json::text("grids")),
            ("grids", views_json(&grids(&store)?)?),
        ]));
    }
    if matches!(*command, "resolve-grid-point" | "resolve-grid-plane") {
        let model = grids(&store)?;
        let required = |key| {
            options
                .get(key)
                .copied()
                .ok_or_else(|| invalid("missing grid query option"))
        };
        let grid = beamcraft_engine::grid_query::resolve_grid(&model, required("--grid")?)?;
        if *command == "resolve-grid-point" {
            let (a, b) = required("--labels")?
                .split_once(',')
                .filter(|(_, b)| !b.contains(','))
                .ok_or_else(|| invalid("--labels needs two comma-separated labels"))?;
            let [x, y] = beamcraft_engine::grid_query::grid_point(grid, a, b)?;
            return Ok(Json::object([
                ("outcome", Json::text("grid_point")),
                ("grid_id", Json::text(grid.id.to_string())),
                ("point_world_mm", Json::Array(vec![num(x)?, num(y)?])),
            ]));
        }
        let p = beamcraft_engine::grid_query::grid_line_plane(grid, required("--label")?)?;
        return Ok(Json::object([
            ("outcome", Json::text("grid_plane")),
            ("grid_id", Json::text(p.grid_id.to_string())),
            ("axis_id", Json::text(p.axis_id.to_string())),
            (
                "point_world_mm",
                crate::plate_cmd::triple(p.point_world_mm)?,
            ),
            ("normal_world", crate::plate_cmd::triple(p.normal_world)?),
        ]));
    }
    let _claim = check_project_option(&store, &options)?;
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| {
            error(
                ErrorCode::BackendUnavailable,
                "no identity could be generated",
            )
        })
    };
    let common = command_context::parse(&options, &mut fresh)?;
    let context = GridContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    if *command == "update-grid" {
        return update_command(&store, &options, &context, &mut fresh);
    }
    if *command != "create-grid" {
        return line_command(&store, &options, command, &context, &mut fresh);
    }
    let id = match options.get("--new-id") {
        Some(s) => ElementId::parse(s).map_err(id_error)?,
        None => ElementId::from_uuid(fresh()?),
    };
    let rotation_rad = options
        .get("--rotation")
        .copied()
        .unwrap_or("0")
        .parse::<f64>()
        .map_err(|_| invalid("invalid grid rotation"))?
        .to_radians();
    if !rotation_rad.is_finite() {
        return Err(error(
            ErrorCode::NonFiniteNumber,
            "grid rotation is not finite",
        ));
    }
    if rotation_rad.abs() > std::f64::consts::TAU {
        return Err(invalid("grid rotation exceeds 360 degrees"));
    }
    let mut grid = Grid {
        id,
        name: options
            .get("--name")
            .ok_or_else(|| invalid("--name is required"))?
            .to_string(),
        translation_mm: pair(options.get("--origin").copied().unwrap_or("0,0"))?,
        orientation_zw: [(rotation_rad / 2.0).sin(), (rotation_rad / 2.0).cos()],
        u: GridFamily::Drawn(Vec::new()),
        v: GridFamily::Drawn(Vec::new()),
    };
    grid.u = family(&options, true, &grid, &mut fresh)?;
    grid.v = family(&options, false, &grid, &mut fresh)?;
    if !options.contains_key("--u-span") {
        set_default_span(&mut grid.u, &grid.v)
    }
    if !options.contains_key("--v-span") {
        set_default_span(&mut grid.v, &grid.u)
    }
    let change = create_grid(&store, &context, &CreateGrid { grid }, &mut NoFault)?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if change.replayed {
                "grid_created_replayed"
            } else {
                "grid_created"
            }),
        ),
        ("op_id", Json::text(context.op_id.to_string())),
        ("element_id", Json::text(id.to_string())),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
        ("grids", views_json(&change.grids)?),
    ]))
}
pub(crate) fn views_json(grids: &[Grid]) -> Run<Json> {
    Ok(Json::Array(
        grids
            .iter()
            .map(|g| {
                let world = |[x, y]: [f64; 2]| -> Run<Json> {
                    let [z, w] = g.orientation_zw;
                    let norm = z * z + w * w;
                    let s = 2.0 * z * w / norm;
                    let c = (w * w - z * z) / norm;
                    let [tx, ty] = g.translation_mm;
                    Ok(Json::Array(vec![
                        num(c * x - s * y + tx)?,
                        num(s * x + c * y + ty)?,
                        num(0.0)?,
                    ]))
                };
                let family = |f: &GridFamily, u: bool| -> Run<Json> {
                    let (mode, expression, lines) = match f {
                        GridFamily::Regular(f) => {
                            let [a, b] = f.span_mm;
                            (
                                "regular",
                                Json::text(&f.expression),
                                f.axis_ids
                                    .iter()
                                    .zip(&f.labels)
                                    .zip(&f.positions_mm)
                                    .map(|((id, label), p)| {
                                        (
                                            *id,
                                            label.clone(),
                                            if u { [*p, a] } else { [a, *p] },
                                            if u { [*p, b] } else { [b, *p] },
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            )
                        }
                        GridFamily::Drawn(lines) => (
                            "drawn",
                            Json::Null,
                            lines
                                .iter()
                                .map(|l| (l.axis_id, l.label.clone(), l.start_mm, l.end_mm))
                                .collect(),
                        ),
                    };
                    Ok(Json::object([
                        ("mode", Json::text(mode)),
                        ("expression", expression),
                        (
                            "lines",
                            Json::Array(
                                lines
                                    .into_iter()
                                    .map(|(id, label, a, b)| {
                                        Ok(Json::object([
                                            ("axis_id", Json::text(id.to_string())),
                                            ("label", Json::text(label)),
                                            ("start_mm", world(a)?),
                                            ("end_mm", world(b)?),
                                        ]))
                                    })
                                    .collect::<Run<Vec<_>>>()?,
                            ),
                        ),
                    ]))
                };
                Ok(Json::object([
                    ("id", Json::text(g.id.to_string())),
                    ("name", Json::text(&g.name)),
                    (
                        "origin_mm",
                        Json::Array(
                            g.translation_mm
                                .iter()
                                .copied()
                                .map(num)
                                .collect::<Run<Vec<_>>>()?,
                        ),
                    ),
                    (
                        "rotation_rad",
                        num(2.0 * g.orientation_zw[0].atan2(g.orientation_zw[1]))?,
                    ),
                    ("u", family(&g.u, true)?),
                    ("v", family(&g.v, false)?),
                ]))
            })
            .collect::<Run<Vec<_>>>()?,
    ))
}

/// Copy a grid through the ordinary `copy-element` command family.
pub(crate) fn copy_command(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| {
            error(
                ErrorCode::BackendUnavailable,
                "no identity could be generated",
            )
        })
    };
    let common = command_context::parse(options, &mut fresh)?;
    let context = GridContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    let source_id = ElementId::parse(
        options
            .get("--id")
            .ok_or_else(|| invalid("--id is required"))?,
    )
    .map_err(id_error)?;
    let new_id = match options.get("--new-id") {
        Some(s) => ElementId::parse(s).map_err(id_error)?,
        None => ElementId::from_uuid(fresh()?),
    };
    let new_axis_ids = match options.get("--new-axis-ids") {
        Some(s) => s
            .split(|c: char| c == ',' || c.is_ascii_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| ElementId::parse(s).map_err(id_error))
            .collect::<Run<Vec<_>>>()?,
        None => {
            let source = grids(store)?
                .into_iter()
                .find(|g| g.id == source_id)
                .ok_or_else(|| error(ErrorCode::NotFound, "source grid not found"))?;
            (0..source.axis_ids().count())
                .map(|_| fresh().map(ElementId::from_uuid))
                .collect::<Run<Vec<_>>>()?
        }
    };
    let offset_mm = match options.get("--offset") {
        None => [0.0; 2],
        Some(s) => {
            let numbers = s
                .split(|c: char| c == ',' || c.is_ascii_whitespace())
                .filter(|s| !s.is_empty())
                .map(|s| s.parse::<f64>().map_err(|_| invalid("invalid copy offset")))
                .collect::<Run<Vec<_>>>()?;
            match numbers.as_slice() {
                [x, y, z] if *z == 0.0 => [*x, *y],
                _ => {
                    return Err(error(
                        ErrorCode::UnsupportedGeometry,
                        "grid copy offset must have zero elevation",
                    ));
                }
            }
        }
    };
    let request = beamcraft_model::grid::CopyGrid {
        source_id,
        new_id,
        name: options.get("--name").map(|s| s.to_string()),
        offset_mm,
        new_axis_ids,
    };
    let change = beamcraft_cli::grid::copy_grid(store, &context, &request, &mut NoFault)?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if change.replayed {
                "element_copy_replayed"
            } else {
                "element_copied"
            }),
        ),
        ("source_id", Json::text(source_id.to_string())),
        ("element_id", Json::text(new_id.to_string())),
        ("op_id", Json::text(context.op_id.to_string())),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
        ("grids", views_json(&change.grids)?),
    ]))
}

fn line_command(
    store: &RevisionStore,
    options: &BTreeMap<&str, &str>,
    command: &str,
    context: &GridContext,
    fresh: &mut dyn FnMut() -> Run<Uuid7>,
) -> Run<Json> {
    use beamcraft_model::grid_edit::{GridFamilyId, GridLineEdit, GridLineEditRequest};
    let required = |name| {
        options
            .get(name)
            .copied()
            .ok_or_else(|| invalid("missing grid-line option"))
    };
    let target_id = ElementId::parse(required("--id")?).map_err(id_error)?;
    let axis = || ElementId::parse(required("--axis-id")?).map_err(id_error);
    let family = || {
        GridFamilyId::parse(required("--family")?).ok_or_else(|| invalid("family must be u or v"))
    };
    let edit = match command {
        "draw-grid-line" => GridLineEdit::Draw {
            family: family()?,
            new_axis_id: match options.get("--new-axis-id") {
                Some(s) => ElementId::parse(s).map_err(id_error)?,
                None => ElementId::from_uuid(fresh()?),
            },
            label: required("--label")?.to_owned(),
            from_mm: pair(required("--from")?)?,
            to_mm: pair(required("--to")?)?,
        },
        "reshape-grid-line" => GridLineEdit::Reshape {
            axis_id: axis()?,
            from_mm: pair(required("--from")?)?,
            to_mm: pair(required("--to")?)?,
        },
        "remove-grid-line" => GridLineEdit::Remove { axis_id: axis()? },
        "relabel-grid-line" => {
            if options.contains_key("--family") {
                if options.contains_key("--axis-id") || options.contains_key("--label") {
                    return Err(invalid("choose one axis or a family label list"));
                }
                let text = required("--labels")?;
                let count = text
                    .split(|c: char| c.is_ascii_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .count();
                GridLineEdit::RelabelFamily {
                    family: family()?,
                    labels: parse_grid_labels(text, count)
                        .map_err(|_| invalid("GRID_LABEL_INVALID"))?,
                }
            } else {
                if options.contains_key("--labels") {
                    return Err(invalid("label list requires a family"));
                }
                GridLineEdit::Relabel {
                    axis_id: axis()?,
                    label: required("--label")?.to_owned(),
                }
            }
        }
        _ => return Err(invalid("unknown grid-line command")),
    };
    let change = beamcraft_cli::grid::edit_grid(
        store,
        context,
        &GridLineEditRequest { target_id, edit },
        &mut NoFault,
    )?;
    let outcome = match command {
        "draw-grid-line" => "grid_line_drawn",
        "reshape-grid-line" => "grid_line_reshaped",
        "remove-grid-line" => "grid_line_removed",
        _ => "grid_line_relabelled",
    };
    Ok(Json::object([
        (
            "outcome",
            Json::text(if change.replayed {
                format!("{outcome}_replayed")
            } else {
                outcome.to_owned()
            }),
        ),
        ("op_id", Json::text(context.op_id.to_string())),
        ("element_id", Json::text(target_id.to_string())),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
        ("grids", views_json(&change.grids)?),
    ]))
}

fn regular_patch(
    store: &RevisionStore,
    target: ElementId,
    options: &BTreeMap<&str, &str>,
    u: bool,
    fresh: &mut dyn FnMut() -> Run<Uuid7>,
) -> Run<Option<beamcraft_model::grid_update::RegularGridPatch>> {
    use beamcraft_model::grid_update::RegularGridPatch;
    let prefix = if u { "--u" } else { "--v" };
    let key = |suffix: &str| format!("{prefix}-{suffix}");
    let get = |suffix: &str| options.get(key(suffix).as_str()).copied();
    if ["expression", "labels", "new-axis-ids", "span"]
        .iter()
        .all(|s| get(s).is_none())
    {
        return Ok(None);
    }
    if get("expression").is_some() != get("labels").is_some() {
        return Err(invalid("GRID_LABEL_COUNT"));
    }
    let parsed = get("expression")
        .map(|text| {
            parse_grid_expression(text).map_err(|e| {
                error(
                    match e {
                        beamcraft_model::grid::GridExpressionError::Limit => {
                            ErrorCode::LimitExceeded
                        }
                        beamcraft_model::grid::GridExpressionError::NonFinite => {
                            ErrorCode::NonFiniteNumber
                        }
                        _ => ErrorCode::InvalidArgument,
                    },
                    match e {
                        beamcraft_model::grid::GridExpressionError::Spacing => {
                            "GRID_SPACING_INVALID"
                        }
                        _ => "GRID_EXPRESSION_INVALID",
                    },
                )
            })
        })
        .transpose()?;
    let expression = parsed.as_ref().map(|(normalized, _)| normalized.clone());
    let labels = parsed
        .map(|(_, positions)| {
            parse_grid_labels(
                get("labels").ok_or_else(|| invalid("GRID_LABEL_COUNT"))?,
                positions.len(),
            )
            .map_err(|e| {
                invalid(match e {
                    beamcraft_model::grid::GridLabelError::Count => "GRID_LABEL_COUNT",
                    beamcraft_model::grid::GridLabelError::Duplicate => "GRID_LABEL_DUPLICATE",
                    _ => "GRID_LABEL_INVALID",
                })
            })
        })
        .transpose()?;
    let new_axis_ids = match get("new-axis-ids") {
        Some(s) => s
            .split(|c: char| c.is_ascii_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(|s| ElementId::parse(s).map_err(id_error))
            .collect::<Run<Vec<_>>>()?,
        None => {
            if let Some(labels) = &labels {
                let source = grids(store)?
                    .into_iter()
                    .find(|g| g.id == target)
                    .ok_or_else(|| error(ErrorCode::NotFound, "grid not found"))?;
                let family = if u { &source.u } else { &source.v };
                let GridFamily::Regular(f) = family else {
                    return Err(error(ErrorCode::UnsupportedProperty, "GRID_FAMILY_MODE"));
                };
                labels
                    .iter()
                    .filter(|label| !f.labels.contains(label))
                    .map(|_| fresh().map(ElementId::from_uuid))
                    .collect::<Run<Vec<_>>>()?
            } else {
                Vec::new()
            }
        }
    };
    Ok(Some(RegularGridPatch {
        expression,
        labels,
        new_axis_ids,
        span_mm: get("span").map(pair).transpose()?,
    }))
}
fn update_command(
    store: &RevisionStore,
    options: &BTreeMap<&str, &str>,
    context: &GridContext,
    fresh: &mut dyn FnMut() -> Run<Uuid7>,
) -> Run<Json> {
    let target_id = ElementId::parse(
        options
            .get("--id")
            .ok_or_else(|| invalid("--id is required"))?,
    )
    .map_err(id_error)?;
    let orientation_zw = options
        .get("--rotation")
        .map(|s| {
            let degrees = s
                .parse::<f64>()
                .map_err(|_| invalid("invalid grid rotation"))?;
            if !degrees.is_finite() {
                return Err(error(
                    ErrorCode::NonFiniteNumber,
                    "grid rotation is not finite",
                ));
            }
            if degrees.abs() > 360.0 {
                return Err(invalid("grid rotation exceeds 360 degrees"));
            }
            let (z, w) = (degrees.to_radians() / 2.0).sin_cos();
            Ok([z, w])
        })
        .transpose()?;
    let request = beamcraft_model::grid_update::UpdateGrid {
        target_id,
        name: options.get("--name").map(|s| s.to_string()),
        origin_mm: options.get("--origin").map(|s| pair(s)).transpose()?,
        orientation_zw,
        u: regular_patch(store, target_id, options, true, fresh)?,
        v: regular_patch(store, target_id, options, false, fresh)?,
    };
    let change = beamcraft_cli::grid::update_grid(store, context, &request, &mut NoFault)?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if change.replayed {
                "grid_update_replayed"
            } else {
                "grid_updated"
            }),
        ),
        ("op_id", Json::text(context.op_id.to_string())),
        ("element_id", Json::text(target_id.to_string())),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
        ("grids", views_json(&change.grids)?),
    ]))
}
