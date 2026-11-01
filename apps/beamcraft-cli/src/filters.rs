// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V14 adapter: strict rule JSON and the admitted query dataset of the verified head.
//!
//! One pinned read turns the head into engine [`Record`](beamcraft_engine::filter::Record)s:
//! every active element with its kind, name, part class and typed user-attribute values.
//! Registry definitions are not objects; inactive targets and values that do not admit under
//! their definitions acquire no defaults, so a rule can only ever match what the model actually
//! says. The same admitted dataset is what a later saved-set or view-binding command evaluates,
//! so a query and a bound view cannot disagree.

use crate::assembly::read_prims;
use crate::plate::{property, text};
use beamcraft_engine::attribute::Schema;
use beamcraft_engine::filter::{Dataset, Field, Operator, Record, Rule};
use beamcraft_host::revision::{RevisionError, RevisionStore};
use beamcraft_json::Json;
use beamcraft_layer_engine::authored::{ATTR_SCHEMA_PATH, schema_identity, validate_layers};
use beamcraft_layer_engine::{OpenRequest, Prim, Reader, Value};
use beamcraft_model::attribute::{AttrDef, AttrType, AttrValue};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, ProjectId, WorkspaceId};
use session::Refusal;
use session::pipeline::{control, integrity, load_base, open_base};
use std::collections::BTreeMap;

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// The admitted query state of the verified head.
pub struct Model {
    /// The attribute schema, in name order.
    pub schema: Schema,
    /// The admitted targets and values of the head.
    pub dataset: Dataset,
    /// Digest of the head manifest, when the store has a revision.
    pub manifest: Option<ContentDigest>,
}

/// Reads the head of the store (`None` manifest for a store without a revision).
pub fn read(store: &RevisionStore) -> Result<Model, Refusal> {
    let empty = |manifest| {
        Ok(Model {
            schema: Schema::default(),
            dataset: Dataset::new(Schema::default(), Vec::new())?,
            manifest,
        })
    };
    let revision = match store.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => return empty(None),
        Err(error) => return Err(error.into()),
    };
    let manifest = revision.info().manifest_digest();
    let content = revision.manifest().content();
    let damaged = |_| integrity("head manifest identity is not a UUIDv7");
    let project = ProjectId::parse(content.project().as_str()).map_err(damaged)?;
    let workspace = WorkspaceId::parse(content.workspace().as_str()).map_err(damaged)?;
    let (_, base) = load_base(store, &project, &workspace)?;
    let request = open_base(&base)?;
    let attrs = crate::attribute::attr_model_of(&request, &control())?;
    let prims = prims_of(&request)?;
    Ok(Model {
        dataset: Dataset::new(attrs.schema.clone(), records_of(&prims, &attrs)?)?,
        schema: attrs.schema,
        manifest: Some(manifest),
    })
}

/// The composed prims of one pinned read, verified before they are interpreted.
fn prims_of(request: &OpenRequest) -> Result<Vec<Prim>, Refusal> {
    let control = control();
    validate_layers(request.clone(), schema_identity()?, &control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), &control)?;
    read_prims(&reader, &opened.context, &control)
}

/// One record per active element: kind, name, part class and defined typed values. Inactive
/// targets and registry definitions are not objects, so they never match a filter.
fn records_of(prims: &[Prim], attrs: &crate::attribute::AttrModel) -> Result<Vec<Record>, Refusal> {
    let scope = format!("{ATTR_SCHEMA_PATH}/");
    let mut records = Vec::new();
    for prim in prims {
        let Some(raw) = prim
            .element_id
            .as_deref()
            .and_then(|id| ElementId::parse(id).ok())
        else {
            continue;
        };
        if prim.path.starts_with(&scope) {
            continue;
        }
        let kind = property(prim, "bim:elementKind")
            .and_then(text)
            .unwrap_or("unknown");
        let deactivated =
            !prim.active || property(prim, "bim:isDeactivated") == Some(&Value::Bool(true));
        if deactivated || kind == "attribute_definition" {
            continue;
        }
        let class = match kind {
            "plate" | "beam" | "column" => Some(crate::plate::decode_class(prim)?),
            _ => None,
        };
        // Only a value that fully admits under its definition participates: a presence left
        // over from an earlier definition — or a stale choice option that outlived an undefine
        // — is not a filter target, so it never matches instead of refusing the dataset.
        let mut attributes = BTreeMap::new();
        for def in attrs.schema.iter() {
            if let Some(value) = attrs.values.value_of(&raw, &def.name)
                && beamcraft_engine::filter::admits(def, value)
            {
                attributes.insert(def.name.clone(), value.clone());
            }
        }
        records.push(Record {
            id: raw,
            kind: kind.to_owned(),
            name: property(prim, "bim:name").and_then(text).map(str::to_owned),
            class,
            attributes,
        });
    }
    Ok(records)
}

/// The scalar literal for one field type; a numeric literal may pin the unit of its definition.
fn scalar(j: &Json, kind: AttrType, def: Option<&AttrDef>) -> Result<AttrValue, Refusal> {
    let j = if let Json::Object(_) = j {
        j.closed(&["value", "unit"])
            .map_err(|_| invalid("filter unit literal must contain value and unit"))?;
        let expected = def
            .and_then(|d| d.unit.as_deref())
            .ok_or_else(|| invalid("standard fields do not take unit literals"))?;
        if !matches!(kind, AttrType::Number | AttrType::Integer)
            || j.get("unit")
                .and_then(Json::string)
                .map_err(|_| invalid("filter unit must be text"))?
                != expected
        {
            return Err(invalid("filter unit differs from its attribute definition"));
        }
        j.get("value")
            .map_err(|_| invalid("filter value is missing"))?
    } else {
        j
    };
    Ok(match kind {
        AttrType::Text => AttrValue::Text(
            j.string()
                .map_err(|_| invalid("filter needs a text literal"))?
                .to_owned(),
        ),
        AttrType::Choice => AttrValue::Choice(
            j.string()
                .map_err(|_| invalid("filter needs a choice literal"))?
                .to_owned(),
        ),
        AttrType::Number => AttrValue::Number(
            j.float()
                .map_err(|_| invalid("filter needs a numeric literal"))?,
        ),
        AttrType::Integer => {
            let Json::Number(s) = j else {
                return Err(invalid("filter needs an exact integer"));
            };
            AttrValue::Integer(
                s.parse()
                    .map_err(|_| invalid("filter integer is outside int32"))?,
            )
        }
        AttrType::Boolean => {
            AttrValue::Boolean(j.boolean().map_err(|_| invalid("filter needs a boolean"))?)
        }
    })
}

/// Parse a bounded closed rule and check attribute names against the schema.
pub fn decode_rule(j: &Json, schema: &Schema) -> Result<Rule, Refusal> {
    let mut nodes = 0;
    parse(j, schema, 0, &mut nodes)
}
fn parse(j: &Json, schema: &Schema, depth: usize, nodes: &mut usize) -> Result<Rule, Refusal> {
    *nodes = nodes.saturating_add(1);
    if depth >= 8 || *nodes > 256 {
        return Err(invalid("filter exceeds 256 nodes or depth eight"));
    }
    if j.get("all").is_ok() || j.get("any").is_ok() {
        let all = j.get("all").is_ok();
        let key = if all { "all" } else { "any" };
        j.closed(&[key])
            .map_err(|_| invalid("filter group has unknown fields"))?;
        let array = j
            .get(key)
            .and_then(Json::array)
            .map_err(|_| invalid("filter group must be an array"))?;
        if array.len() > 256 {
            return Err(invalid("filter group is too large"));
        }
        let rules = array
            .iter()
            .map(|j| parse(j, schema, depth + 1, nodes))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(if all {
            Rule::All(rules)
        } else {
            Rule::Any(rules)
        });
    }
    let f = j
        .get("field")
        .and_then(Json::string)
        .map_err(|_| invalid("filter field is required"))?;
    j.closed(if f == "attribute" {
        &["field", "attribute", "op", "value"]
    } else {
        &["field", "op", "value"]
    })
    .map_err(|_| invalid("filter comparison has unknown fields"))?;
    let field = match f {
        "type" => Field::Kind,
        "name" => Field::Name,
        "class" => Field::Class,
        "attribute" => {
            let name = j
                .get("attribute")
                .and_then(Json::string)
                .map_err(|_| invalid("attribute selector is required"))?;
            if !schema.contains(name) {
                return Err(Refusal {
                    rule_id: None,
                    code: ErrorCode::NotFound,
                    message: "filter refers to an unknown attribute",
                });
            }
            Field::Attribute(name.to_owned())
        }
        _ => return Err(invalid("unknown filter field")),
    };
    if f != "attribute" && j.get("attribute").is_ok() {
        return Err(invalid("standard field cannot take an attribute selector"));
    }
    let op = Operator::parse(
        j.get("op")
            .and_then(Json::string)
            .map_err(|_| invalid("filter operator is required"))?,
    )
    .ok_or_else(|| invalid("unknown filter operator"))?;
    let def = match &field {
        Field::Attribute(name) => schema.get(name),
        _ => None,
    };
    let kind = match field {
        Field::Kind | Field::Name => AttrType::Text,
        Field::Class => AttrType::Integer,
        Field::Attribute(_) => def.map_or(AttrType::Text, |d| d.ty),
    };
    let value = scalar(
        j.get("value")
            .map_err(|_| invalid("filter literal is required"))?,
        kind,
        def,
    )?;
    Ok(Rule::Compare { field, op, value })
}

/// One admitted query over the head: the same rule, dataset and semantics as the engine API.
pub fn query(model: &Model, rule: &Rule) -> Result<Vec<ElementId>, Refusal> {
    Ok(model.dataset.query(rule)?)
}
