// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! I04 native user-property definitions and scalar values, preserving unknown properties.
use super::*;
use beamcraft_model::{
    attribute::{
        AttributeDefinition, AttributeType, AttributeValue, MAX_DEFINITIONS, MAX_VALUES,
        property_name,
    },
    id::ElementId,
};
use openusd::sdf;
/// Stored definition fields; the UUID/name are the standard element identity and label.
pub const FIELDS: &[(&str, &str)] = &[
    ("bim:attribute:type", "token"),
    ("bim:attribute:unit", "string"),
    ("bim:attribute:choices", "string[]"),
    ("bim:attribute:sets", "string[]"),
];
/// Parse a UUID-backed user property; unrecognized user names remain opaque.
pub fn property_id(name: &str) -> Option<ElementId> {
    let s = name.strip_prefix("user:a_")?;
    if s.len() != 32
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut full = String::new();
    for (i, c) in s.chars().enumerate() {
        if matches!(i, 8 | 12 | 16 | 20) {
            full.push('-');
        }
        full.push(c);
    }
    ElementId::parse(&full).ok()
}
fn get<'a>(p: &'a Prim, name: &str) -> Result<&'a Value, Diagnostic> {
    let expected = FIELDS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, ty)| *ty)
        .unwrap_or(match name {
            "bim:elementKind" | "bim:geometryMode" => "token",
            _ => "string",
        });
    p.properties
        .iter()
        .find(|v| v.name == name && v.type_name == expected)
        .and_then(|v| v.value.as_ref())
        .ok_or_else(invalid)
}
fn text(p: &Prim, n: &str) -> Result<String, Diagnostic> {
    if let Value::Text(s) = get(p, n)? {
        Ok(s.clone())
    } else {
        Err(invalid())
    }
}
fn texts(p: &Prim, n: &str) -> Result<Vec<String>, Diagnostic> {
    let Value::Array(v) = get(p, n)? else {
        return Err(invalid());
    };
    if v.len() > 128 {
        return Err(limit());
    }
    v.iter()
        .map(|v| {
            if let Value::Text(s) = v {
                Ok(s.clone())
            } else {
                Err(invalid())
            }
        })
        .collect()
}
/// Read all owned supported definitions; names and identities are unique.
pub fn definitions(prims: &[Prim]) -> Result<BTreeMap<ElementId, AttributeDefinition>, Diagnostic> {
    let mut out = BTreeMap::new();
    let mut names = std::collections::BTreeSet::new();
    for p in prims {
        if p.sources.is_empty()
            || p.sources.iter().any(|s| !s.owned)
            || text(p, "bim:elementKind").ok().as_deref() != Some("attribute_definition")
            || text(p, "bim:schemaVersion").ok().as_deref() != Some("0.1.0")
            || text(p, "bim:geometryMode").ok().as_deref() != Some("parametric")
        {
            continue;
        }
        if out.len() >= MAX_DEFINITIONS {
            return Err(limit());
        }
        let id = ElementId::parse(p.element_id.as_deref().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        let d = AttributeDefinition {
            id,
            name: text(p, "bim:name")?,
            kind: AttributeType::parse(&text(p, "bim:attribute:type")?).ok_or_else(invalid)?,
            unit: text(p, "bim:attribute:unit")?,
            choices: texts(p, "bim:attribute:choices")?,
            sets: texts(p, "bim:attribute:sets")?,
        };
        d.validate().map_err(|e| fail(e.code, e.message))?;
        if !names.insert(d.name.clone()) || out.insert(id, d).is_some() {
            return Err(invalid());
        }
    }
    Ok(out)
}
/// Scalar interpretation; unsupported user properties remain visible as opaque typed values.
pub fn scalar(property: &Property) -> Option<AttributeValue> {
    match (property.type_name.as_str(), property.value.as_ref()?) {
        ("string" | "token", Value::Text(s)) => Some(AttributeValue::Text(s.clone())),
        ("double" | "float", Value::Number(n)) => Some(AttributeValue::Number(*n)),
        ("int" | "int64", Value::Int(n)) => Some(AttributeValue::Integer(*n)),
        ("bool", Value::Bool(v)) => Some(AttributeValue::Boolean(*v)),
        _ => None,
    }
}
/// Admit known properties while preserving unknown keys, blocked values and future definitions.
pub(super) fn validate_snapshot(prims: &[Prim]) -> Result<(), Diagnostic> {
    let defs = definitions(prims)?;
    for p in prims {
        if p.sources.is_empty() || p.sources.iter().any(|s| !s.owned) || p.element_id.is_none() {
            continue;
        }
        let fields = p
            .properties
            .iter()
            .filter(|p| p.name.starts_with("user:"))
            .take(MAX_VALUES+1)
            .collect::<Vec<_>>();
        if fields.len() > MAX_VALUES {
            return Err(limit());
        }
        for field in fields {
            let Some(id) = property_id(&field.name) else {
                continue;
            };
            if let Some(def) = defs.get(&id) {
                let native_matches = match def.kind {
                    AttributeType::Text | AttributeType::Choice => {
                        matches!(field.type_name.as_str(), "string" | "token")
                    }
                    AttributeType::Number => matches!(field.type_name.as_str(), "double" | "float"),
                    AttributeType::Integer => matches!(field.type_name.as_str(), "int" | "int64"),
                    AttributeType::Boolean => field.type_name == "bool",
                };
                if !native_matches {
                    return Err(invalid());
                }
                if matches!(field.value, None | Some(Value::Block)) {
                    continue;
                }
                let v = scalar(field).ok_or_else(invalid)?;
                def.admits(&v).map_err(|e| fail(e.code, e.message))?;
            }
        }
    }
    Ok(())
}
pub(super) fn author_definition(
    data: &mut sdf::Data,
    p: &str,
    d: &AttributeDefinition,
) -> Result<(), Diagnostic> {
    d.validate().map_err(|e| fail(e.code, e.message))?;
    edit::attr(
        data,
        p,
        "bim:attribute:type",
        "token",
        sdf::Value::token(d.kind.token()),
    )?;
    edit::attr(
        data,
        p,
        "bim:attribute:unit",
        "string",
        sdf::Value::String(d.unit.clone()),
    )?;
    edit::attr(
        data,
        p,
        "bim:attribute:choices",
        "string[]",
        sdf::Value::StringVec(d.choices.clone()),
    )?;
    edit::attr(
        data,
        p,
        "bim:attribute:sets",
        "string[]",
        sdf::Value::StringVec(d.sets.clone()),
    )
}
/// Native type label for a supported scalar, including a scalar removed from an older layer.
pub(super) fn scalar_type(s: &str) -> Result<&'static str, Diagnostic> {
    match s {
        "string" => Ok("string"),
        "token" => Ok("token"),
        "double" => Ok("double"),
        "float" => Ok("float"),
        "int64" => Ok("int64"),
        "int" => Ok("int"),
        "bool" => Ok("bool"),
        _ => Err(unsupported()),
    }
}
pub(super) fn author_values(
    data: &mut sdf::Data,
    p: &str,
    values: &BTreeMap<ElementId, Option<AttributeValue>>,
) -> Result<(), Diagnostic> {
    for (id, value) in values {
        let name = property_name(*id);
        match value {
            Some(v) => {
                v.validate().map_err(|e| fail(e.code, e.message))?;
                let value = match v {
                    AttributeValue::Text(s) => sdf::Value::String(s.clone()),
                    AttributeValue::Number(n) => sdf::Value::Double(*n),
                    AttributeValue::Integer(n) => sdf::Value::Int64(*n),
                    AttributeValue::Boolean(v) => sdf::Value::Bool(*v),
                };
                edit::attr(data, p, &name, v.usd_type(), value)?;
            }
            None => {
                edit::attribute(data, p, &name, "string", Opinion::Remove, true)?;
            }
        }
    }
    Ok(())
}
/// Validate a complete value patch and its definition dependencies before any layer is changed.
pub(super) fn validate_patch(
    values: &BTreeMap<ElementId, Option<AttributeValue>>,
    prims: &[Prim],
) -> Result<Vec<(ElementId, bool)>, Diagnostic> {
    if values.is_empty() || values.len() > MAX_VALUES {
        return Err(limit());
    }
    let defs = definitions(prims)?;
    let mut dependencies = Vec::new();
    for (id, value) in values {
        if let Some(v) = value {
            defs.get(id)
                .ok_or_else(|| fail(ErrorCode::NotFound, "attribute definition is absent"))?
                .admits(v)
                .map_err(|e| fail(e.code, e.message))?;
        }
        dependencies.push((*id, value.is_some()));
    }
    Ok(dependencies)
}
