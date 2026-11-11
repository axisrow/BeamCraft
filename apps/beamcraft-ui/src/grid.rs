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
    /// Authored member axes from the same inspection, including offset and non-box sections.
    pub member_axes: Vec<beamcraft_engine::query::pick::MemberSnapAxis>,
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
        let mut member_axes = Vec::new();
        if let Ok(axes) = json.get("member_axes") {
            let axes = axes.array().map_err(|_| bad())?;
            if axes.len() > beamcraft_engine::query::pick::MAX_SCANNED_ELEMENTS {
                return Err(bad());
            }
            let mut seen = std::collections::BTreeSet::new();
            for axis in axes {
                let id = text(axis, "element_id")
                    .and_then(|s| ElementId::parse(&s).ok())
                    .ok_or_else(bad)?;
                if !seen.insert(id) {
                    return Err(bad());
                }
                let points = axis
                    .get("points_mm")
                    .and_then(Json::array)
                    .map_err(|_| bad())?;
                if !(2..=beamcraft_model::limits::CONTOUR_POINT_CAP).contains(&points.len()) {
                    return Err(bad());
                }
                let points_mm = points
                    .iter()
                    .map(|p| {
                        let [x, y, z] = p.array().map_err(|_| bad())? else {
                            return Err(bad());
                        };
                        let point = [
                            x.float().map_err(|_| bad())?,
                            y.float().map_err(|_| bad())?,
                            z.float().map_err(|_| bad())?,
                        ];
                        if point.iter().any(|v| {
                            !v.is_finite() || v.abs() > beamcraft_model::limits::COORDINATE_CAP_MM
                        }) {
                            return Err(bad());
                        }
                        Ok(point)
                    })
                    .collect::<Result<Vec<_>, ToolError>>()?;
                member_axes.push(beamcraft_engine::query::pick::MemberSnapAxis {
                    element_id: id,
                    points_mm,
                });
            }
        }
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
            member_axes,
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
    /// A bubble hangs outward off each line end, past the corner its line shares with the
    /// crossing family, and a bubble that would cover one already placed is dropped: at a low
    /// pitch or in an elevation the ends pile up, and stacked labels read as one long chain
    /// (issue 562 items 35 and 38). A line seen nearly end-on owns no labels at all, so an
    /// elevation does not draw a plan grid's labels on one horizontal line. The selected grid's
    /// bubbles claim their places first. `shown` is the view the pane displays: in a vertical
    /// view, lines outside its depth slab draw and label nothing, so an elevation on grid axis
    /// B labels B and not the family's first line, which an elevation flattens onto the same
    /// screen line (issue 584 item 8). A grid line stands for its vertical grid plane, which a
    /// plan or inclined slab always crosses, so a depth-limited level plan keeps its whole grid
    /// (the 584 recheck: a level plan at 2000 with a ±500 depth lost every line). The work
    /// area is not part of the test: its box may lie strictly inside the spans while grid
    /// endpoints sit at the model limits, so requiring the endpoints inside would drop every
    /// guide from a work-area view (PR 587 review).
    pub fn paint(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        view: &View,
        origin: [f64; 3],
        selected: Option<&str>,
        shown: Option<&beamcraft_engine::views::ModelView>,
    ) {
        // A grid line stands for the vertical grid plane through it (the engine snaps to grid
        // guides the same way). A plane that is not vertical meets that grid plane at every
        // distance, so its slab always crosses it; in a vertical view the distance does not
        // depend on height, and the line's two ground endpoints carry the whole plane.
        let in_view = |points: &[[f64; 3]; 2]| {
            shown.is_none_or(|v| {
                let Some([above, below]) = v.depth else {
                    return true;
                };
                if crate::views::view_kind(v) != crate::views::ViewKind::Vertical {
                    return true;
                }
                let within = |p: &[f64; 3]| {
                    let d = v.plane_distance(*p);
                    d >= -below && d <= above
                };
                points.iter().all(within)
            })
        };
        let painter = ui.painter_at(rect);
        let palette = theme::Palette::current(ui);
        let selected_lines = selected
            .and_then(|id| self.get(id))
            .map(|g| g.lines.clone());
        for (index, line) in self.lines.iter().enumerate() {
            if !in_view(&line.points) {
                continue;
            }
            let active = selected_lines
                .as_ref()
                .is_some_and(|range| range.contains(&index));
            let color = if active {
                theme::BRAND
            } else {
                palette.link.gamma_multiply(0.6)
            };
            if let [Some(a), Some(b)] = line.points.map(|p| project(p, rect, view, origin)) {
                // A line seen end-on projects to a dot; drawing it adds nothing (a plan grid
                // in an elevation, issue 562 item 38).
                if (b - a).length_sq() >= 4.0 {
                    painter.line_segment(
                        [a, b],
                        egui::Stroke::new(if active { 2.5 } else { 1.0 }, color),
                    );
                }
            }
        }
        // Labels in a second pass, the selected grid first, so its bubbles claim their places;
        // the stable sort keeps the model order within each group.
        let mut order: Vec<usize> = (0..self.lines.len()).collect();
        order.sort_by_key(|index| {
            usize::from(
                !selected_lines
                    .as_ref()
                    .is_some_and(|range| range.contains(index)),
            )
        });
        let mut placed: Vec<egui::Rect> = Vec::new();
        for index in order {
            let Some(line) = self.lines.get(index) else {
                continue;
            };
            if !in_view(&line.points) {
                continue;
            }
            let active = selected_lines
                .as_ref()
                .is_some_and(|range| range.contains(&index));
            let [Some(a), Some(b)] = line.points.map(|p| project(p, rect, view, origin)) else {
                continue;
            };
            let toward = b - a;
            let length = toward.length();
            // A line seen nearly end-on keeps no labels: a plan grid's U lines in an elevation
            // would stamp their bubbles onto one horizontal line.
            if length < 24.0 {
                continue;
            }
            let outward = toward / length;
            let galley = painter.layout_no_wrap(
                line.label.clone(),
                egui::FontId::proportional(12.0),
                palette.text,
            );
            let size = (galley.size() + egui::vec2(10.0, 6.0)).max(egui::vec2(24.0, 24.0));
            // A bubble wider or taller than the pane (a long label in a deeply split pane)
            // cannot be clamped into it: shrink2 would invert the inner bounds and
            // `Pos2::clamp` panics (AGENTS.md section 4). Such an end keeps no label.
            if size.x > rect.width() || size.y > rect.height() {
                continue;
            }
            let inner = rect.shrink2(size * 0.5);
            for (end, ahead) in [(a, -1.0f32), (b, 1.0)] {
                if !rect.shrink(14.0).contains(end) {
                    continue;
                }
                // Outward off the end and clamped into the pane, so the bubble hangs past the
                // line end yet stays visible. The hang clears a bubble's full 24-point width
                // on one axis, so the crossing family's bubble at the same corner (offset
                // along its own line) cannot touch it.
                let centre = (end + outward * (26.0 * ahead)).clamp(inner.min, inner.max);
                let label_rect = egui::Rect::from_center_size(centre, size);
                // A shared corner or a dense family: the first label on the spot wins, so
                // nothing stacks into a chain.
                if placed.iter().any(|other| other.intersects(label_rect)) {
                    continue;
                }
                placed.push(label_rect);
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
                painter.galley(centre - galley.size() / 2.0, galley.clone(), palette.text);
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

    /// A 3 x 3 grid: U lines 1..3 along Y, V lines A..C along X, one square 4 m x 5 m.
    fn demo_scene() -> GridScene {
        GridScene {
            lines: vec![
                GridLine {
                    label: "1".into(),
                    points: [[0.0, 0.0, 0.0], [0.0, 10000.0, 0.0]],
                },
                GridLine {
                    label: "2".into(),
                    points: [[4000.0, 0.0, 0.0], [4000.0, 10000.0, 0.0]],
                },
                GridLine {
                    label: "3".into(),
                    points: [[8000.0, 0.0, 0.0], [8000.0, 10000.0, 0.0]],
                },
                GridLine {
                    label: "A".into(),
                    points: [[0.0, 0.0, 0.0], [8000.0, 0.0, 0.0]],
                },
                GridLine {
                    label: "B".into(),
                    points: [[0.0, 5000.0, 0.0], [8000.0, 5000.0, 0.0]],
                },
                GridLine {
                    label: "C".into(),
                    points: [[0.0, 10000.0, 0.0], [8000.0, 10000.0, 0.0]],
                },
            ],
            ..GridScene::default()
        }
    }

    /// The bubble rectangles one paint pass places, in `rect`-relative points.
    fn painted_bubbles(
        scene: &GridScene,
        plane: beamcraft_engine::work_plane::WorkPlane,
    ) -> Vec<egui::Rect> {
        let ctx = egui::Context::default();
        crate::app::inter(&ctx);
        crate::theme::install(&ctx);
        let view = crate::camera::Camera::aligned_to(plane, [0.0; 3], 20_000.0, 100.0)
            .unwrap()
            .view(800.0 / 600.0)
            .unwrap();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            },
            |ui| scene.paint(ui, rect, &view, [0.0; 3], None, None),
        );
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                // The bubbles are the filled rects; their strokes share the same rect.
                egui::Shape::Rect(rect_shape)
                    if rect_shape.fill != egui::Color32::TRANSPARENT
                        && rect_shape.rect.width() >= 20.0 =>
                {
                    Some(rect_shape.rect)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn plan_bubbles_never_cover_each_other_and_an_elevation_keeps_one_pair() {
        let scene = demo_scene();
        // Plan: looking straight down. Every crossing is a shared corner; the bubbles hang
        // outward and none covers another (issue 562 item 35). The ends that fall on the pane
        // edge keep none (the bubble would sit outside), the rest all do.
        let plan = beamcraft_engine::work_plane::WorkPlane::new(
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        )
        .unwrap();
        let bubbles = painted_bubbles(&scene, plan);
        assert!(
            bubbles.len() >= 6,
            "the off-edge ends keep a bubble: {bubbles:?}"
        );
        for (i, a) in bubbles.iter().enumerate() {
            for b in bubbles.iter().skip(i + 1) {
                assert!(!a.intersects(*b), "bubbles {a:?} and {b:?} overlap");
            }
        }
        // Elevation: looking along -Y. The U lines are end-on (no labels) and the V lines all
        // project onto one segment, so only the first pair of labels is kept (item 38).
        let elevation = beamcraft_engine::work_plane::WorkPlane::new(
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        )
        .unwrap();
        let bubbles = painted_bubbles(&scene, elevation);
        assert_eq!(
            bubbles.len(),
            2,
            "one pair of labels on the horizontal line: {bubbles:?}"
        );
        for (i, a) in bubbles.iter().enumerate() {
            for b in bubbles.iter().skip(i + 1) {
                assert!(!a.intersects(*b), "bubbles {a:?} and {b:?} overlap");
            }
        }
    }

    /// The label texts one paint pass draws, in paint order, with `shown` as the view the
    /// pane displays.
    fn painted_labels(
        scene: &GridScene,
        plane: beamcraft_engine::work_plane::WorkPlane,
        shown: Option<&beamcraft_engine::views::ModelView>,
    ) -> Vec<String> {
        let ctx = egui::Context::default();
        crate::app::inter(&ctx);
        crate::theme::install(&ctx);
        let view = crate::camera::Camera::aligned_to(plane, [0.0; 3], 20_000.0, 100.0)
            .unwrap()
            .view(800.0 / 600.0)
            .unwrap();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            },
            |ui| scene.paint(ui, rect, &view, [0.0; 3], None, shown),
        );
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_elevation_on_a_grid_axis_labels_that_axis_not_the_family_head() {
        // Issue 584 item 8: an elevation on grid line B flattens the parallel lines A and B
        // onto one screen line, and the first placed bubble won, so the view read A. The
        // view's depth slab excludes the family's other lines, and only B labels anything.
        let scene = demo_scene();
        let on_a = beamcraft_engine::work_plane::WorkPlane::new(
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        )
        .unwrap();
        let on_b = beamcraft_engine::work_plane::WorkPlane::new(
            [0.0, 5000.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        )
        .unwrap();
        let mut elevation_b = beamcraft_engine::views::ModelView::new(
            beamcraft_model::id::Uuid7::parse("019a7bf0-0000-7000-8000-000000000001").unwrap(),
            "Elevation B",
            on_b,
            20_000.0,
        );
        elevation_b.depth = Some([300.0, 300.0]);
        fn as_str(labels: &[String]) -> Vec<&str> {
            labels.iter().map(String::as_str).collect()
        }
        // A free view keeps the shared-corner rule: one pair, and the family head wins it.
        let labels = painted_labels(&scene, on_a, None);
        assert_eq!(
            as_str(&labels),
            ["A", "A"],
            "the uncut elevation keeps one pair: {labels:?}"
        );
        // The elevation on B shows only line B: the slab excludes A at v=0 and the U lines.
        let labels = painted_labels(&scene, on_b, Some(&elevation_b));
        assert_eq!(
            as_str(&labels),
            ["B", "B"],
            "the elevation on B labels B: {labels:?}"
        );
        // A work area is not part of the test: its box may lie strictly inside the spans
        // while grid endpoints sit at the model limits, and a box test would drop every
        // guide from such a view (PR 587 review). With no depth, a boxed view paints
        // exactly what a free view paints.
        let mut boxed = beamcraft_engine::views::ModelView::new(
            beamcraft_model::id::Uuid7::parse("019a7bf0-0000-7000-8000-000000000002").unwrap(),
            "Boxed",
            on_b,
            20_000.0,
        );
        boxed.work_area = beamcraft_engine::view_clipping::WorkArea::new(
            [2000.0, 2000.0, -1000.0],
            [7000.0, 9000.0, 1000.0],
        )
        .ok();
        let free = painted_labels(&scene, on_a, None);
        let labels = painted_labels(&scene, on_a, Some(&boxed));
        assert_eq!(
            as_str(&labels),
            as_str(&free),
            "a work-area view without a depth keeps every guide: {labels:?}"
        );
    }

    #[test]
    fn a_depth_limited_plan_keeps_the_whole_grid() {
        // The 584 recheck: the item 8 slab test dropped every line from a level plan whose
        // depth does not reach the grid's ground plane (a level at 2000 with ±500 mm), and
        // from an inclined view likewise. A grid line stands for its vertical grid plane,
        // which every plan or inclined slab crosses, so both keep what a free view paints.
        let scene = demo_scene();
        fn as_str(labels: &[String]) -> Vec<&str> {
            labels.iter().map(String::as_str).collect()
        }
        let view = |name: &str, plane, depth| {
            let mut view = beamcraft_engine::views::ModelView::new(
                beamcraft_model::id::Uuid7::parse("019a7bf0-0000-7000-8000-000000000003").unwrap(),
                name,
                plane,
                20_000.0,
            );
            view.depth = Some(depth);
            view
        };
        let level = beamcraft_engine::work_plane::WorkPlane::parallel("xy", 2000.0).unwrap();
        let plan = view("L1 plan", level, [500.0, 500.0]);
        let free = painted_labels(&scene, level, None);
        assert!(!free.is_empty(), "the free plan labels the grid: {free:?}");
        let labels = painted_labels(&scene, level, Some(&plan));
        assert_eq!(
            as_str(&labels),
            as_str(&free),
            "a depth-limited plan keeps the whole grid: {labels:?}"
        );
        let tilted = beamcraft_engine::work_plane::WorkPlane::new(
            [0.0, 0.0, 2000.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.6, 0.8],
        )
        .unwrap();
        let inclined = view("Inclined", tilted, [500.0, 500.0]);
        let free = painted_labels(&scene, tilted, None);
        assert!(
            !free.is_empty(),
            "the free inclined view labels the grid: {free:?}"
        );
        let labels = painted_labels(&scene, tilted, Some(&inclined));
        assert_eq!(
            as_str(&labels),
            as_str(&free),
            "a depth-limited inclined view keeps the whole grid: {labels:?}"
        );
        // A vertical view still tests its slab: the section on line 2 labels 2 alone.
        let on_2 = beamcraft_engine::work_plane::WorkPlane::new(
            [4000.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        )
        .unwrap();
        let section = view("Section 2", on_2, [300.0, 300.0]);
        let labels = painted_labels(&scene, on_2, Some(&section));
        assert_eq!(
            as_str(&labels),
            ["2", "2"],
            "the section on 2 labels 2: {labels:?}"
        );
    }

    #[test]
    fn a_bubble_that_cannot_fit_the_pane_is_dropped_instead_of_panicking() {
        // Issue 562 review: a label longer than a deeply split pane inverts the clamped
        // bounds, and `Pos2::clamp` panics on min > max. The paint must quietly drop what
        // cannot fit (AGENTS.md section 4).
        let scene = demo_scene();
        let ctx = egui::Context::default();
        crate::app::inter(&ctx);
        crate::theme::install(&ctx);
        let view = crate::camera::Camera::aligned_to(
            beamcraft_engine::work_plane::WorkPlane::new(
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            )
            .unwrap(),
            [0.0; 3],
            20_000.0,
            100.0,
        )
        .unwrap()
        .view(2.0)
        .unwrap();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(20.0, 10.0));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            },
            |ui| scene.paint(ui, rect, &view, [0.0; 3], None, None),
        );
        output.textures_delta.clear();
        assert!(
            output
                .shapes
                .iter()
                .all(|shape| !matches!(&shape.shape, egui::Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT)),
            "no bubble fits a 20 x 10 pane"
        );
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
