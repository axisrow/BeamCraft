// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Explicit, bounded T02 helper records in authored USD.
use super::*;
use beamcraft_model::helper::{Helper, HelperGeometry};
use beamcraft_model::id::ElementId;
use openusd::sdf;

/// Authored properties of helper geometry.
pub const FIELDS: &[(&str, &str)] = &[
    ("bim:helper:kind", "token"),
    ("bim:helper:parameters", "double[]"),
];

pub(super) fn author(
    data: &mut sdf::Data,
    path: &str,
    geometry: &HelperGeometry,
) -> Result<(), Diagnostic> {
    geometry.validate().map_err(|e| fail(e.code, e.message))?;
    edit::attr(
        data,
        path,
        "bim:helper:kind",
        "token",
        sdf::Value::token(geometry.kind()),
    )?;
    edit::attr(
        data,
        path,
        "bim:helper:parameters",
        "double[]",
        sdf::Value::DoubleVec(geometry.parameters()),
    )
}

/// Reads and admits one owned helper; foreign content is never interpreted by this API.
pub fn read(p: &Prim) -> Result<Helper, Diagnostic> {
    if p.sources.is_empty() || p.sources.iter().any(|s| !s.owned) {
        return Err(invalid());
    }
    let get = |name: &str, ty: &str| {
        p.properties
            .iter()
            .find(|v| v.name == name && v.type_name == ty)
            .and_then(|v| v.value.as_ref())
            .ok_or_else(invalid)
    };
    let text = |name, ty| match get(name, ty)? {
        Value::Text(s) => Ok(s.as_str()),
        _ => Err(invalid()),
    };
    if text("bim:elementKind", "token")? != "helper"
        || text("bim:schemaVersion", "string")? != "0.1.0"
        || text("bim:geometryMode", "token")? != "parametric"
    {
        return Err(invalid());
    }
    let Value::Array(values) = get("bim:helper:parameters", "double[]")? else {
        return Err(invalid());
    };
    if values.len() > 7 {
        return Err(invalid());
    }
    let values = values
        .iter()
        .map(|v| match v {
            Value::Number(n) => Ok(*n),
            _ => Err(invalid()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let geometry = HelperGeometry::from_parameters(text("bim:helper:kind", "token")?, &values)
        .map_err(|e| fail(e.code, e.message))?;
    let helper = Helper {
        id: ElementId::parse(p.element_id.as_deref().ok_or_else(invalid)?)
            .map_err(|_| invalid())?,
        name: text("bim:name", "string")?.to_owned(),
        geometry,
    };
    helper.validate().map_err(|e| fail(e.code, e.message))?;
    Ok(helper)
}
