// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 bounded, explicit USD construction object records. No schema plugin or hidden model.

use super::*;
use beamcraft_model::construction::{
    ConstructionGeometry, ConstructionObject, MAX_CONSTRUCTION_OBJECTS,
};
use beamcraft_model::id::ElementId;
use openusd::sdf;

/// The attributes authored by a construction object creation.
pub const FIELDS: &[(&str, &str)] = &[
    ("bim:discipline", "token"),
    ("bim:construction:kind", "token"),
    ("bim:construction:aMm", "double3"),
    ("bim:construction:bMm", "double3"),
    ("bim:construction:radiusMm", "double"),
];

pub(super) fn validate_record(o: &ConstructionObject) -> Result<(), Diagnostic> {
    o.validate().map_err(|e| fail(e.code, e.message))
}

/// The geometry carried by one stored kind: `a`, optionally `b`, optionally a radius.
fn parts(g: &ConstructionGeometry) -> ([f64; 3], Option<[f64; 3]>, Option<f64>) {
    match *g {
        ConstructionGeometry::Point { point_mm } => (point_mm, None, None),
        ConstructionGeometry::Line { start_mm, end_mm } => (start_mm, Some(end_mm), None),
        ConstructionGeometry::Circle {
            centre_mm,
            axis,
            radius_mm,
        } => (centre_mm, Some(axis), Some(radius_mm)),
        ConstructionGeometry::Plane { point_mm, normal } => (point_mm, Some(normal), None),
    }
}

pub(super) fn author(
    data: &mut sdf::Data,
    path: &str,
    o: &ConstructionObject,
) -> Result<(), Diagnostic> {
    edit::attr(
        data,
        path,
        "bim:discipline",
        "token",
        sdf::Value::token("steel"),
    )?;
    let (a, b, radius) = parts(&o.geometry);
    edit::attr(
        data,
        path,
        "bim:construction:kind",
        "token",
        sdf::Value::token(o.geometry.as_str()),
    )?;
    let [ax, ay, az] = a;
    edit::attr(
        data,
        path,
        "bim:construction:aMm",
        "double3",
        sdf::Value::vec3d(ax, ay, az),
    )?;
    let [bx, by, bz] = b.unwrap_or([0.0; 3]);
    edit::attr(
        data,
        path,
        "bim:construction:bMm",
        "double3",
        sdf::Value::vec3d(bx, by, bz),
    )?;
    edit::attr(
        data,
        path,
        "bim:construction:radiusMm",
        "double",
        sdf::Value::Double(radius.unwrap_or(0.0)),
    )?;
    Ok(())
}

fn text<'a>(p: &'a Prim, name: &str, ty: &str) -> Result<&'a str, Diagnostic> {
    match p
        .properties
        .iter()
        .find(|v| v.name == name && v.type_name == ty)
        .and_then(|v| v.value.as_ref())
    {
        Some(Value::Text(s)) => Ok(s),
        _ => Err(invalid()),
    }
}

fn triple(p: &Prim, name: &str) -> Result<[f64; 3], Diagnostic> {
    let Value::Array(v) = p
        .properties
        .iter()
        .find(|v| v.name == name && v.type_name == "double3")
        .and_then(|v| v.value.as_ref())
        .ok_or_else(invalid)?
    else {
        return Err(invalid());
    };
    if v.len() != 3 {
        return Err(invalid());
    }
    let numbers = v
        .iter()
        .map(|v| match v {
            Value::Number(n) => Ok(*n),
            _ => Err(invalid()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let [x, y, z] = numbers.as_slice() else {
        return Err(invalid());
    };
    Ok([*x, *y, *z])
}

fn scalar(p: &Prim, name: &str) -> Result<f64, Diagnostic> {
    match p
        .properties
        .iter()
        .find(|v| v.name == name && v.type_name == "double")
        .and_then(|v| v.value.as_ref())
    {
        Some(Value::Number(n)) => Ok(*n),
        _ => Err(invalid()),
    }
}

fn token(p: &Prim, name: &str) -> Result<String, Diagnostic> {
    text(p, name, "token").map(str::to_owned)
}

/// Decode and validate one owned construction object (T02).
pub fn read(p: &Prim) -> Result<ConstructionObject, Diagnostic> {
    let id = ElementId::parse(text(p, "bim:id", "string")?)
        .map_err(|_| fail(ErrorCode::InvalidId, "invalid construction object identity"))?;
    if text(p, "bim:discipline", "token")? != "steel" {
        return Err(unsupported());
    }
    let name = text(p, "bim:name", "string")?;
    if name.len() > 256 {
        return Err(limit());
    }
    let a = triple(p, "bim:construction:aMm")?;
    let geometry = match token(p, "bim:construction:kind")?.as_str() {
        "point" => ConstructionGeometry::Point { point_mm: a },
        "line" => ConstructionGeometry::Line {
            start_mm: a,
            end_mm: triple(p, "bim:construction:bMm")?,
        },
        "circle" => ConstructionGeometry::Circle {
            centre_mm: a,
            axis: triple(p, "bim:construction:bMm")?,
            radius_mm: scalar(p, "bim:construction:radiusMm")?,
        },
        "plane" => ConstructionGeometry::Plane {
            point_mm: a,
            normal: triple(p, "bim:construction:bMm")?,
        },
        _ => return Err(unsupported()),
    };
    let object = ConstructionObject {
        id,
        name: name.to_owned(),
        geometry,
    };
    validate_record(&object)?;
    Ok(object)
}

/// Read the owned construction objects; validate the project-wide count.
pub fn read_constructions(prims: &[Prim]) -> Result<Vec<ConstructionObject>, Diagnostic> {
    let mut out = Vec::new();
    for p in prims
        .iter()
        .filter(|p| !p.sources.is_empty() && p.sources.iter().all(|s| s.owned))
    {
        if text(p, "bim:elementKind", "token").ok() != Some("construction") {
            continue;
        }
        if text(p, "bim:schemaVersion", "string")? != "0.1.0" {
            continue;
        }
        if out.len() >= MAX_CONSTRUCTION_OBJECTS {
            return Err(limit());
        }
        out.push(read(p)?);
    }
    out.sort_by_key(|o| o.id);
    Ok(out)
}
