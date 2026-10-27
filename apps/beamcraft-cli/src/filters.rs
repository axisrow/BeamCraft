// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V14 strict rule JSON and personal saved filters, sharing one engine evaluator.
use crate::{
    attributes::{self, Attributes},
    plate::PlateError,
    saved_views::Views,
};
use beamcraft_engine::filter::{Dataset, Field, Operator, Record, Rule};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{
    attribute::{AttributeDefinition, AttributeType, AttributeValue},
    error::ErrorCode,
    id::ElementId,
};

fn invalid(message: &'static str) -> PlateError {
    PlateError::new(ErrorCode::InvalidArgument, message)
}
/// Build admitted object metadata from one pinned attribute read. Registry definitions are not
/// objects; inactive targets and blocked/unknown scalar values acquire no defaults.
pub fn dataset(attrs: &Attributes) -> Result<Dataset, PlateError> {
    let records = attrs
        .elements
        .iter()
        .filter(|(_, r)| !r.deactivated && r.kind != "attribute_definition")
        .map(|(id, r)| {
            let values = r
                .properties
                .iter()
                .filter_map(|p| {
                    let key = beamcraft_layer_engine::authored::attribute::property_id(&p.name)?;
                    attrs.definitions.get(&key)?;
                    Some((key, beamcraft_layer_engine::authored::attribute::scalar(p)?))
                })
                .collect();
            Record {
                id: *id,
                kind: r.kind.clone(),
                name: (!r.name.is_empty()).then(|| r.name.clone()),
                class: r.class,
                attributes: values,
            }
        })
        .collect();
    Ok(Dataset::new(attrs.definitions.clone(), records)?)
}
fn scalar(
    j: &Json,
    kind: AttributeType,
    def: Option<&AttributeDefinition>,
) -> Result<AttributeValue, PlateError> {
    let j = if let Json::Object(_) = j {
        j.closed(&["value", "unit"])
            .map_err(|_| invalid("filter unit literal must contain value and unit"))?;
        let expected = def
            .map(|d| d.unit.as_str())
            .ok_or_else(|| invalid("standard fields do not take unit literals"))?;
        if !matches!(kind, AttributeType::Number | AttributeType::Integer)
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
        AttributeType::Text | AttributeType::Choice => AttributeValue::Text(
            j.string()
                .map_err(|_| invalid("filter needs a text literal"))?
                .to_owned(),
        ),
        AttributeType::Number => AttributeValue::Number(
            j.float()
                .map_err(|_| invalid("filter needs a numeric literal"))?,
        ),
        AttributeType::Integer => {
            let Json::Number(s) = j else {
                return Err(invalid("filter needs an exact integer"));
            };
            AttributeValue::Integer(
                s.parse()
                    .map_err(|_| invalid("filter integer is outside int64"))?,
            )
        }
        AttributeType::Boolean => {
            AttributeValue::Boolean(j.boolean().map_err(|_| invalid("filter needs a boolean"))?)
        }
    })
}
/// Parse a bounded closed rule and resolve attribute names once to UUIDs.
pub fn decode_rule(j: &Json, attrs: &Attributes) -> Result<Rule, PlateError> {
    let mut nodes = 0;
    parse(j, attrs, 0, &mut nodes)
}
fn parse(
    j: &Json,
    attrs: &Attributes,
    depth: usize,
    nodes: &mut usize,
) -> Result<Rule, PlateError> {
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
            .map(|j| parse(j, attrs, depth + 1, nodes))
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
        "attribute" => Field::Attribute(
            attributes::definition(
                attrs,
                j.get("attribute")
                    .and_then(Json::string)
                    .map_err(|_| invalid("attribute selector is required"))?,
            )?
            .id,
        ),
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
    let def = if let Field::Attribute(id) = field {
        attrs.definitions.get(&id)
    } else {
        None
    };
    let kind = match field {
        Field::Kind | Field::Name => AttributeType::Text,
        Field::Class => AttributeType::Integer,
        Field::Attribute(_) => def.ok_or_else(|| invalid("unknown attribute"))?.kind,
    };
    let value = scalar(
        j.get("value")
            .map_err(|_| invalid("filter literal is required"))?,
        kind,
        def,
    )?;
    Ok(Rule::Compare { field, op, value })
}
fn scalar_json(v: &AttributeValue) -> Result<Json, PlateError> {
    Ok(match v {
        AttributeValue::Text(v) => Json::text(v),
        AttributeValue::Number(v) => {
            Json::number(*v).map_err(|_| invalid("nonfinite filter literal"))?
        }
        AttributeValue::Integer(v) => Json::Number(v.to_string()),
        AttributeValue::Boolean(v) => Json::Bool(*v),
    })
}
/// Normalize saved selectors to UUIDs and pin numeric unit labels in their literals.
pub fn encode_rule(rule: &Rule, attrs: &Attributes) -> Result<Json, PlateError> {
    match rule {
        Rule::All(rules) => Ok(Json::object([(
            "all",
            Json::Array(
                rules
                    .iter()
                    .map(|r| encode_rule(r, attrs))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        )])),
        Rule::Any(rules) => Ok(Json::object([(
            "any",
            Json::Array(
                rules
                    .iter()
                    .map(|r| encode_rule(r, attrs))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        )])),
        Rule::Compare { field, op, value } => {
            let field_name = match field {
                Field::Kind => "type",
                Field::Name => "name",
                Field::Class => "class",
                Field::Attribute(_) => "attribute",
            };
            let mut literal = scalar_json(value)?;
            if let Field::Attribute(id) = field {
                let d = attrs
                    .definitions
                    .get(id)
                    .ok_or_else(|| invalid("unknown filter attribute"))?;
                if matches!(d.kind, AttributeType::Number | AttributeType::Integer) {
                    literal = Json::object([("value", literal), ("unit", Json::text(&d.unit))]);
                }
            }
            let mut j = Json::object([
                ("field", Json::text(field_name)),
                ("op", Json::text(op.token())),
                ("value", literal),
            ]);
            if let (Field::Attribute(id), Json::Object(fields)) = (field, &mut j) {
                fields.insert("attribute".into(), Json::text(id.to_string()));
            }
            Ok(j)
        }
    }
}
/// Saved rule metadata. Typed compilation is deferred until use so missing attributes are errors,
/// while inspection/deletion can still expose a previously saved rule without dropping it.
#[derive(Debug, Clone)]
pub struct SavedFilter {
    /// Stable personal filter UUID.
    pub id: ElementId,
    /// Readable name.
    pub name: String,
    /// Canonical rule JSON, with UUID attribute selectors.
    pub rule: Json,
}
fn key(id: ElementId) -> String {
    format!("filter_set_{}", id.to_string().replace('-', ""))
}
/// Read one stored filter without silently accepting another format or identity.
pub fn saved(views: &Views, id: ElementId) -> Result<SavedFilter, PlateError> {
    let s = views
        .document
        .records
        .get(&key(id))
        .ok_or_else(|| PlateError::new(ErrorCode::NotFound, "saved filter not found"))?;
    let j = beamcraft_json::decode(s.as_bytes(), &Limits::DEFAULT)
        .map_err(|_| invalid("saved filter is malformed"))?;
    j.closed(&["version", "id", "name", "rule"])
        .map_err(|_| invalid("saved filter has unknown fields"))?;
    if j.get("version")
        .and_then(Json::unsigned)
        .map_err(|_| invalid("saved filter version is missing"))?
        != 1
    {
        return Err(PlateError::new(
            ErrorCode::UnsupportedVersion,
            "unsupported saved filter version",
        ));
    }
    if j.get("id")
        .and_then(Json::string)
        .map_err(|_| invalid("saved filter id is missing"))?
        != id.to_string()
    {
        return Err(invalid("saved filter identity mismatch"));
    }
    let name = j
        .get("name")
        .and_then(Json::string)
        .map_err(|_| invalid("saved filter name is missing"))?
        .to_owned();
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err(invalid("invalid saved filter name"));
    }
    Ok(SavedFilter {
        id,
        name,
        rule: j
            .get("rule")
            .map_err(|_| invalid("saved rule is missing"))?
            .clone(),
    })
}
/// Inspect personal filters in persistent UUID order.
pub fn list(views: &Views) -> Result<Vec<SavedFilter>, PlateError> {
    let mut out = Vec::new();
    for key in views
        .document
        .records
        .keys()
        .filter(|k| k.starts_with("filter_set_"))
    {
        let suffix = key
            .strip_prefix("filter_set_")
            .ok_or_else(|| invalid("invalid filter key"))?;
        let id =
            beamcraft_layer_engine::authored::attribute::property_id(&format!("user:a_{suffix}"))
                .ok_or_else(|| invalid("invalid saved filter UUID"))?;
        out.push(saved(views, id)?);
    }
    Ok(out)
}
/// Admit and replace one named personal rule in memory; caller publishes with file CAS.
pub fn save(views: &mut Views, filter: SavedFilter, attrs: &Attributes) -> Result<(), PlateError> {
    let rule = decode_rule(&filter.rule, attrs)?;
    dataset(attrs)?.predicate(&rule)?;
    if filter.name.trim().is_empty()
        || filter.name.len() > 256
        || filter.name.chars().any(char::is_control)
    {
        return Err(invalid("invalid filter name"));
    }
    let j = Json::object([
        ("version", Json::uint(1)),
        ("id", Json::text(filter.id.to_string())),
        ("name", Json::text(filter.name)),
        ("rule", encode_rule(&rule, attrs)?),
    ]);
    let bytes = j
        .encode(&Limits::DEFAULT)
        .map_err(|_| invalid("saved filter exceeds its JSON bound"))?;
    if bytes.len() > 65536 {
        return Err(invalid("saved filter exceeds its record bound"));
    }
    views.document.records.insert(
        key(filter.id),
        String::from_utf8(bytes).map_err(|_| invalid("filter is not UTF-8"))?,
    );
    Ok(())
}
fn binding_key(id: ElementId) -> String {
    format!("filter_binding_{}", id.to_string().replace('-', ""))
}
/// A view's optional object-filter binding; an unknown reference never means show-all.
pub fn binding(views: &Views, id: ElementId) -> Result<Option<ElementId>, PlateError> {
    views
        .document
        .records
        .get(&binding_key(id))
        .map(|s| ElementId::parse(s).map_err(|_| invalid("invalid view filter binding")))
        .transpose()
}
/// Set or clear a personal view binding; the saved filter must exist.
pub fn bind(
    views: &mut Views,
    view: ElementId,
    filter: Option<ElementId>,
) -> Result<(), PlateError> {
    if !views.document.views.iter().any(|v| v.id == view) {
        return Err(PlateError::new(ErrorCode::NotFound, "view not found"));
    }
    if let Some(id) = filter {
        saved(views, id)?;
        views
            .document
            .records
            .insert(binding_key(view), id.to_string());
    } else {
        views.document.records.remove(&binding_key(view));
    }
    Ok(())
}
/// Delete an unreferenced saved filter. Referenced local rules are never dropped silently.
pub fn delete(views: &mut Views, id: ElementId) -> Result<(), PlateError> {
    saved(views, id)?;
    for (value_key, value) in &views.document.records {
        if value_key.starts_with("filter_binding_") && value == &id.to_string() {
            return Err(PlateError::new(
                ErrorCode::DependencyConflict,
                "saved filter is used by a view",
            ));
        }
    }
    views.document.records.remove(&key(id));
    Ok(())
}
/// Resolve and evaluate the same admitted rule used by direct CLI queries.
pub fn matching(
    views: &Views,
    id: ElementId,
    attrs: &Attributes,
) -> Result<Vec<ElementId>, PlateError> {
    let filter = saved(views, id)?;
    let rule = decode_rule(&filter.rule, attrs)?;
    Ok(dataset(attrs)?.query(&rule)?)
}
