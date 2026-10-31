// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D06 bounded, explicit USD annotation records. No schema plugin or hidden model.

use super::*;
use beamcraft_model::annotation::{
    Annotation, AnnotationPayload, FeatureRef, MAX_ANNOTATIONS, MAX_FEATURE_REF_BYTES,
};
use beamcraft_model::id::ElementId;
use core::str::FromStr;
use openusd::sdf;

/// The attributes authored by an annotation creation.
pub const FIELDS: &[(&str, &str)] = &[
    ("bim:discipline", "token"),
    ("bim:annotation:kind", "token"),
    ("bim:annotation:from", "string"),
    ("bim:annotation:to", "string"),
    ("bim:annotation:anchor", "string"),
    ("bim:annotation:attribute", "string"),
    ("bim:annotation:text", "string"),
    ("bim:annotation:endMm", "double3"),
    ("bim:annotation:pinDigest", "string[]"),
];

pub(super) fn validate_record(a: &Annotation) -> Result<(), Diagnostic> {
    a.validate().map_err(|e| fail(e.code, e.message))
}

/// Writes the anchor and its attribute name verbatim: the stored address is the placed address.
pub(super) fn author(data: &mut sdf::Data, path: &str, a: &Annotation) -> Result<(), Diagnostic> {
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
        "bim:annotation:kind",
        "token",
        sdf::Value::token(a.payload.as_str()),
    )?;
    edit::attr(
        data,
        path,
        "bim:annotation:pinDigest",
        "string[]",
        sdf::Value::StringVec(a.pins.iter().map(|p| p.to_string()).collect()),
    )?;
    let text = |data: &mut sdf::Data, name: &str, value: &str| {
        edit::attr(
            data,
            path,
            name,
            "string",
            sdf::Value::String(value.to_owned()),
        )
    };
    match &a.payload {
        AnnotationPayload::Dimension { from, to } => {
            text(data, "bim:annotation:from", from.as_str())?;
            text(data, "bim:annotation:to", to.as_str())?;
        }
        AnnotationPayload::Mark { anchor, attribute } => {
            text(data, "bim:annotation:anchor", anchor.as_str())?;
            text(data, "bim:annotation:attribute", attribute)?;
        }
        AnnotationPayload::Leader {
            anchor,
            text: leader,
            end_mm,
        } => {
            text(data, "bim:annotation:anchor", anchor.as_str())?;
            text(data, "bim:annotation:text", leader)?;
            let [x, y, z] = *end_mm;
            edit::attr(
                data,
                path,
                "bim:annotation:endMm",
                "double3",
                sdf::Value::vec3d(x, y, z),
            )?;
        }
    }
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

fn anchor(p: &Prim, name: &str) -> Result<FeatureRef, Diagnostic> {
    let text = text(p, name, "string")?;
    if text.len() > MAX_FEATURE_REF_BYTES || !text.is_ascii() {
        return Err(invalid());
    }
    FeatureRef::parse(text).map_err(|e| fail(e.code, e.message))
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

/// Decode and validate one owned annotation (D06).
pub fn read(p: &Prim) -> Result<Annotation, Diagnostic> {
    let id = ElementId::parse(text(p, "bim:id", "string")?)
        .map_err(|_| fail(ErrorCode::InvalidId, "invalid annotation identity"))?;
    if text(p, "bim:discipline", "token")? != "steel" {
        return Err(unsupported());
    }
    let name = text(p, "bim:name", "string")?;
    if name.len() > 256 {
        return Err(limit());
    }
    let pins = match p
        .properties
        .iter()
        .find(|v| v.name == "bim:annotation:pinDigest" && v.type_name == "string[]")
        .and_then(|v| v.value.as_ref())
    {
        Some(Value::Array(texts)) => {
            let mut pins = Vec::with_capacity(texts.len());
            for t in texts {
                match t {
                    Value::Text(text) => pins.push(
                        ContentDigest::from_str(text)
                            .map_err(|_| fail(ErrorCode::InvalidId, "invalid placement digest"))?,
                    ),
                    _ => return Err(invalid()),
                }
            }
            pins
        }
        _ => return Err(invalid()),
    };
    let payload = match text(p, "bim:annotation:kind", "token")? {
        "dimension" => AnnotationPayload::Dimension {
            from: anchor(p, "bim:annotation:from")?,
            to: anchor(p, "bim:annotation:to")?,
        },
        "mark" => AnnotationPayload::Mark {
            anchor: anchor(p, "bim:annotation:anchor")?,
            attribute: text(p, "bim:annotation:attribute", "string")?.to_owned(),
        },
        "leader" => AnnotationPayload::Leader {
            anchor: anchor(p, "bim:annotation:anchor")?,
            text: text(p, "bim:annotation:text", "string")?.to_owned(),
            end_mm: triple(p, "bim:annotation:endMm")?,
        },
        _ => return Err(unsupported()),
    };
    let annotation = Annotation {
        id,
        name: name.to_owned(),
        payload,
        pins,
    };
    validate_record(&annotation)?;
    Ok(annotation)
}

/// Read the owned annotations; validate the project-wide count.
pub fn read_annotations(prims: &[Prim]) -> Result<Vec<Annotation>, Diagnostic> {
    let mut out = Vec::new();
    for p in prims
        .iter()
        .filter(|p| !p.sources.is_empty() && p.sources.iter().all(|s| s.owned))
    {
        if text(p, "bim:elementKind", "token").ok() != Some("annotation") {
            continue;
        }
        if text(p, "bim:schemaVersion", "string")? != "0.1.0" {
            continue;
        }
        if out.len() >= MAX_ANNOTATIONS {
            return Err(limit());
        }
        out.push(read(p)?);
    }
    out.sort_by_key(|a| a.id);
    Ok(out)
}
