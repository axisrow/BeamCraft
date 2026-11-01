// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The steel plate readers (#388 slice): the loaded-content checks of a plate's contour,
//! holes, hole records, member-hole records and cut records. Extracted from `validate.rs`
//! unchanged; the discipline reversal moves this file to `crates/steel`.

use super::validate::{cap, points, required};
use super::*;
use beamcraft_model::id::OpId;
use steel::model::command::KeepSide;

pub(super) fn norm([x, y, z]: [f64; 3]) -> f64 {
    ((x * x + y * y) + z * z).sqrt()
}
pub(super) fn difference([x, y, z]: [f64; 3], [a, b, c]: [f64; 3]) -> [f64; 3] {
    [x - a, y - b, z - c]
}
/// The stored face contours of a lofted plate (A11): the shared coordinate cap and the
/// vertex-count bounds of the schema descriptor. Winding, planarity, simplicity and the
/// equal vertex count of the loft correspondence are the engine's evaluation decision.
pub(super) fn lofted_plate(start: &[[f64; 3]], end: &[[f64; 3]]) -> Result<(), Diagnostic> {
    cap(start.iter().flatten().copied(), 1e9)?;
    cap(end.iter().flatten().copied(), 1e9)?;
    let span = |points: &[[f64; 3]]| -> f64 {
        let Some(&first) = points.first() else {
            return 0.0;
        };
        let mut size = 0.0;
        for &p in points {
            let n = norm(difference(p, first));
            if n > size {
                size = n;
            }
        }
        size
    };
    if start.len() < 3 || start.len() != end.len() || span(start).max(span(end)) > 2e6 {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn plate(points: &[[f64; 3]], thickness: f64) -> Result<(), Diagnostic> {
    cap(points.iter().flatten().copied(), 1e9)?;
    cap([thickness], 1e6)?;
    let Some(&first) = points.first() else {
        return Err(invalid());
    };
    let mut size = 0.0;
    for &p in points {
        let n = norm(difference(p, first));
        if n > size {
            size = n;
        }
    }
    if size > 2e6 {
        return Err(limit());
    }
    let eps = 0.001f64.max(size / 4194304.0);
    let mut too_small = points.len() < 3 || thickness < eps;
    for (&p, &q) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        let length = norm(difference(q, p));
        if length > 1e6 {
            return Err(limit());
        }
        if length < eps {
            too_small = true;
        }
    }
    if too_small {
        return Err(invalid());
    }
    // Planarity, exact topology and rectangle support are deliberately the engine's F06 gate.
    Ok(())
}

/// The optional through-hole polygons of a loaded plate: either both attributes or neither, the
/// counts add up to the points, every hole has at least three points, and the totals are capped.
pub(super) fn plate_holes(prim: &Prim) -> Result<Vec<Vec<[f64; 3]>>, Diagnostic> {
    let present = |name: &str| prim.properties.iter().any(|p| p.name == name);
    let (has_points, has_counts) = (
        present("steel:plate:holePointsMm"),
        present("steel:plate:holeVertexCounts"),
    );
    if !has_points && !has_counts {
        return Ok(Vec::new());
    }
    if has_points != has_counts {
        return Err(invalid());
    }
    let points = points(prim, "steel:plate:holePointsMm")?;
    let Value::Array(counts) = required(prim, "steel:plate:holeVertexCounts", "int[]")? else {
        return Err(invalid());
    };
    if counts.len() > MAX_HOLES || points.len() > MAX_HOLES * MAX_HOLE_POINTS {
        return Err(limit());
    }
    cap(points.iter().flatten().copied(), 1e9)?;
    let mut holes = Vec::with_capacity(counts.len());
    let mut at = 0usize;
    for count in counts {
        let n = match count {
            Value::Int(n) => usize::try_from(*n).map_err(|_| invalid())?,
            _ => return Err(invalid()),
        };
        if !(3..=MAX_HOLE_POINTS).contains(&n) {
            return Err(invalid());
        }
        let end = at.checked_add(n).ok_or_else(invalid)?;
        holes.push(points.get(at..end).ok_or_else(invalid)?.to_vec());
        at = end;
    }
    if at != points.len() || holes.iter().flatten().any(|p| p[2] != 0.0) {
        return Err(invalid());
    }
    Ok(holes)
}

/// The optional through-hole operation records of a loaded plate (A04): all four attributes or
/// none, the counts add up to the points, every polygon has at least three in-cap points with
/// every `z` zero, there is one full UUIDv7 and one strictly increasing non-negative ordinal
/// per polygon, and the totals are capped. Winding, placement and clearance are the engine's
/// decision on replay, never re-derived here.
pub(super) fn plate_hole_records(prim: &Prim) -> Result<(), Diagnostic> {
    let present = |name: &str| prim.properties.iter().any(|p| p.name == name);
    let names = [
        "steel:hole:pointsMm",
        "steel:hole:vertexCounts",
        "steel:hole:operationIds",
        "steel:hole:ordinals",
        "steel:hole:kinds",
        "steel:hole:generatedBy",
    ];
    let declared: Vec<bool> = names.iter().map(|name| present(name)).collect();
    if declared.iter().all(|seen| !seen) {
        return Ok(());
    }
    if !declared.iter().all(|seen| *seen) {
        return Err(invalid());
    }
    let Value::Array(counts) = required(prim, "steel:hole:vertexCounts", "int[]")? else {
        return Err(invalid());
    };
    let Value::Array(ids) = required(prim, "steel:hole:operationIds", "string[]")? else {
        return Err(invalid());
    };
    let Value::Array(ordinals) = required(prim, "steel:hole:ordinals", "int[]")? else {
        return Err(invalid());
    };
    let Value::Array(kinds) = required(prim, "steel:hole:kinds", "token[]")? else {
        return Err(invalid());
    };
    let Value::Array(owners) = required(prim, "steel:hole:generatedBy", "string[]")? else {
        return Err(invalid());
    };
    if counts.len() > MAX_HOLES
        || counts.len() != ids.len()
        || counts.len() != ordinals.len()
        || counts.len() != kinds.len()
        || counts.len() != owners.len()
    {
        return Err(invalid());
    }
    // Every generated record names its full generating bolt group; a hand-placed hole stores
    // the empty string (A14.4).
    for owner in owners {
        let Value::Text(owner) = owner else {
            return Err(invalid());
        };
        if !owner.is_empty() && !uuid7(owner) {
            return Err(invalid());
        }
    }
    let points = points(prim, "steel:hole:pointsMm")?;
    if points.len() > MAX_HOLES * MAX_HOLE_POINTS {
        return Err(limit());
    }
    cap(points.iter().flatten().copied(), 1e9)?;
    let mut at = 0usize;
    let mut last: Option<i64> = None;
    for count in counts {
        let n = match count {
            Value::Int(n) => usize::try_from(*n).map_err(|_| invalid())?,
            _ => return Err(invalid()),
        };
        if !(3..=MAX_HOLE_POINTS).contains(&n) {
            return Err(invalid());
        }
        let end = at.checked_add(n).ok_or_else(invalid)?;
        let hole = points.get(at..end).ok_or_else(invalid)?;
        if hole.iter().any(|p| p[2] != 0.0) {
            return Err(invalid());
        }
        at = end;
    }
    if at != points.len() {
        return Err(invalid());
    }
    for (id, ordinal) in ids.iter().zip(ordinals.iter()) {
        let Value::Text(id) = id else {
            return Err(invalid());
        };
        if !uuid7(id) {
            return Err(invalid());
        }
        let Value::Int(n) = ordinal else {
            return Err(invalid());
        };
        if *n < 0 || last.is_some_and(|seen| *n <= seen) {
            return Err(invalid());
        }
        last = Some(*n);
    }
    for kind in kinds {
        let Value::Text(kind) = kind else {
            return Err(invalid());
        };
        if kind != "hole" && kind != "polygon_cut" {
            return Err(invalid());
        }
    }
    Ok(())
}
/// Most through holes of one plate and most points of one hole.
pub(super) const MAX_HOLES: usize = 64;
pub(super) const MAX_HOLE_POINTS: usize = 1024;

/// The optional member-hole operation records of a loaded beam or column (A04.2): all nine
/// attributes or none, the counts add up to the points, every polygon has at least three
/// in-cap points with every `z` zero, there is one full UUIDv7 and one strictly increasing
/// non-negative ordinal per polygon, each axis point is finite and in cap, each axis direction
/// is a signed unit token, and each extent is `through` (zero range) or a `from < to` range.
/// Winding, placement and fit are the engine's decision on replay, never re-derived here.
pub(super) fn member_hole_records(prim: &Prim) -> Result<(), Diagnostic> {
    let present = |name: &str| prim.properties.iter().any(|p| p.name == name);
    let names = [
        "steel:mhole:pointsMm",
        "steel:mhole:vertexCounts",
        "steel:mhole:operationIds",
        "steel:mhole:ordinals",
        "steel:mhole:axisPointsMm",
        "steel:mhole:axisDirections",
        "steel:mhole:extents",
        "steel:mhole:extentFromMm",
        "steel:mhole:extentToMm",
    ];
    let declared: Vec<bool> = names.iter().map(|name| present(name)).collect();
    if declared.iter().all(|seen| !seen) {
        return Ok(());
    }
    if !declared.iter().all(|seen| *seen) {
        return Err(invalid());
    }
    let Value::Array(counts) = required(prim, "steel:mhole:vertexCounts", "int[]")? else {
        return Err(invalid());
    };
    let Value::Array(ids) = required(prim, "steel:mhole:operationIds", "string[]")? else {
        return Err(invalid());
    };
    let Value::Array(ordinals) = required(prim, "steel:mhole:ordinals", "int[]")? else {
        return Err(invalid());
    };
    let Value::Array(extents) = required(prim, "steel:mhole:extents", "token[]")? else {
        return Err(invalid());
    };
    let Value::Array(from) = required(prim, "steel:mhole:extentFromMm", "double[]")? else {
        return Err(invalid());
    };
    let Value::Array(to) = required(prim, "steel:mhole:extentToMm", "double[]")? else {
        return Err(invalid());
    };
    if counts.len() > MAX_HOLES
        || counts.len() != ids.len()
        || counts.len() != ordinals.len()
        || counts.len() != extents.len()
        || counts.len() != from.len()
        || counts.len() != to.len()
    {
        return Err(invalid());
    }
    let polygons = points(prim, "steel:mhole:pointsMm")?;
    let axis = points(prim, "steel:mhole:axisPointsMm")?;
    let directions = points(prim, "steel:mhole:axisDirections")?;
    if axis.len() != counts.len() || directions.len() != counts.len() {
        return Err(invalid());
    }
    if polygons.len() > MAX_HOLES * MAX_HOLE_POINTS {
        return Err(limit());
    }
    cap(polygons.iter().flatten().copied(), 1e9)?;
    cap(axis.iter().flatten().copied(), 1e9)?;
    for direction in &directions {
        let unit = direction.iter().filter(|v| **v != 0.0).count() == 1
            && direction.iter().all(|v| matches!(v, 1.0 | -1.0 | 0.0));
        if !unit {
            return Err(invalid());
        }
    }
    let mut at = 0usize;
    for count in counts {
        let n = match count {
            Value::Int(n) => usize::try_from(*n).map_err(|_| invalid())?,
            _ => return Err(invalid()),
        };
        if !(3..=MAX_HOLE_POINTS).contains(&n) {
            return Err(invalid());
        }
        let end = at.checked_add(n).ok_or_else(invalid)?;
        let hole = polygons.get(at..end).ok_or_else(invalid)?;
        if hole.iter().any(|p| p[2] != 0.0) {
            return Err(invalid());
        }
        at = end;
    }
    if at != polygons.len() {
        return Err(invalid());
    }
    let double = |value: &&Value| match value {
        Value::Number(v) => Some(*v),
        _ => None,
    };
    let mut last: Option<i64> = None;
    for (((id, ordinal), extent), (start, end)) in ids
        .iter()
        .zip(ordinals.iter())
        .zip(extents.iter())
        .zip(from.iter().zip(to.iter()))
    {
        let Value::Text(id) = id else {
            return Err(invalid());
        };
        if !uuid7(id) {
            return Err(invalid());
        }
        let Value::Int(n) = ordinal else {
            return Err(invalid());
        };
        if *n < 0 || last.is_some_and(|seen| *n <= seen) {
            return Err(invalid());
        }
        last = Some(*n);
        let Value::Text(extent) = extent else {
            return Err(invalid());
        };
        let (Some(start), Some(end)) = (double(&start), double(&end)) else {
            return Err(invalid());
        };
        if extent == "through" {
            if start != 0.0 || end != 0.0 {
                return Err(invalid());
            }
        } else if extent == "range" {
            if start >= end {
                return Err(invalid());
            }
            cap([start, end], 1e9)?;
        } else {
            return Err(invalid());
        }
    }
    Ok(())
}

/// The owned plane-cut records of a loaded plate, beam or column (A03, amendment A-2): five
/// parallel attributes, all five or none, of one common length, in ascending ordinals at most
/// 999; each point is finite and within the coordinate cap and each normal is not the zero
/// vector. Whether a plane crosses its element is the engine's decision on replay, never
/// re-derived here. A damaged or half-authored list is invalid, never a default.
pub(super) fn element_cut(prim: &Prim) -> Result<Vec<CutRecord>, Diagnostic> {
    const FIELDS: [&str; 5] = [
        "steel:cut:operationId",
        "steel:cut:ordinal",
        "steel:cut:pointMm",
        "steel:cut:normal",
        "steel:cut:keep",
    ];
    let present = |name: &str| prim.properties.iter().any(|p| p.name == name);
    let count = FIELDS.iter().filter(|name| present(name)).count();
    if count == 0 {
        return Ok(Vec::new());
    }
    if count != FIELDS.len() {
        return Err(invalid());
    }
    let strings = |name: &str| -> Result<Vec<String>, Diagnostic> {
        let Value::Array(values) = required(prim, name, "string[]")? else {
            return Err(invalid());
        };
        values
            .iter()
            .map(|v| match v {
                Value::Text(s) => Ok(s.clone()),
                _ => Err(invalid()),
            })
            .collect()
    };
    let ids = strings(FIELDS[0])?;
    let Value::Array(ordinal_values) = required(prim, FIELDS[1], "int[]")? else {
        return Err(invalid());
    };
    let mut ordinals = Vec::with_capacity(ordinal_values.len());
    for value in ordinal_values {
        let Value::Int(n) = value else {
            return Err(invalid());
        };
        let ordinal = u16::try_from(*n).map_err(|_| invalid())?;
        if ordinal > 999 {
            return Err(invalid());
        }
        if ordinals.last().is_some_and(|last: &u16| *last >= ordinal) {
            return Err(invalid());
        }
        ordinals.push(ordinal);
    }
    let cut_points = points(prim, FIELDS[2])?;
    let normals = points(prim, FIELDS[3])?;
    let Value::Array(keeps) = required(prim, FIELDS[4], "token[]")? else {
        return Err(invalid());
    };
    let keep = |value: &Value| match value {
        Value::Text(s) => KeepSide::parse(s),
        _ => None,
    };
    let keeps = keeps
        .iter()
        .map(|value| keep(value).ok_or_else(invalid))
        .collect::<Result<Vec<_>, Diagnostic>>()?;
    if ids.len() != ordinals.len()
        || cut_points.len() != ordinals.len()
        || normals.len() != ordinals.len()
        || keeps.len() != ordinals.len()
    {
        return Err(invalid());
    }
    let records = ids
        .iter()
        .zip(
            ordinals
                .iter()
                .zip(cut_points.iter().zip(normals.iter().zip(keeps.iter()))),
        )
        .map(|(id, (ordinal, (point, (normal, keep))))| {
            let record = CutRecord {
                operation_id: OpId::parse(id).map_err(|_| invalid())?,
                ordinal: *ordinal,
                point_mm: *point,
                normal: *normal,
                keep: *keep,
            };
            record.validate().map_err(|_| invalid())?;
            Ok(record)
        })
        .collect::<Result<Vec<_>, Diagnostic>>()?;
    Ok(records)
}
