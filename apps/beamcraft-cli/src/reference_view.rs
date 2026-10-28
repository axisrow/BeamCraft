// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Read-only adapter from composed foreign USD prims to the engine's display geometry.

mod display;

use crate::plate::{PlateError, property, text};
use crate::reference::wrapper_of;
use beamcraft_engine::SceneElement;
use beamcraft_engine::limits::{MAX_BLOB_TRIANGLES, MAX_BLOB_VERTICES};
use beamcraft_engine::scene::{ReferenceGeometry, reference_element};
use beamcraft_layer_engine::authored::{ReferenceView, read_references};
use beamcraft_layer_engine::{Prim, Value};
use beamcraft_model::error::ErrorCode;
use display::display_meshes;
use std::collections::{BTreeMap, BTreeSet};

fn invalid() -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message: "invalid foreign display geometry",
    }
}

fn unsupported() -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::UnsupportedGeometry,
        message: "reference view supports translated Cube and triangle/convex-quad Mesh cages only",
    }
}

fn array<'a>(p: &'a Prim, name: &str, cap: usize) -> Result<&'a [Value], PlateError> {
    match property(p, name) {
        Some(Value::Array(values)) if values.len() <= cap => Ok(values),
        Some(Value::Array(_)) => Err(PlateError {
            rule_id: None,
            code: ErrorCode::LimitExceeded,
            message: "foreign display geometry exceeds the view budget",
        }),
        _ => Err(invalid()),
    }
}

fn point(value: &Value) -> Result<[f64; 3], PlateError> {
    let Value::Array(values) = value else {
        return Err(invalid());
    };
    let [Value::Number(x), Value::Number(y), Value::Number(z)] = values.as_slice() else {
        return Err(invalid());
    };
    Ok([*x, *y, *z])
}

fn index(value: &Value) -> Result<u32, PlateError> {
    match value {
        Value::Int(i) => u32::try_from(*i).map_err(|_| invalid()),
        Value::Uint(i) => u32::try_from(*i).map_err(|_| invalid()),
        _ => Err(invalid()),
    }
}

fn translation(p: &Prim) -> Result<[f64; 3], PlateError> {
    // USD operations not named in xformOpOrder are inactive, not an implicit transform.
    if property(p, "xformOpOrder").is_none() {
        return Ok([0.0; 3]);
    }
    let mut result = [0.0; 3];
    let mut seen = BTreeSet::new();
    for op in array(p, "xformOpOrder", 64)? {
        let name = text(op).ok_or_else(invalid)?;
        if !(name == "xformOp:translate" || name.starts_with("xformOp:translate:"))
            || !seen.insert(name)
        {
            return Err(unsupported());
        }
        let value = property(p, name).ok_or_else(invalid)?;
        for (total, v) in result.iter_mut().zip(point(value)?) {
            *total += v;
        }
    }
    Ok(result)
}

fn geometry(p: &Prim, translation: [f64; 3]) -> Result<Option<ReferenceGeometry>, PlateError> {
    let mut mesh = match p.type_name.as_deref() {
        Some("Xform" | "Scope") | None => return Ok(None),
        Some("Cube") => {
            let size = match property(p, "size") {
                None => 2.0, // USD Cube schema fallback.
                Some(Value::Number(n)) => *n,
                _ => return Err(invalid()),
            };
            ReferenceGeometry::cube(size, translation)?
        }
        Some("Mesh") => {
            // USD defaults subdivisionScheme to catmullClark, so an absent attribute is that
            // implicit default, not a bypass: only an explicit `none` draws the raw cage.
            let explicit_none =
                property(p, "subdivisionScheme").is_some_and(|v| text(v) == Some("none"));
            if !explicit_none {
                return Err(unsupported());
            }
            for name in ["holeIndices", "creaseIndices", "cornerIndices"] {
                if property(p, name).is_some()
                    && !array(p, name, MAX_BLOB_TRIANGLES as usize)?.is_empty()
                {
                    return Err(unsupported());
                }
            }
            ReferenceGeometry {
                points: array(p, "points", MAX_BLOB_VERTICES as usize)?
                    .iter()
                    .map(point)
                    .collect::<Result<_, _>>()?,
                face_vertex_counts: array(p, "faceVertexCounts", MAX_BLOB_TRIANGLES as usize)?
                    .iter()
                    .map(index)
                    .collect::<Result<_, _>>()?,
                face_vertex_indices: array(
                    p,
                    "faceVertexIndices",
                    MAX_BLOB_TRIANGLES as usize * 3,
                )?
                .iter()
                .map(index)
                .collect::<Result<_, _>>()?,
                translation,
                left_handed: false,
            }
        }
        _ => return Err(unsupported()),
    };
    mesh.left_handed = match property(p, "orientation") {
        None => false,
        Some(Value::Text(t)) if t == "rightHanded" => false,
        Some(Value::Text(t)) if t == "leftHanded" => true,
        _ => return Err(invalid()),
    };
    Ok(Some(mesh))
}

/// The composed surfaces of every wrapper, in wrapper id order, under the strict profile the R02
/// measurement certifies: translated Cube and Mesh cages only, anything else a refusal. The
/// display uses [`scene_elements`] and its own rules (`display_meshes`).
pub(crate) fn reference_surfaces(
    prims: &[Prim],
) -> Result<Vec<(ReferenceView, Vec<ReferenceGeometry>)>, PlateError> {
    surfaces(prims, false)
}

fn surfaces(
    prims: &[Prim],
    display: bool,
) -> Result<Vec<(ReferenceView, Vec<ReferenceGeometry>)>, PlateError> {
    let by_path: BTreeMap<_, _> = prims.iter().map(|p| (p.path.as_str(), p)).collect();
    read_references(prims)?
        .iter()
        .map(|view| {
            let meshes = if display {
                display_meshes(view, &by_path)
            } else {
                wrapper_meshes(view, &by_path)?
            };
            Ok((view.clone(), meshes))
        })
        .collect()
}

/// The foreign surfaces of one wrapper: a bounded walk of the composed prims below its arc
/// holder, accumulating translations, honouring activity and visibility, with the same
/// unsupported-representation refusals the view path has always made.
fn wrapper_meshes(
    view: &ReferenceView,
    by_path: &BTreeMap<&str, &Prim>,
) -> Result<Vec<ReferenceGeometry>, PlateError> {
    let prefix = format!("{}/", view.source_root);
    let mut ancestors = BTreeMap::new();
    let mut meshes = Vec::new();
    for (path, p) in by_path
        .range(view.source_root.as_str()..)
        .take_while(|(path, _)| **path == view.source_root || path.starts_with(&prefix))
    {
        let (parent, hidden) = if *path == view.source_root {
            ([0.0; 3], false)
        } else {
            let (parent, _) = path.rsplit_once('/').ok_or_else(invalid)?;
            *ancestors.get(parent).ok_or_else(invalid)?
        };
        let invisible = match property(p, "visibility") {
            None => false,
            Some(Value::Text(t)) if t == "inherited" => false,
            Some(Value::Text(t)) if t == "invisible" => true,
            _ => return Err(invalid()),
        };
        let hidden = hidden || !p.active || invisible;
        if hidden {
            ancestors.insert(*path, (parent, true));
            continue;
        }
        if property(p, "purpose").is_some_and(|v| !matches!(text(v), Some("default" | "render"))) {
            return Err(unsupported());
        }
        let [x, y, z] = translation(p)?;
        let [px, py, pz] = parent;
        let offset = [px + x, py + y, pz + z];
        if offset.iter().any(|v| !v.is_finite()) {
            return Err(PlateError {
                rule_id: None,
                code: ErrorCode::NonFiniteNumber,
                message: "foreign translation overflows",
            });
        }
        ancestors.insert(*path, (offset, false));
        if let Some(mesh) = geometry(p, offset)? {
            meshes.push(mesh);
        }
    }
    Ok(meshes)
}

/// Converts a complete, validated composed snapshot; it does not reopen or write a source file.
pub(crate) fn scene_elements(prims: &[Prim]) -> Result<Vec<SceneElement>, PlateError> {
    let mut elements = Vec::new();
    for (view, meshes) in surfaces(prims, true)? {
        let wrapper = wrapper_of(&view)?;
        if let Some(element) = reference_element(&wrapper.record, &meshes)? {
            elements.push(element);
        }
    }
    Ok(elements)
}
