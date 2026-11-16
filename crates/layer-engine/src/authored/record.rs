// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 bounded, explicit USD record inputs. No schema plugin or hidden model.

use super::*;
use beamcraft_model::id::ElementId;
use beamcraft_model::record::{
    ConnectionKind, CornerCutRequirement, CutPurpose, DeclaredConnection, GapRequirement,
    JointIntent, MAX_CONNECTIONS, MAX_SCOPE, Record, RecordPayload, UpperBound,
};
use openusd::sdf;

/// The attributes authored by a record creation.
pub const FIELDS: &[(&str, &str)] = &[
    ("bim:discipline", "token"),
    ("bim:record:type", "token"),
    ("bim:record:scope", "string[]"),
    ("bim:record:weldConnections", "string[]"),
    ("bim:record:contactConnections", "string[]"),
    ("bim:record:a", "string"),
    ("bim:record:b", "string"),
    ("bim:record:permittedMinMm", "double"),
    ("bim:record:permittedMaxMm", "double"),
    ("bim:record:toleranceMm", "double"),
    ("bim:record:target", "string"),
    ("bim:record:neighbours", "string[]"),
    ("bim:record:requiredMm", "double"),
    ("bim:record:upperBoundMm", "double"),
    ("bim:record:upperBoundToleranceMm", "double"),
    ("bim:record:purpose", "token"),
];

pub(super) fn validate_record(r: &Record) -> Result<(), Diagnostic> {
    r.validate().map_err(|e| fail(e.code, e.message))
}

/// Writes the declared scope and connection pairs verbatim: the stored ids are the
/// declared element ids.
pub(super) fn author(data: &mut sdf::Data, path: &str, r: &Record) -> Result<(), Diagnostic> {
    edit::attr(
        data,
        path,
        "bim:discipline",
        "token",
        sdf::Value::token("steel"),
    )?;
    edit::attr(
        data,
        path,
        "bim:record:type",
        "token",
        sdf::Value::token(r.payload_token()),
    )?;
    let pair = |c: &DeclaredConnection| format!("{}|{}|{}", c.a, c.b, c.kind.token());
    match &r.payload {
        RecordPayload::JointIntent(intent) => {
            strings(data, path, "bim:record:scope", &intent.scope)?;
            let welds = intent
                .connections
                .iter()
                .filter(|c| c.kind == ConnectionKind::Weld)
                .map(pair)
                .collect::<Vec<_>>();
            let contacts = intent
                .connections
                .iter()
                .filter(|c| c.kind == ConnectionKind::Contact)
                .map(pair)
                .collect::<Vec<_>>();
            strings(data, path, "bim:record:weldConnections", &welds)?;
            strings(data, path, "bim:record:contactConnections", &contacts)?;
        }
        RecordPayload::GapRequirement(gap) => {
            let text = |data: &mut sdf::Data, name: &str, value: &str| {
                edit::attr(
                    data,
                    path,
                    name,
                    "string",
                    sdf::Value::String(value.to_owned()),
                )
            };
            text(data, "bim:record:a", &gap.a.to_string())?;
            text(data, "bim:record:b", &gap.b.to_string())?;
            edit::attr(
                data,
                path,
                "bim:record:permittedMinMm",
                "double",
                sdf::Value::Double(gap.permitted_min_mm),
            )?;
            edit::attr(
                data,
                path,
                "bim:record:permittedMaxMm",
                "double",
                sdf::Value::Double(gap.permitted_max_mm),
            )?;
            edit::attr(
                data,
                path,
                "bim:record:toleranceMm",
                "double",
                sdf::Value::Double(gap.tolerance_mm),
            )?;
        }
        RecordPayload::CornerCut(corner) => {
            let text = |data: &mut sdf::Data, name: &str, value: &str| {
                edit::attr(
                    data,
                    path,
                    name,
                    "string",
                    sdf::Value::String(value.to_owned()),
                )
            };
            text(data, "bim:record:target", &corner.target.to_string())?;
            strings(data, path, "bim:record:neighbours", &corner.neighbours)?;
            edit::attr(
                data,
                path,
                "bim:record:requiredMm",
                "double",
                sdf::Value::Double(corner.required_mm),
            )?;
            edit::attr(
                data,
                path,
                "bim:record:toleranceMm",
                "double",
                sdf::Value::Double(corner.tolerance_mm),
            )?;
            // The standalone upper bound travels as its own attributes; their absence is
            // the recorded `none` (contract section 4.1).
            if let Some(bound) = &corner.upper_bound {
                edit::attr(
                    data,
                    path,
                    "bim:record:upperBoundMm",
                    "double",
                    sdf::Value::Double(bound.max_mm),
                )?;
                edit::attr(
                    data,
                    path,
                    "bim:record:upperBoundToleranceMm",
                    "double",
                    sdf::Value::Double(bound.tolerance_mm),
                )?;
            }
            edit::attr(
                data,
                path,
                "bim:record:purpose",
                "token",
                sdf::Value::token(corner.purpose.token()),
            )?;
        }
    }
    Ok(())
}

fn strings(
    data: &mut sdf::Data,
    path: &str,
    name: &str,
    values: &[impl std::fmt::Display],
) -> Result<(), Diagnostic> {
    edit::attr(
        data,
        path,
        name,
        "string[]",
        sdf::Value::StringVec(values.iter().map(|v| v.to_string()).collect()),
    )
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

fn strings_of(p: &Prim, name: &str) -> Result<Vec<String>, Diagnostic> {
    match p
        .properties
        .iter()
        .find(|v| v.name == name && v.type_name == "string[]")
        .and_then(|v| v.value.as_ref())
    {
        Some(Value::Array(v)) => {
            let mut out = Vec::with_capacity(v.len());
            for t in v {
                match t {
                    Value::Text(text) => out.push(text.to_owned()),
                    _ => return Err(invalid()),
                }
            }
            Ok(out)
        }
        _ => Err(invalid()),
    }
}

fn number(p: &Prim, name: &str) -> Result<f64, Diagnostic> {
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

fn element(text: &str) -> Result<ElementId, Diagnostic> {
    ElementId::parse(text).map_err(|_| fail(ErrorCode::InvalidId, "invalid record element id"))
}

fn pairs(texts: &[String]) -> Result<Vec<DeclaredConnection>, Diagnostic> {
    if texts.len() > MAX_CONNECTIONS {
        return Err(limit());
    }
    texts
        .iter()
        .map(|text| {
            let parts = text.split('|').collect::<Vec<_>>();
            let [a, b, kind] = parts.as_slice() else {
                return Err(invalid());
            };
            let kind = ConnectionKind::of_token(kind).ok_or_else(invalid)?;
            Ok(DeclaredConnection {
                a: element(a)?,
                b: element(b)?,
                kind,
            })
        })
        .collect()
}

/// Decode and validate one owned record (Q03).
pub fn read(p: &Prim) -> Result<Record, Diagnostic> {
    let id = element(text(p, "bim:id", "string")?)?;
    if text(p, "bim:discipline", "token")? != "steel" {
        return Err(unsupported());
    }
    let name = text(p, "bim:name", "string")?;
    if name.len() > 256 {
        return Err(limit());
    }
    let payload = match text(p, "bim:record:type", "token")? {
        "joint_intent" => {
            let scope = strings_of(p, "bim:record:scope")?;
            if scope.len() < 2 || scope.len() > MAX_SCOPE {
                return Err(limit());
            }
            let scope = scope
                .iter()
                .map(|t| element(t))
                .collect::<Result<Vec<_>, _>>()?;
            let mut connections = pairs(&strings_of(p, "bim:record:weldConnections")?)?;
            connections.extend(pairs(&strings_of(p, "bim:record:contactConnections")?)?);
            RecordPayload::JointIntent(JointIntent { scope, connections })
        }
        "gap_requirement" => RecordPayload::GapRequirement(GapRequirement {
            a: element(text(p, "bim:record:a", "string")?)?,
            b: element(text(p, "bim:record:b", "string")?)?,
            permitted_min_mm: number(p, "bim:record:permittedMinMm")?,
            permitted_max_mm: number(p, "bim:record:permittedMaxMm")?,
            tolerance_mm: number(p, "bim:record:toleranceMm")?,
        }),
        "corner_cut" => {
            let neighbours = strings_of(p, "bim:record:neighbours")?;
            let [a, b] = neighbours.as_slice() else {
                return Err(invalid());
            };
            let upper_bound = match p
                .properties
                .iter()
                .find(|v| v.name == "bim:record:upperBoundMm" && v.type_name == "double")
                .and_then(|v| v.value.as_ref())
            {
                Some(Value::Number(n)) => Some(UpperBound {
                    max_mm: *n,
                    tolerance_mm: number(p, "bim:record:upperBoundToleranceMm")?,
                }),
                None => None,
                _ => return Err(invalid()),
            };
            RecordPayload::CornerCut(CornerCutRequirement {
                target: element(text(p, "bim:record:target", "string")?)?,
                neighbours: [element(a)?, element(b)?],
                required_mm: number(p, "bim:record:requiredMm")?,
                tolerance_mm: number(p, "bim:record:toleranceMm")?,
                upper_bound,
                purpose: CutPurpose::of_token(text(p, "bim:record:purpose", "token")?)
                    .ok_or_else(invalid)?,
            })
        }
        _ => return Err(unsupported()),
    };
    let record = Record {
        id,
        name: name.to_owned(),
        payload,
    };
    validate_record(&record)?;
    Ok(record)
}

/// Read the owned records; validate the project-wide count.
pub fn read_records(prims: &[Prim]) -> Result<Vec<Record>, Diagnostic> {
    let mut out = Vec::new();
    for p in prims
        .iter()
        .filter(|p| !p.sources.is_empty() && p.sources.iter().all(|s| s.owned))
    {
        if text(p, "bim:elementKind", "token").ok() != Some("record") {
            continue;
        }
        if text(p, "bim:schemaVersion", "string")? != "0.1.0" {
            continue;
        }
        if out.len() >= beamcraft_model::record::MAX_RECORDS {
            return Err(limit());
        }
        out.push(read(p)?);
    }
    out.sort_by_key(|r| r.id);
    Ok(out)
}
