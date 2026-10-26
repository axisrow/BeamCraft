// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01 bounded, explicit USD grid records. No schema plugin or hidden serialized model.

use super::*;
use beamcraft_model::grid::{
    DrawnGridLine, Grid, GridFamily, MAX_GRID_LINES_PER_FAMILY, MAX_GRIDS, RegularGridFamily,
};
use beamcraft_model::id::ElementId;
use openusd::sdf;

/// The attributes authored by a grid creation.
pub const FIELDS: &[(&str, &str)] = &[
    ("bim:discipline", "token"),
    ("bim:grid:u:mode", "token"),
    ("bim:grid:u:expression", "string"),
    ("bim:grid:u:spanMm", "double[]"),
    ("bim:grid:u:ids", "string[]"),
    ("bim:grid:u:labels", "string[]"),
    ("bim:grid:u:shapes", "token[]"),
    ("bim:grid:u:pointsMm", "double[]"),
    ("bim:grid:v:mode", "token"),
    ("bim:grid:v:expression", "string"),
    ("bim:grid:v:spanMm", "double[]"),
    ("bim:grid:v:ids", "string[]"),
    ("bim:grid:v:labels", "string[]"),
    ("bim:grid:v:shapes", "token[]"),
    ("bim:grid:v:pointsMm", "double[]"),
];

pub(super) fn validate_record(g: &Grid) -> Result<(), Diagnostic> {
    g.validate().map_err(|e| fail(e.code, e.message))
}

/// The rigid placement authored for the grid's plan frame.
pub fn placement(g: &Grid) -> Placement {
    let [x, y] = g.translation_mm;
    let [s, c] = g.orientation_zw;
    Placement {
        translation_mm: [x, y, 0.0],
        orientation_xyzw: [0.0, 0.0, s, c],
    }
}

fn arrays(f: &GridFamily, u: bool) -> (Vec<String>, Vec<String>, Vec<f64>) {
    match f {
        GridFamily::Regular(f) => {
            let [start, end] = f.span_mm;
            (
                f.axis_ids.iter().map(ToString::to_string).collect(),
                f.labels.clone(),
                f.positions_mm
                    .iter()
                    .flat_map(|&p| {
                        if u {
                            [p, start, p, end]
                        } else {
                            [start, p, end, p]
                        }
                    })
                    .collect(),
            )
        }
        GridFamily::Drawn(lines) => (
            lines.iter().map(|l| l.axis_id.to_string()).collect(),
            lines.iter().map(|l| l.label.clone()).collect(),
            lines
                .iter()
                .flat_map(|l| l.start_mm.into_iter().chain(l.end_mm))
                .collect(),
        ),
    }
}

pub(super) fn author(data: &mut sdf::Data, path: &str, g: &Grid) -> Result<(), Diagnostic> {
    edit::attr(
        data,
        path,
        "bim:discipline",
        "token",
        sdf::Value::token("steel"),
    )?;
    for (name, f, u) in [("u", &g.u, true), ("v", &g.v, false)] {
        let (ids, labels, points) = arrays(f, u);
        let shapes = vec!["line".to_owned(); ids.len()];
        let mode = match f {
            GridFamily::Regular(_) => "regular",
            GridFamily::Drawn(_) => "drawn",
        };
        let key = |suffix: &str| format!("bim:grid:{name}:{suffix}");
        edit::attr(data, path, &key("mode"), "token", sdf::Value::token(mode))?;
        if let GridFamily::Regular(f) = f {
            edit::attr(
                data,
                path,
                &key("expression"),
                "string",
                sdf::Value::String(f.expression.clone()),
            )?;
            edit::attr(
                data,
                path,
                &key("spanMm"),
                "double[]",
                sdf::Value::DoubleVec(f.span_mm.to_vec()),
            )?;
        }
        edit::attr(
            data,
            path,
            &key("ids"),
            "string[]",
            sdf::Value::StringVec(ids),
        )?;
        edit::attr(
            data,
            path,
            &key("labels"),
            "string[]",
            sdf::Value::StringVec(labels),
        )?;
        edit::attr(
            data,
            path,
            &key("shapes"),
            "token[]",
            sdf::Value::TokenVec(shapes.into_iter().map(Into::into).collect()),
        )?;
        edit::attr(
            data,
            path,
            &key("pointsMm"),
            "double[]",
            sdf::Value::DoubleVec(points),
        )?;
    }
    Ok(())
}

fn value<'a>(p: &'a Prim, name: &str, ty: &str) -> Result<&'a Value, Diagnostic> {
    p.properties
        .iter()
        .find(|v| v.name == name && v.type_name == ty)
        .and_then(|v| v.value.as_ref())
        .ok_or_else(invalid)
}
fn text<'a>(p: &'a Prim, name: &str, ty: &str) -> Result<&'a str, Diagnostic> {
    match value(p, name, ty)? {
        Value::Text(s) => Ok(s),
        _ => Err(invalid()),
    }
}
fn numbers(p: &Prim, name: &str, ty: &str, cap: usize) -> Result<Vec<f64>, Diagnostic> {
    let Value::Array(v) = value(p, name, ty)? else {
        return Err(invalid());
    };
    if v.len() > cap {
        return Err(limit());
    }
    v.iter()
        .map(|v| match v {
            Value::Number(n) => Ok(*n),
            _ => Err(invalid()),
        })
        .collect()
}
fn strings(p: &Prim, name: &str, ty: &str) -> Result<Vec<String>, Diagnostic> {
    let Value::Array(v) = value(p, name, ty)? else {
        return Err(invalid());
    };
    if v.len() > MAX_GRID_LINES_PER_FAMILY {
        return Err(limit());
    }
    v.iter()
        .map(|v| match v {
            Value::Text(s) if s.len() <= 64 => Ok(s.clone()),
            _ => Err(invalid()),
        })
        .collect()
}
fn family(p: &Prim, name: &str, u: bool) -> Result<GridFamily, Diagnostic> {
    let key = |suffix: &str| format!("bim:grid:{name}:{suffix}");
    let ids = strings(p, &key("ids"), "string[]")?
        .iter()
        .map(|s| {
            ElementId::parse(s)
                .map_err(|_| fail(ErrorCode::InvalidId, "invalid grid axis identity"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let labels = strings(p, &key("labels"), "string[]")?;
    let shapes = strings(p, &key("shapes"), "token[]")?;
    let points = numbers(
        p,
        &key("pointsMm"),
        "double[]",
        MAX_GRID_LINES_PER_FAMILY * 4,
    )?;
    if labels.len() != ids.len() || shapes.len() != ids.len() || points.len() != ids.len() * 4 {
        return Err(invalid());
    }
    if shapes.iter().any(|s| s != "line") {
        return Err(unsupported());
    }
    match text(p, &key("mode"), "token")? {
        "regular" => {
            let span = numbers(p, &key("spanMm"), "double[]", 2)?;
            let [start, end] = span.as_slice() else {
                return Err(invalid());
            };
            let positions = points
                .as_chunks::<4>()
                .0
                .iter()
                .map(|v| match v {
                    [x1, y1, x2, y2] if u && x1 == x2 && y1 == start && y2 == end => Ok(*x1),
                    [x1, y1, x2, y2] if !u && y1 == y2 && x1 == start && x2 == end => Ok(*y1),
                    _ => Err(invalid()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let expression = text(p, &key("expression"), "string")?;
            if expression.len() > 4096 {
                return Err(limit());
            }
            Ok(GridFamily::Regular(RegularGridFamily {
                expression: expression.to_owned(),
                axis_ids: ids,
                labels,
                span_mm: [*start, *end],
                positions_mm: positions,
            }))
        }
        "drawn" => {
            if p.properties
                .iter()
                .any(|v| v.name == key("expression") || v.name == key("spanMm"))
            {
                return Err(unsupported());
            }
            let lines = ids
                .into_iter()
                .zip(labels)
                .zip(points.as_chunks::<4>().0.iter())
                .map(|((axis_id, label), [x1, y1, x2, y2])| DrawnGridLine {
                    axis_id,
                    label,
                    start_mm: [*x1, *y1],
                    end_mm: [*x2, *y2],
                })
                .collect();
            Ok(GridFamily::Drawn(lines))
        }
        _ => Err(unsupported()),
    }
}

/// Decode and validate one owned grid, including every stored parallel array (T01-V01).
pub fn read(p: &Prim) -> Result<Grid, Diagnostic> {
    let id = ElementId::parse(text(p, "bim:id", "string")?)
        .map_err(|_| fail(ErrorCode::InvalidId, "invalid grid identity"))?;
    if text(p, "bim:discipline", "token")? != "steel" {
        return Err(unsupported());
    }
    let translation = numbers(p, "xformOp:translate", "double3", 3)?;
    let orientation = numbers(p, "xformOp:orient", "quatd", 4)?;
    let ([x, y, z], [w, qx, qy, qz]) = (translation.as_slice(), orientation.as_slice()) else {
        return Err(invalid());
    };
    if *z != 0.0 || *qx != 0.0 || *qy != 0.0 {
        return Err(unsupported());
    }
    let name = text(p, "bim:name", "string")?;
    if name.len() > 256 {
        return Err(limit());
    }
    let g = Grid {
        id,
        name: name.to_owned(),
        translation_mm: [*x, *y],
        orientation_zw: [*qz, *w],
        u: family(p, "u", true)?,
        v: family(p, "v", false)?,
    };
    validate_record(&g)?;
    Ok(g)
}

/// Read the owned project grids; validate project-wide AxisId uniqueness and the grid count.
pub fn read_grids(prims: &[Prim]) -> Result<Vec<Grid>, Diagnostic> {
    let mut out = Vec::new();
    let mut ids = prims
        .iter()
        .filter_map(|p| p.element_id.as_deref())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    for p in prims
        .iter()
        .filter(|p| !p.sources.is_empty() && p.sources.iter().all(|s| s.owned))
    {
        if text(p, "bim:elementKind", "token").ok() != Some("grid") {
            continue;
        }
        if text(p, "bim:schemaVersion", "string")? != "0.1.0" {
            continue;
        }
        if out.len() >= MAX_GRIDS {
            return Err(limit());
        }
        if prims.iter().any(|child| {
            child
                .path
                .strip_prefix(&p.path)
                .is_some_and(|suffix| suffix == "/Geometry" || suffix.starts_with("/Geometry/"))
        }) {
            return Err(unsupported());
        }
        let g = read(p)?;
        for id in g.axis_ids() {
            if !ids.insert(id.to_string()) {
                return Err(fail(
                    ErrorCode::DuplicateId,
                    "grid axis identity is already in use",
                ));
            }
        }
        out.push(g);
    }
    out.sort_by_key(|g| g.id);
    Ok(out)
}
