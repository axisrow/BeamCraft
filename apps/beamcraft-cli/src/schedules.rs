// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! I05 adapter: schedule template decoding and neutral rows from one verified revision.
//!
//! Maps [`crate::bom::rows`]' report rows and the I04 attribute state to engine input rows;
//! the filter rule decodes into the engine's own [`beamcraft_engine::schedule::Rule`], per
//! [the design note](../../../docs/design/i05-schedules.md).

use crate::attribute::{AttrModel, attr_model_of};
use crate::bom;
use crate::plate::{self, Inspection, PlateError};
use beamcraft_engine::bom::BomRow;
use beamcraft_engine::schedule::{
    CellKind, CellValue, Column, Field, InputRow, MAX_COLUMNS, Rule, StandardField, Template,
};
use beamcraft_host::revision::RevisionStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::attribute::{AttrType, AttrValue};
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeMap;

fn invalid(message: &'static str) -> PlateError {
    PlateError::new(ErrorCode::InvalidArgument, message)
}

/// One template column plus the attribute name an attribute column resolved to, if any; the
/// engine column carries only the value type, while the cell filling needs the identity.
#[derive(Debug, Clone)]
pub struct ColumnSpec {
    /// The engine column.
    pub column: Column,
    /// The resolved definition name of an attribute column; `None` for standard columns and
    /// for the unknown definitions a schedule keeps visible.
    pub attribute: Option<String>,
}

/// The attribute layer's scalar as a neutral schedule cell.
fn cell(value: &AttrValue) -> CellValue {
    match value {
        AttrValue::Text(text) | AttrValue::Choice(text) => CellValue::Text(text.clone()),
        AttrValue::Number(v) => CellValue::Number(*v),
        AttrValue::Integer(v) => CellValue::Integer(i64::from(*v)),
        AttrValue::Boolean(v) => CellValue::Boolean(*v),
    }
}

/// The attribute layer's type token as a neutral schedule value type; a choice compares as
/// its text.
fn cell_kind(kind: AttrType) -> CellKind {
    match kind {
        AttrType::Text | AttrType::Choice => CellKind::Text,
        AttrType::Number => CellKind::Number,
        AttrType::Integer => CellKind::Integer,
        AttrType::Boolean => CellKind::Boolean,
    }
}

/// The pinned attribute state of the head, with the exact revision it was read from, so the
/// command can require the same revision as the report rows.
pub fn pinned(store: &RevisionStore) -> Result<(AttrModel, String), PlateError> {
    let revision = store.open().map_err(PlateError::from)?;
    let id = plate::summarize(revision.info()).revision_id;
    let base = plate::base_of(&revision)?;
    let model = attr_model_of(&plate::open_base(&base)?, &plate::control())?;
    Ok((model, id))
}

/// Decodes and admits the template JSON against one pinned attribute state. An attribute
/// column whose definition is unknown renders empty and is named in `unknown_columns`; every
/// other template defect refuses.
pub fn template(
    json: &Json,
    model: &AttrModel,
) -> Result<(Template, Vec<ColumnSpec>, Vec<String>), PlateError> {
    let mut admitted = vec!["version", "columns"];
    for optional in ["filter", "group_by", "sort_by"] {
        if json.get(optional).is_ok() {
            admitted.push(optional);
        }
    }
    json.closed(&admitted)
        .map_err(|_| invalid("schedule template has unknown fields"))?;
    if json
        .get("version")
        .and_then(Json::unsigned)
        .map_err(|_| invalid("schedule template version is missing"))?
        != 1
    {
        return Err(PlateError::new(
            ErrorCode::UnsupportedVersion,
            "unsupported schedule template version",
        ));
    }
    let declared = json
        .get("columns")
        .and_then(Json::array)
        .map_err(|_| invalid("schedule template columns are required"))?;
    if declared.is_empty() || declared.len() > MAX_COLUMNS {
        return Err(invalid("a schedule needs one to 64 columns"));
    }
    let mut columns = Vec::with_capacity(declared.len());
    let mut names = Vec::with_capacity(declared.len());
    let mut unknown_columns = Vec::new();
    for declared in declared {
        if declared.get("column").is_err() && declared.get("attribute").is_err() {
            return Err(invalid("a column reads a standard token or an attribute"));
        }
        let standard = declared.get("column").is_ok();
        let mut allowed = vec![if standard { "column" } else { "attribute" }];
        if declared.get("total").is_ok() {
            allowed.push("total");
        }
        declared
            .closed(&allowed)
            .map_err(|_| invalid("schedule column has unknown fields"))?;
        let total = match declared.get("total") {
            Ok(flag) => flag
                .boolean()
                .map_err(|_| invalid("schedule column total must be a boolean"))?,
            Err(_) => false,
        };
        let (field, label, name) = if standard {
            let token = declared
                .get("column")
                .and_then(Json::string)
                .map_err(|_| invalid("schedule column token must be text"))?;
            let field =
                StandardField::parse(token).ok_or_else(|| invalid("unknown standard column"))?;
            (Field::Standard(field), field.token().to_owned(), None)
        } else {
            let selector = declared
                .get("attribute")
                .and_then(Json::string)
                .map_err(|_| invalid("schedule attribute selector must be text"))?;
            match model.schema.get(selector) {
                Some(def) => (
                    Field::Attribute(cell_kind(def.ty)),
                    def.name.clone(),
                    Some(def.name.clone()),
                ),
                None => {
                    unknown_columns.push(selector.to_owned());
                    (Field::Unknown, selector.to_owned(), None)
                }
            }
        };
        names.push(name);
        columns.push(Column {
            label,
            field,
            total,
        });
    }
    let keys: Vec<String> = columns.iter().map(|c| c.label.clone()).collect();
    let group_by = indices(json, "group_by", &keys)?;
    let sort_by = indices(json, "sort_by", &keys)?;
    let specs = columns
        .iter()
        .zip(names)
        .map(|(column, attribute)| ColumnSpec {
            column: column.clone(),
            attribute,
        })
        .collect();
    Ok((
        Template {
            columns,
            group_by,
            sort_by,
        },
        specs,
        unknown_columns,
    ))
}

/// Resolves an optional key list to column indices: first matching column wins.
fn indices(json: &Json, key: &str, keys: &[String]) -> Result<Vec<usize>, PlateError> {
    let Ok(list) = json.get(key) else {
        return Ok(Vec::new());
    };
    let list = list
        .array()
        .map_err(|_| invalid("schedule keys must be an array"))?;
    let mut out = Vec::with_capacity(list.len());
    for entry in list {
        let name = entry
            .string()
            .map_err(|_| invalid("schedule keys must be text"))?;
        let index = keys
            .iter()
            .position(|k| k == name)
            .ok_or_else(|| invalid("schedule keys name known columns"))?;
        if out.contains(&index) {
            return Err(invalid("schedule keys repeat no column"));
        }
        out.push(index);
    }
    Ok(out)
}

/// Decodes a filter rule JSON into the engine's rule over the template's columns; the
/// literal is checked against the column's value type, with no unit conversion — a number
/// compares against the stored value in the definition's unit.
pub fn decode_rule(json: &Json, columns: &[Column]) -> Result<Rule, PlateError> {
    if json.get("all").is_ok() || json.get("any").is_ok() {
        let key = if json.get("all").is_ok() {
            "all"
        } else {
            "any"
        };
        json.closed(&[key])
            .map_err(|_| invalid("filter group has unknown fields"))?;
        let list = json
            .get(key)
            .and_then(Json::array)
            .map_err(|_| invalid("filter group must be an array"))?;
        let mut rules = Vec::with_capacity(list.len());
        for entry in list {
            rules.push(decode_rule(entry, columns)?);
        }
        return Ok(if key == "all" {
            Rule::All(rules)
        } else {
            Rule::Any(rules)
        });
    }
    json.closed(&["key", "op", "value"])
        .map_err(|_| invalid("filter comparison has unknown fields"))?;
    let key = json
        .get("key")
        .and_then(Json::string)
        .map_err(|_| invalid("filter key is required"))?
        .to_owned();
    let op = beamcraft_engine::schedule::Operator::parse(
        json.get("op")
            .and_then(Json::string)
            .map_err(|_| invalid("filter operator is required"))?,
    )
    .ok_or_else(|| invalid("unknown filter operator"))?;
    let Some(column) = columns.iter().find(|column| column.label == key) else {
        return Err(invalid("a filter names known columns"));
    };
    let Some(kind) = column.field.cell_kind() else {
        return Err(invalid("a filter compares a column with values"));
    };
    let value = json
        .get("value")
        .map_err(|_| invalid("filter value is required"))?;
    // The literal admits against the column's own value type, with no unit conversion.
    let literal = match kind {
        CellKind::Text => CellValue::Text(
            value
                .string()
                .map_err(|_| invalid("filter needs a text literal"))?
                .to_owned(),
        ),
        CellKind::Number => CellValue::Number(
            value
                .float()
                .map_err(|_| invalid("filter needs a numeric literal"))?,
        ),
        CellKind::Integer => {
            let Json::Number(text) = value else {
                return Err(invalid("filter needs an exact integer"));
            };
            CellValue::Integer(
                text.parse()
                    .map_err(|_| invalid("filter integer is outside int64"))?,
            )
        }
        CellKind::Boolean => CellValue::Boolean(
            value
                .boolean()
                .map_err(|_| invalid("filter needs a boolean literal"))?,
        ),
    };
    Ok(Rule::Compare { key, op, literal })
}

/// The value type of every defined attribute name, for the cell filling's type check.
fn types(model: &AttrModel) -> BTreeMap<String, AttrType> {
    model
        .schema
        .iter()
        .map(|def| (def.name.clone(), def.ty))
        .collect()
}

/// The standard cell of one BOM row: the E03 report values as typed cells.
fn std_cell(field: StandardField, row: &BomRow) -> Option<CellValue> {
    let text = |s: &str| Some(CellValue::Text(s.to_owned()));
    let number = |v: f64| Some(CellValue::Number(v));
    match field {
        StandardField::Mark => text(&row.mark),
        StandardField::Id => text(&row.id.to_string()),
        StandardField::Kind => text(row.kind.as_token()),
        StandardField::Name => text(&row.name),
        StandardField::Profile => text(&row.profile),
        StandardField::Material => text(&row.material),
        StandardField::LengthMm => row.length_mm.and_then(number),
        StandardField::AreaMm2 => row.area_mm2.and_then(number),
        StandardField::VolumeMm3 => number(row.volume_mm3),
        StandardField::VolumeM3 => number(row.volume_mm3 / 1e9),
        StandardField::MassKg => row.mass_kg.and_then(number),
    }
}

/// One engine row per scheduled element: the BOM universe of one verified revision,
/// filtered through the admitted rule when the template carries one. A stored value whose
/// type no longer matches its definition stays an empty cell, never a default.
pub fn rows(
    source: &[BomRow],
    model: &AttrModel,
    filter: Option<&(Template, Rule)>,
    specs: &[ColumnSpec],
) -> Result<Vec<InputRow>, PlateError> {
    let defined = types(model);
    let mut out = Vec::with_capacity(source.len());
    for row in source {
        let cells: Vec<Option<CellValue>> = specs
            .iter()
            .map(|spec| match &spec.column.field {
                Field::Standard(field) => std_cell(*field, row),
                Field::Attribute(_) => spec.attribute.as_ref().and_then(|name| {
                    let ty = defined.get(name)?;
                    let value = model.values.value_of(&row.id, name)?;
                    (value.attr_type() == *ty).then(|| cell(value))
                }),
                Field::Unknown => None,
            })
            .collect();
        let input = InputRow { id: row.id, cells };
        if let Some((template, rule)) = filter
            && !rule.matches(template, &input).map_err(PlateError::from)?
        {
            continue;
        }
        out.push(input);
    }
    Ok(out)
}

/// The template JSON bound, shared by the command's bounded file read and the decoder.
pub const TEMPLATE_BOUND: usize = 65_536;

/// The template JSON bound acknowledged by the command: bounded bytes and a closed object.
pub fn decode(bytes: &[u8]) -> Result<Json, PlateError> {
    if bytes.len() > TEMPLATE_BOUND {
        return Err(invalid("the schedule template exceeds its JSON bound"));
    }
    beamcraft_json::decode(bytes, &Limits::DEFAULT)
        .map_err(|_| invalid("the schedule template is malformed"))
}

/// Decodes the template against the pinned attribute state and builds one engine row per
/// scheduled element: the BOM universe of one verified revision, filtered through the
/// engine's rule when the template carries one. Attribute cells and report rows read the
/// same revision, so quantities and attributes are consistent.
pub fn prepare(
    inspection: &Inspection,
    marks: &[beamcraft_engine::numbering_approval::StoredMark],
    model: &AttrModel,
    template_json: &Json,
) -> Result<(Template, Vec<InputRow>, Vec<String>), PlateError> {
    let (template, specs, unknown_columns) = template(template_json, model)?;
    let filter = match template_json.get("filter") {
        Ok(rule) => {
            let rule = decode_rule(rule, &template.columns)?;
            rule.admit(&template).map_err(PlateError::from)?;
            Some((template.clone(), rule))
        }
        Err(_) => None,
    };
    let source = bom::rows(inspection, marks)?;
    let input_rows = rows(&source, model, filter.as_ref(), &specs)?;
    Ok((template, input_rows, unknown_columns))
}
