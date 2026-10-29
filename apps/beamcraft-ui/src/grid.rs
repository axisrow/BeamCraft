// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Regular-grid insertion and display, using the existing grid commands and engine endpoints.

use crate::camera::View;
use crate::props::RevisionStamp;
use crate::scene::SceneSummary;
use crate::stats::Stats;
use crate::tools::{FormError, ToolError, decode, push_actor, rejection, text};
use crate::{command, theme};
use beamcraft_json::Json;
use beamcraft_model::grid::{
    GridLabelError, MAX_GRID_LINES_PER_FAMILY, MAX_GRIDS, parse_grid_expression, parse_grid_labels,
};
use beamcraft_model::id::ElementId;
use std::path::Path;

fn pair(json: &Json, key: &str) -> Option<[f64; 2]> {
    let [a, b] = json.get(key).ok()?.array().ok()? else {
        return None;
    };
    let values = [a.float().ok()?, b.float().ok()?];
    values.iter().all(|n| n.is_finite()).then_some(values)
}

fn project(point: [f64; 3], rect: egui::Rect, view: &View, origin: [f64; 3]) -> Option<egui::Pos2> {
    let local = std::array::from_fn(|i| {
        (point.get(i).copied().unwrap_or(0.0) - origin.get(i).copied().unwrap_or(0.0)) as f32
    });
    let [x, y, depth] = view.project(local);
    (x.is_finite()
        && y.is_finite()
        && x.abs() < 1e5
        && y.abs() < 1e5
        && (0.0..=1.0).contains(&depth))
    .then(|| {
        egui::pos2(
            rect.left() + (x + 1.0) * rect.width() / 2.0,
            rect.top() + (1.0 - y) * rect.height() / 2.0,
        )
    })
}

/// Inputs of one regular grid family. Empty labels ask the CLI for its default label scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyForm {
    /// First position followed by spacings, in millimetres.
    pub expression: String,
    /// Explicit labels, or empty for automatic labels.
    pub labels: String,
    /// Nonnegative start distance (negated in the command) and the end coordinate,
    /// or both empty for the engine default.
    pub span: [String; 2],
}

/// Editable regular-grid request, in millimetres and display degrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridForm {
    /// Name stored on the grid.
    pub name: String,
    /// World XY origin.
    pub origin: [String; 2],
    /// Rotation about world Z in degrees.
    pub rotation: String,
    /// U lines, parallel to local Y.
    pub u: FamilyForm,
    /// V lines, parallel to local X.
    pub v: FamilyForm,
}

impl Default for GridForm {
    fn default() -> Self {
        Self {
            name: "Grid".into(),
            origin: ["0".into(), "0".into()],
            rotation: "0".into(),
            u: FamilyForm {
                expression: "0 2*4000".into(),
                labels: String::new(),
                span: [String::new(), String::new()],
            },
            v: FamilyForm {
                expression: "0 2*5000".into(),
                labels: String::new(),
                span: [String::new(), String::new()],
            },
        }
    }
}

fn form_error(field: &'static str, message: &str) -> FormError {
    FormError {
        field,
        message: message.to_owned(),
    }
}

fn decimal(field: &'static str, value: &str) -> Result<String, FormError> {
    let value = value.trim();
    if value.len() > 64 || !value.parse::<f64>().is_ok_and(f64::is_finite) {
        return Err(form_error(
            field,
            "Enter a finite number using a decimal point.",
        ));
    }
    Ok(value.to_owned())
}

/// Draft of one drawn-line command. Clicks only fill these fields; Apply publishes once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawLineForm {
    /// Draw, reshape, relabel or remove, using the existing headless command.
    pub action: String,
    /// U or V for a new line.
    pub family: String,
    /// Existing AxisId for an edit.
    pub axis: String,
    /// Explicit new label.
    pub label: String,
    /// World XY endpoints in millimetres.
    pub points: [[String; 2]; 2],
    /// Next click fills the first or second endpoint; absent when not drawing.
    pub click: Option<bool>,
}

impl Default for DrawLineForm {
    fn default() -> Self {
        Self {
            action: "draw-grid-line".into(),
            family: "u".into(),
            axis: String::new(),
            label: "D1".into(),
            points: [["0".into(), "0".into()], ["3000".into(), "4000".into()]],
            click: None,
        }
    }
}

impl DrawLineForm {
    /// Build one command against an existing grid, or create a grid with one drawn line.
    /// All engineering admission remains in the engine.
    pub fn prepare(
        &self,
        grid: Option<&str>,
        form: &GridForm,
        store: &Path,
        actor: Option<&str>,
    ) -> Result<Vec<String>, FormError> {
        let store = store
            .to_str()
            .ok_or_else(|| form_error("Model", "The model path must be UTF-8."))?;
        if !matches!(self.family.as_str(), "u" | "v") {
            return Err(form_error("Family", "Choose U or V."));
        }
        let drawing = self.action == "draw-grid-line";
        if !matches!(
            self.action.as_str(),
            "draw-grid-line"
                | "reshape-grid-line"
                | "relabel-grid-line"
                | "remove-grid-line"
                | "update-grid"
        ) || grid.is_none() && !drawing
        {
            return Err(form_error("Line", "Choose a supported line operation."));
        }
        let mut args = vec![
            if grid.is_some() {
                self.action.clone()
            } else {
                "create-grid".into()
            },
            "--store".into(),
            store.into(),
        ];
        if let Some(id) = grid {
            ElementId::parse(id).map_err(|_| form_error("Grid", "Select a grid."))?;
            args.extend(["--id".into(), id.into()]);
            if self.action == "update-grid" {
                if form.name.trim().is_empty() || form.name.len() > 256 {
                    return Err(form_error(
                        "Name",
                        "Enter a grid name of at most 256 UTF-8 bytes.",
                    ));
                }
                args.extend([
                    "--name".into(),
                    form.name.trim().into(),
                    "--origin".into(),
                    [
                        decimal("Origin X", &form.origin[0])?,
                        decimal("Origin Y", &form.origin[1])?,
                    ]
                    .join(","),
                    "--rotation".into(),
                    decimal("Rotation", &form.rotation)?,
                ]);
            } else if drawing {
                args.extend(["--family".into(), self.family.clone()]);
            } else {
                ElementId::parse(&self.axis)
                    .map_err(|_| form_error("Axis", "Select an existing axis."))?;
                args.extend(["--axis-id".into(), self.axis.clone()]);
            }
        } else {
            if form.name.trim().is_empty() || form.name.len() > 256 {
                return Err(form_error(
                    "Name",
                    "Enter a grid name of at most 256 UTF-8 bytes.",
                ));
            }
            args.extend([
                "--name".into(),
                form.name.trim().into(),
                "--origin".into(),
                [
                    decimal("Origin X", &form.origin[0])?,
                    decimal("Origin Y", &form.origin[1])?,
                ]
                .join(","),
                "--rotation".into(),
                decimal("Rotation", &form.rotation)?,
            ]);
        }
        let key = |suffix| {
            if grid.is_none() {
                format!("--{}-{suffix}", self.family)
            } else {
                format!("--{suffix}")
            }
        };
        if drawing || self.action == "relabel-grid-line" {
            beamcraft_model::grid::validate_grid_labels(std::slice::from_ref(&self.label))
                .map_err(|_| form_error("Label", "Enter a valid unique axis label."))?;
            args.extend([key("label"), self.label.clone()]);
        }
        if drawing || self.action == "reshape-grid-line" {
            for (suffix, point) in ["from", "to"].into_iter().zip(&self.points) {
                args.extend([
                    key(suffix),
                    [decimal("X", &point[0])?, decimal("Y", &point[1])?].join(","),
                ]);
            }
        }
        push_actor(&mut args, actor);
        Ok(args)
    }
}

fn start_coordinate(value: &str) -> Result<String, FormError> {
    let value = decimal("Line start", value)?;
    let distance = value
        .parse::<f64>()
        .map_err(|_| form_error("Line start", "Enter a number."))?;
    if distance < 0.0 {
        return Err(form_error(
            "Line start",
            "Enter a positive distance or zero.",
        ));
    }
    Ok((-distance).to_string())
}

fn display_rotation(radians: f64) -> String {
    let rounded = format!("{:.3}", radians.to_degrees());
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".into()
    } else {
        trimmed.into()
    }
}

impl GridForm {
    /// Prepare only changed common properties, including for a fully drawn grid.
    pub fn prepare_properties(
        &self,
        original: &Self,
        id: &str,
        store: &Path,
        actor: Option<&str>,
    ) -> Result<Vec<String>, FormError> {
        let prepared = DrawLineForm {
            action: "update-grid".into(),
            ..DrawLineForm::default()
        }
        .prepare(Some(id), self, store, actor)?;
        let mut args = vec!["update-grid".into()];
        let mut changed = false;
        for [key, value] in prepared.get(1..).unwrap_or_default().as_chunks::<2>().0 {
            let include = match key.as_str() {
                "--name" => self.name.trim() != original.name.trim(),
                "--origin" => self.origin != original.origin,
                "--rotation" => self.rotation != original.rotation,
                _ => true,
            };
            if include {
                changed |= matches!(key.as_str(), "--name" | "--origin" | "--rotation");
                args.extend([key.clone(), value.clone()]);
            }
        }
        if !changed {
            return Err(form_error("Grid", "No properties have changed."));
        }
        Ok(args)
    }

    /// Prepares a regular-grid update, sending only fields changed in the properties panel.
    /// Unchanged frames and labels retain their exact engine-authored values.
    pub fn prepare_update(
        &self,
        original: &Self,
        id: &str,
        store: &Path,
        actor: Option<&str>,
    ) -> Result<Vec<String>, FormError> {
        if ElementId::parse(id).is_err() {
            return Err(form_error(
                "Grid",
                "The selected grid has an invalid identity.",
            ));
        }
        let mut normalized = self.clone();
        for (family, numeric) in [(&mut normalized.u, true), (&mut normalized.v, false)] {
            if !family.expression.trim().is_empty() && family.labels.trim().is_empty() {
                let (_, positions) = parse_grid_expression(family.expression.trim())
                    .map_err(|_| form_error("Coordinates", "Enter valid grid coordinates."))?;
                family.labels = beamcraft_model::grid::generate_grid_labels(
                    if numeric { "numeric" } else { "alpha" },
                    positions.len(),
                    None,
                    None,
                    beamcraft_model::grid::GridLabelOrder::Ascending,
                )
                .map_err(|_| form_error("Labels", "Automatic labels could not be generated."))?
                .join(" ");
            }
        }
        let prepared = normalized.prepare(store, actor)?;
        let mut args = vec!["update-grid".into(), "--id".into(), id.to_owned()];
        let (_, tail) = prepared
            .split_first()
            .ok_or_else(|| form_error("Grid", "The grid request is empty."))?;
        for [key, value] in tail.as_chunks::<2>().0 {
            let changed = match key.as_str() {
                "--name" => self.name.trim() != original.name.trim(),
                "--origin" => {
                    self.origin.clone().map(|n| n.trim().parse::<f64>().ok())
                        != original
                            .origin
                            .clone()
                            .map(|n| n.trim().parse::<f64>().ok())
                }
                "--rotation" => {
                    self.rotation.trim().parse::<f64>().ok()
                        != original.rotation.trim().parse::<f64>().ok()
                }
                "--u-expression" | "--u-labels" => {
                    self.u.expression != original.u.expression || self.u.labels != original.u.labels
                }
                "--v-expression" | "--v-labels" => {
                    self.v.expression != original.v.expression || self.v.labels != original.v.labels
                }
                "--u-span" => self.u.span != original.u.span,
                "--v-span" => self.v.span != original.v.span,
                _ => true,
            };
            if changed {
                args.extend([key.clone(), value.clone()]);
            }
        }
        for (family, old) in [(&self.u, &original.u), (&self.v, &original.v)] {
            if family.expression.trim().is_empty() && !old.expression.is_empty() {
                return Err(form_error(
                    "Coordinates",
                    "A regular family needs at least one coordinate.",
                ));
            }
        }
        if self.u.expression != original.u.expression || self.v.expression != original.v.expression
        {
            args.extend(["--fit-spans".into(), "true".into()]);
        }
        if !args.iter().any(|key| {
            matches!(
                key.as_str(),
                "--name"
                    | "--origin"
                    | "--rotation"
                    | "--u-expression"
                    | "--v-expression"
                    | "--u-span"
                    | "--v-span"
            )
        }) {
            return Err(form_error("Grid", "No properties have changed."));
        }
        Ok(args)
    }

    /// Builds the real `create-grid` arguments. No model is read or changed here.
    pub fn prepare(&self, store: &Path, actor: Option<&str>) -> Result<Vec<String>, FormError> {
        let name = self.name.trim();
        if name.is_empty() || name.len() > 256 {
            return Err(form_error(
                "Name",
                "Enter a grid name of at most 256 UTF-8 bytes.",
            ));
        }
        let store = store
            .to_str()
            .ok_or_else(|| form_error("Model", "The model path must be UTF-8."))?;
        let origin = [
            decimal("Origin X", &self.origin[0])?,
            decimal("Origin Y", &self.origin[1])?,
        ]
        .join(",");
        let mut args = vec![
            "create-grid".into(),
            "--store".into(),
            store.into(),
            "--name".into(),
            name.into(),
            "--origin".into(),
            origin,
            "--rotation".into(),
            decimal("Rotation", &self.rotation)?,
        ];
        for (prefix, family, field, labels_field) in [
            ("u", &self.u, "U coordinates", "U labels"),
            ("v", &self.v, "V coordinates", "V labels"),
        ] {
            let expression = family.expression.trim();
            if expression.is_empty() {
                if !family.labels.trim().is_empty()
                    || family.span.iter().any(|s| !s.trim().is_empty())
                {
                    return Err(form_error(
                        field,
                        "Coordinates are required when labels or line limits are supplied.",
                    ));
                }
                continue;
            }
            let (normalized, positions) = parse_grid_expression(expression).map_err(|_| form_error(field, "Enter a start coordinate followed by positive spacings, for example 0 2*4000. At most 200 lines are allowed."))?;
            args.extend([format!("--{prefix}-expression"), normalized]);
            if !family.labels.trim().is_empty() {
                let labels = parse_grid_labels(&family.labels, positions.len()).map_err(|error| {
                    let message = match error {
                        GridLabelError::Count => {
                            "Enter a starting label, or leave Labels blank for automatic labels."
                        }
                        GridLabelError::Duplicate => "Each axis needs a unique label.",
                        GridLabelError::Invalid | GridLabelError::Limit =>
                            "Use labels of up to 16 letters or digits, with -, _, ., + or /. Separate labels with spaces or commas.",
                    };
                    form_error(labels_field, message)
                })?;
                args.extend([format!("--{prefix}-labels"), labels.join(" ")]);
            }
            let [start, end] = &family.span;
            if !start.trim().is_empty() || !end.trim().is_empty() {
                args.extend([
                    format!("--{prefix}-span"),
                    [start_coordinate(start)?, decimal("Span end", end)?].join(","),
                ]);
            }
        }
        if self.u.expression.trim().is_empty() && self.v.expression.trim().is_empty() {
            return Err(form_error("Coordinates", "Enter at least one grid family."));
        }
        push_actor(&mut args, actor);
        Ok(args)
    }
}

/// Prepare a standalone repair of regular grid extents without changing coordinates or labels.
pub fn fit_args(store: &Path, id: &str, actor: Option<&str>) -> Result<Vec<String>, FormError> {
    if ElementId::parse(id).is_err() {
        return Err(form_error(
            "Grid",
            "The selected grid has an invalid identity.",
        ));
    }
    let store = store
        .to_str()
        .ok_or_else(|| form_error("Model", "The model path must be UTF-8."))?;
    let mut args = vec![
        "update-grid".into(),
        "--store".into(),
        store.into(),
        "--id".into(),
        id.into(),
        "--fit-spans".into(),
        "true".into(),
    ];
    push_actor(&mut args, actor);
    Ok(args)
}

/// An accepted grid creation or property update, returned by the real CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedGrid {
    /// Identity of the created or updated grid.
    pub id: String,
    /// Revision containing it.
    pub revision: RevisionStamp,
}

/// Reads a grid command reply while preserving engine refusals.
pub fn parse_created(exit: Option<i32>, bytes: &[u8]) -> Result<CreatedGrid, ToolError> {
    let json = decode(bytes)?;
    if let Some(mut error) = rejection(&json, exit) {
        let message = match error.message.as_str() {
            "GRID_LABEL_DUPLICATE" => Some(
                "Axis labels must be unique across both grid families. Choose different U and V labels.",
            ),
            "GRID_LABEL_COUNT" => Some("Enter exactly one label for each grid line."),
            "GRID_FAMILY_MODE" => {
                Some("This edit cannot change a family between regular axes and drawn lines.")
            }
            "GRID_IN_USE" => {
                Some("Some axes are referenced by other model objects and cannot be removed.")
            }
            "GRID_AXIS_ID_COUNT" => Some(
                "The grid update could not allocate the required axis identities. Reload its properties and retry.",
            ),
            "GRID_LINE_DEGENERATE" => Some("The line end must be at least 1 mm beyond its start."),
            "GRID_LABEL_INVALID" => {
                Some("Use axis labels of up to 16 letters or digits, with -, _, ., + or /.")
            }
            "GRID_SPACING_INVALID" | "GRID_SPACING_MM" => {
                Some("Each spacing must be at least 1 mm.")
            }
            "GRID_EXPRESSION_INVALID" => {
                Some("Enter a start coordinate followed by positive spacings.")
            }
            "GRID_LINES_PER_FAMILY" => Some("A grid family can contain at most 200 lines."),
            "GRID_COORDINATE_CAP_MM" => {
                Some("Grid coordinates must stay within 1,000,000 mm of the origin.")
            }
            _ => None,
        };
        if let Some(message) = message {
            error.message = message.to_owned();
        }
        return Err(error);
    }
    let bad = || {
        ToolError::local(
            "UNREADABLE_REPLY",
            "The grid command returned an incomplete reply.",
        )
    };
    if !matches!(
        text(&json, "outcome").as_deref(),
        Some(
            "grid_created"
                | "grid_created_replayed"
                | "grid_updated"
                | "grid_update_replayed"
                | "grid_line_drawn"
                | "grid_line_drawn_replayed"
                | "grid_line_reshaped"
                | "grid_line_reshaped_replayed"
                | "grid_line_removed"
                | "grid_line_removed_replayed"
                | "grid_line_relabelled"
                | "grid_line_relabelled_replayed"
        )
    ) {
        return Err(bad());
    }
    let id = text(&json, "element_id")
        .filter(|id| ElementId::parse(id).is_ok())
        .ok_or_else(bad)?;
    let revision = json
        .get("revision")
        .ok()
        .and_then(RevisionStamp::from_json)
        .ok_or_else(bad)?;
    Ok(CreatedGrid { id, revision })
}

/// Executes one prepared grid request; call on a worker thread.
pub fn commit(cli: &Path, args: &[String], stats: &Stats) -> Result<CreatedGrid, ToolError> {
    let out = command::run(cli, args, command::DEADLINE, stats)?;
    parse_created(out.code, &out.stdout)
}

/// One label and its engine-computed world segment.
#[derive(Debug, Clone, PartialEq)]
pub struct GridLine {
    /// Axis label.
    pub label: String,
    /// World endpoints in millimetres.
    pub points: [[f64; 3]; 2],
}

/// Read-only display data from the same pinned revision as the meshes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridScene {
    /// World segments, bounded by the grid model limits.
    pub lines: Vec<GridLine>,
    /// Grid identities and the source properties belonging to those guide segments.
    pub grids: Vec<GridProperties>,
    /// Bounded grid records for the engine's snap query, pinned with the displayed scene.
    pub query_grids: Vec<beamcraft_model::grid::Grid>,
}

/// Properties read from the same engine inspection as the displayed guides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridProperties {
    /// Stable model identity.
    pub id: String,
    /// Source fields, not reconstructed from projected endpoints.
    pub form: GridForm,
    /// Whether all nonempty families are regular and supported by this editor.
    pub editable: bool,
    /// Range of this grid's segments in [`GridScene::lines`].
    pub lines: std::ops::Range<usize>,
}

impl GridScene {
    /// Reads a bounded `list-grids`-shaped reply. An absent legacy frame means no grids.
    pub fn from_reply(bytes: &[u8]) -> Result<Self, ToolError> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        let json = decode(bytes)?;
        if let Some(error) = rejection(&json, Some(0)) {
            return Err(error);
        }
        let bad = || {
            ToolError::local(
                "UNREADABLE_REPLY",
                "The model returned invalid grid display data.",
            )
        };
        if text(&json, "outcome").as_deref() != Some("grids") {
            return Err(bad());
        }
        let grids = json.get("grids").and_then(Json::array).map_err(|_| bad())?;
        if grids.len() > MAX_GRIDS {
            return Err(bad());
        }
        let mut lines = Vec::new();
        let mut properties = Vec::new();
        let mut query_grids = Vec::new();
        let mut identities = std::collections::BTreeSet::new();
        for grid in grids {
            let initial_count = lines.len();
            let id = text(grid, "id")
                .filter(|id| ElementId::parse(id).is_ok() && identities.insert(id.clone()))
                .ok_or_else(bad)?;
            let name = text(grid, "name")
                .filter(|name| !name.is_empty() && name.len() <= 256)
                .ok_or_else(bad)?;
            let origin = pair(grid, "origin_mm").ok_or_else(bad)?;
            let rotation = grid
                .get("rotation_rad")
                .and_then(Json::float)
                .map_err(|_| bad())?;
            if origin.iter().any(|n| n.abs() > 1e9)
                || !rotation.is_finite()
                || rotation.abs() > std::f64::consts::TAU + 1e-9
            {
                return Err(bad());
            }
            let mut form = GridForm {
                name,
                origin: origin.map(|n| n.to_string()),
                rotation: display_rotation(rotation),
                ..GridForm::default()
            };
            let mut editable = true;
            let mut labels = Vec::new();
            let mut query_grid = beamcraft_model::grid::Grid {
                id: ElementId::parse(&id).map_err(|_| bad())?,
                name: form.name.clone(),
                translation_mm: origin,
                orientation_zw: [(rotation / 2.0).sin(), (rotation / 2.0).cos()],
                u: beamcraft_model::grid::GridFamily::Drawn(Vec::new()),
                v: beamcraft_model::grid::GridFamily::Drawn(Vec::new()),
            };
            for family in ["u", "v"] {
                let family_start = labels.len();
                let family_json = grid.get(family).map_err(|_| bad())?;
                let axes = grid
                    .get(family)
                    .and_then(|f| f.get("lines"))
                    .and_then(Json::array)
                    .map_err(|_| bad())?;
                if axes.len() > MAX_GRID_LINES_PER_FAMILY {
                    return Err(bad());
                }
                let mut query_lines = Vec::new();
                for axis in axes {
                    let label = text(axis, "label")
                        .filter(|s| s.len() <= 64 && s.chars().count() <= 16 && !s.is_empty())
                        .ok_or_else(bad)?;
                    labels.push(label.clone());
                    let point = |key| -> Result<[f64; 3], ToolError> {
                        let coords = axis.get(key).and_then(Json::array).map_err(|_| bad())?;
                        let [x, y, z] = coords else {
                            return Err(bad());
                        };
                        let point = [
                            x.float().map_err(|_| bad())?,
                            y.float().map_err(|_| bad())?,
                            z.float().map_err(|_| bad())?,
                        ];
                        if point.iter().any(|n| !n.is_finite() || n.abs() > 1e7) || point[2] != 0.0
                        {
                            return Err(bad());
                        }
                        Ok(point)
                    };
                    let points = [point("start_mm")?, point("end_mm")?];
                    let local = |key, p: [f64; 3]| {
                        let result = if axis.get(key).is_ok() {
                            pair(axis, key).ok_or_else(bad)?
                        } else {
                            beamcraft_engine::grid_edit::world_to_local(&query_grid, [p[0], p[1]])
                        };
                        let world =
                            beamcraft_engine::grid_query::local_to_world(&query_grid, result);
                        if (world[0] - p[0]).abs() > 1e-6 || (world[1] - p[1]).abs() > 1e-6 {
                            return Err(bad());
                        }
                        Ok(result)
                    };
                    query_lines.push(beamcraft_model::grid::DrawnGridLine {
                        axis_id: ElementId::parse(&text(axis, "axis_id").ok_or_else(bad)?)
                            .map_err(|_| bad())?,
                        label: label.clone(),
                        start_mm: local("start_local_mm", points[0])?,
                        end_mm: local("end_local_mm", points[1])?,
                    });
                    lines.push(GridLine { label, points });
                }
                let family_form = match text(family_json, "mode").as_deref() {
                    Some("regular") => {
                        let expression = text(family_json, "expression").ok_or_else(bad)?;
                        let (_, positions) =
                            parse_grid_expression(&expression).map_err(|_| bad())?;
                        let span = pair(family_json, "span_mm").ok_or_else(bad)?;
                        if positions.len() != axes.len()
                            || span.iter().any(|n| n.abs() > 1e6)
                            || span[1] - span[0] < 1.0
                        {
                            return Err(bad());
                        }
                        FamilyForm {
                            expression,
                            labels: labels.get(family_start..).ok_or_else(bad)?.join(" "),
                            span: [span[0].abs().to_string(), span[1].to_string()],
                        }
                    }
                    Some("drawn") => {
                        editable &= axes.is_empty();
                        FamilyForm {
                            expression: String::new(),
                            labels: String::new(),
                            span: [String::new(), String::new()],
                        }
                    }
                    _ => return Err(bad()),
                };
                if family == "u" {
                    form.u = family_form;
                    query_grid.u = beamcraft_model::grid::GridFamily::Drawn(query_lines);
                } else {
                    form.v = family_form;
                    query_grid.v = beamcraft_model::grid::GridFamily::Drawn(query_lines);
                }
            }
            if lines.len() == initial_count
                || beamcraft_model::grid::validate_grid_labels(&labels).is_err()
            {
                return Err(bad());
            }
            query_grids.push(query_grid);
            properties.push(GridProperties {
                id,
                form,
                editable,
                lines: initial_count..lines.len(),
            });
        }
        beamcraft_engine::grid_query::validate(&query_grids).map_err(|_| bad())?;
        Ok(Self {
            lines,
            grids: properties,
            query_grids,
        })
    }

    /// Finds an identity in the displayed revision.
    pub fn get(&self, id: &str) -> Option<&GridProperties> {
        self.grids.iter().find(|grid| grid.id == id)
    }

    /// Picks a visible guide in screen pixels. This is display hit testing of engine endpoints.
    pub fn hit_test(
        &self,
        rect: egui::Rect,
        view: &View,
        origin: [f64; 3],
        point: egui::Pos2,
    ) -> Option<&GridProperties> {
        if !rect.contains(point) {
            return None;
        }
        let mut nearest = 8.0_f32;
        let mut hit = None;
        for grid in &self.grids {
            for line in self.lines.get(grid.lines.clone()).unwrap_or_default() {
                let [Some(a), Some(b)] = line.points.map(|p| project(p, rect, view, origin)) else {
                    continue;
                };
                let delta = b - a;
                let t = if delta.length_sq() > 1e-8 {
                    ((point - a).dot(delta) / delta.length_sq()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let mut distance = point.distance(a + delta * t);
                if rect.shrink(14.0).contains(a) && point.distance(a) <= 14.0
                    || rect.shrink(14.0).contains(b) && point.distance(b) <= 14.0
                {
                    distance = 0.0;
                }
                if distance < nearest {
                    nearest = distance;
                    hit = Some(grid);
                }
            }
        }
        hit
    }

    /// Fits the camera to the union of engineering bounds and grid guides, in scene coordinates.
    pub fn framing(&self, scene: &SceneSummary) -> ([f32; 3], f32) {
        if self.lines.is_empty() {
            return scene.framing();
        }
        let mut low = if scene.elements == 0 {
            [f32::INFINITY; 3]
        } else {
            scene.min
        };
        let mut high = if scene.elements == 0 {
            [f32::NEG_INFINITY; 3]
        } else {
            scene.max
        };
        for line in &self.lines {
            for point in line.points {
                for (((lo, hi), value), origin) in low
                    .iter_mut()
                    .zip(&mut high)
                    .zip(point)
                    .zip(scene.origin_mm)
                {
                    let value = (value - origin) as f32;
                    *lo = lo.min(value);
                    *hi = hi.max(value);
                }
            }
        }
        let centre = std::array::from_fn(|i| {
            (low.get(i).copied().unwrap_or(0.0) + high.get(i).copied().unwrap_or(0.0)) / 2.0
        });
        let extent = low
            .iter()
            .zip(high)
            .map(|(a, b)| (b - a) * (b - a))
            .sum::<f32>()
            .sqrt()
            .max(1.0);
        (centre, extent)
    }

    /// Paints construction guides and labels using engine endpoints, without authoring geometry.
    pub fn paint(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        view: &View,
        origin: [f64; 3],
        selected: Option<&str>,
    ) {
        let painter = ui.painter_at(rect);
        let palette = theme::Palette::current(ui);
        let selected_lines = selected
            .and_then(|id| self.get(id))
            .map(|g| g.lines.clone());
        for (index, line) in self.lines.iter().enumerate() {
            let active = selected_lines
                .as_ref()
                .is_some_and(|range| range.contains(&index));
            let color = if active {
                theme::BRAND
            } else {
                palette.link.gamma_multiply(0.6)
            };
            let [Some(a), Some(b)] = line.points.map(|p| project(p, rect, view, origin)) else {
                continue;
            };
            painter.line_segment(
                [a, b],
                egui::Stroke::new(if active { 2.5 } else { 1.0 }, color),
            );
            for end in [a, b] {
                if rect.shrink(14.0).contains(end) {
                    let galley = painter.layout_no_wrap(
                        line.label.clone(),
                        egui::FontId::proportional(12.0),
                        palette.text,
                    );
                    let size = (galley.size() + egui::vec2(10.0, 6.0)).max(egui::vec2(24.0, 24.0));
                    let label_rect = egui::Rect::from_center_size(end, size);
                    painter.rect_filled(
                        label_rect,
                        12,
                        if active {
                            palette.select
                        } else {
                            palette.surface
                        },
                    );
                    painter.rect_stroke(
                        label_rect,
                        12,
                        egui::Stroke::new(1.0, palette.link),
                        egui::StrokeKind::Inside,
                    );
                    painter.galley(end - galley.size() / 2.0, galley, palette.text);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod form_tests {
    use super::*;

    #[test]
    fn start_distance_is_positive_in_the_form_and_negative_in_the_command() {
        let mut form = GridForm::default();
        form.u.span = ["2500".into(), "12000".into()];
        let args = form.prepare(Path::new("model"), None).unwrap();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--u-span", "-2500,12000"])
        );
        form.u.span[0] = "-2500".into();
        assert!(form.prepare(Path::new("model"), None).is_err());
        form.u.span[0] = "0".into();
        assert!(form.prepare(Path::new("model"), None).is_ok());
    }

    #[test]
    fn rotation_display_has_at_most_three_decimal_places() {
        assert_eq!(display_rotation(45.00000000000001_f64.to_radians()), "45");
        assert_eq!(display_rotation(12.3456789_f64.to_radians()), "12.346");
        assert_eq!(display_rotation(-0.00001_f64.to_radians()), "0");
    }

    #[test]
    fn drawn_grid_name_edit_preserves_the_unedited_precise_frame() {
        let original = GridForm {
            rotation: "12.346".into(),
            ..GridForm::default()
        };
        let mut form = original.clone();
        form.name = "Renamed".into();
        let args = form
            .prepare_properties(
                &original,
                "01990000-0000-7000-8000-000000001001",
                Path::new("model"),
                None,
            )
            .unwrap();
        assert!(args.windows(2).any(|p| p == ["--name", "Renamed"]));
        assert!(
            !args
                .iter()
                .any(|p| matches!(p.as_str(), "--origin" | "--rotation"))
        );
        assert!(
            original
                .prepare_properties(
                    &original,
                    "01990000-0000-7000-8000-000000001001",
                    Path::new("model"),
                    None
                )
                .is_err()
        );
    }
}
