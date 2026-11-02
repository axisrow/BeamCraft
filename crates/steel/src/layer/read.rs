// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The steel readers of a stored layer (#388 slice): the loaded-content checks of a plate's
//! contour, holes, hole records, member-hole records and cut records, the connection checks,
//! the cross-element `bim:connects` rules and the cut-record admission. Extracted from the
//! platform's `validate.rs` unchanged; the discipline reversal moved them to `crates/steel`,
//! where the discipline's hooks answer the platform's questions with them.

use super::BoltGroup;
use super::definition::{
    FastenerDef, PROFILE_KINDS, RectangleProfileDef, SectionProfileDef, read_definition,
};
use crate::model::command::CutRecord;
use crate::model::{command::KeepSide, fastener::FastenerKind, fastener::FastenerSpec};
use beamcraft_layer_engine::authored::seam::read::{
    cap, definition, finite, link, number, optional, optional_number, points, relationship,
    required, resource, supported_kind, text,
};
use beamcraft_layer_engine::authored::{Definition, cut_history, invalid, is_part_kind};
use beamcraft_layer_engine::{Diagnostic, Prim, Value, corrupt, fail, limit, unsupported, uuid7};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::OpId;
use beamcraft_model::limits::{
    MAX_MEMBER_PATH_POINTS, MAX_RELATIONSHIP_TARGETS, MIN_WELD_SEGMENT_MM,
};
use beamcraft_model::prim::DefinitionCategory;
use std::collections::BTreeMap;

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

/// The A15 weld path rules shared by the loaded-content and candidate checks: 2 to 1024 points,
/// finite and within the coordinate cap, consecutive points at least the minimum apart.
pub(super) fn weld_path_points(points: &[[f64; 3]]) -> Result<(), Diagnostic> {
    if !(2..=1024).contains(&points.len()) {
        return Err(invalid());
    }
    for point in points {
        finite(*point)?;
        cap(*point, 1e9)?;
    }
    for pair in points.windows(2) {
        let [from, to] = pair else {
            continue;
        };
        let [fx, fy, fz] = from;
        let [tx, ty, tz] = to;
        let (dx, dy, dz) = (tx - fx, ty - fy, tz - fz);
        if (dx * dx + dy * dy + dz * dz) < MIN_WELD_SEGMENT_MM * MIN_WELD_SEGMENT_MM {
            return Err(invalid());
        }
    }
    Ok(())
}

/// The weld properties of a loaded `connection` (A06): every one is authored, with a supported
/// token, and the size is finite in `(0, 1000]`. A connection without them is not a weld this
/// slice can read, so it is refused rather than guessed. The optional geometry attributes (A15)
/// are validated as a whole when the path is present and refused in part.
pub(super) fn weld_properties(p: &Prim) -> Result<(), Diagnostic> {
    weld_tokens(
        text(p, "steel:weld:type", "token")?,
        text(p, "steel:weld:location", "token")?,
        text(p, "steel:weld:side", "token")?,
    )?;
    weld_size(number(p, "steel:weld:sizeMm")?)?;
    let path = p.properties.iter().any(|v| v.name == "steel:weld:pathMm");
    let all_around = p
        .properties
        .iter()
        .any(|v| v.name == "steel:weld:allAround");
    let contour = p.properties.iter().any(|v| v.name == "steel:weld:contour");
    if path {
        weld_path_points(&points(p, "steel:weld:pathMm")?)?;
        if all_around && !matches!(text(p, "steel:weld:allAround", "token")?, "true" | "false") {
            return Err(invalid());
        }
        if contour
            && !matches!(
                text(p, "steel:weld:contour", "token")?,
                "none" | "flush" | "concave" | "convex"
            )
        {
            return Err(invalid());
        }
    } else if all_around || contour {
        // Symbol data without the path it belongs to has no supported meaning.
        return Err(invalid());
    }
    Ok(())
}
pub(super) fn weld_tokens(weld_type: &str, location: &str, side: &str) -> Result<(), Diagnostic> {
    if weld_type != "fillet" {
        return Err(unsupported());
    }
    if !matches!(location, "workshop" | "site") || !matches!(side, "arrow" | "other" | "both") {
        return Err(invalid());
    }
    Ok(())
}
pub(super) fn weld_size(mm: f64) -> Result<(), Diagnostic> {
    finite([mm])?;
    if mm <= 0.0 {
        return Err(invalid());
    }
    cap([mm], 1000.0)
}

/// A05 bolt properties of a loaded `connection`: every one is authored, the diameter is finite in
/// `(0, 1000]`, the count is 1 to 512, the pitch is finite in `(0, 1e6]`, the origin is finite
/// within the coordinate cap and the direction is a finite unit vector. A connection without them
/// is not a bolt group this slice can read, so it is refused rather than guessed.
pub(super) fn bolt_properties(p: &Prim) -> Result<(), Diagnostic> {
    // A stray A14.3 attribute on an A05 group is mixed content this version cannot read.
    if p.properties.iter().any(|v| {
        matches!(
            v.name.as_str(),
            "steel:bolt:fastener"
                | "steel:bolt:point1Mm"
                | "steel:bolt:point2Mm"
                | "steel:bolt:planeNormalMm"
                | "steel:bolt:rotationRad"
                | "steel:bolt:onPlaneMm"
                | "steel:bolt:depthMm"
                | "steel:bolt:offsetStartMm"
                | "steel:bolt:spacingMode"
                | "steel:bolt:spacingValueMm"
                | "steel:bolt:spacingsMm"
                | "steel:bolt:lengthMode"
                | "steel:bolt:lengthMm"
                | "steel:bolt:washerUnderHead"
                | "steel:bolt:washerUnderNut"
        )
    }) {
        return Err(invalid());
    }
    let diameter = number(p, "steel:bolt:diameterMm")?;
    finite([diameter])?;
    if diameter <= 0.0 {
        return Err(invalid());
    }
    cap([diameter], 1000.0)?;
    let count = match required(p, "steel:bolt:count", "int")? {
        Value::Int(n) => *n,
        _ => return Err(invalid()),
    };
    if !(1..=512).contains(&count) {
        return Err(invalid());
    }
    let pitch = number(p, "steel:bolt:pitchMm")?;
    finite([pitch])?;
    if pitch <= 0.0 {
        return Err(invalid());
    }
    cap([pitch], 1e6)?;
    let origin = bolt_vec3(p, "steel:bolt:originMm")?;
    finite(origin)?;
    cap(origin, 1e9)?;
    let direction = bolt_vec3(p, "steel:bolt:directionMm")?;
    finite(direction)?;
    let squared = direction.iter().map(|v| v * v).sum::<f64>();
    if (squared - 1.0).abs() > 1e-6 {
        return Err(invalid());
    }
    Ok(())
}

/// One scalar `double3` property of a prim.
fn bolt_vec3(p: &Prim, name: &str) -> Result<[f64; 3], Diagnostic> {
    let Value::Array(values) = required(p, name, "double3")? else {
        return Err(invalid());
    };
    match values.as_slice() {
        [Value::Number(x), Value::Number(y), Value::Number(z)] => Ok([*x, *y, *z]),
        _ => Err(invalid()),
    }
}

/// One `double[]` property of a prim.
fn bolt_f64s(p: &Prim, name: &str) -> Result<Vec<f64>, Diagnostic> {
    let Value::Array(values) = required(p, name, "double[]")? else {
        return Err(invalid());
    };
    values
        .iter()
        .map(|v| match v {
            Value::Number(n) => Ok(*n),
            _ => Err(invalid()),
        })
        .collect()
}

/// The `connects` identities of an A14.3 group: 2 to 1024 full UUIDv7s, distinct, never the
/// group itself (`DV-REL-004`, `DV-REL-006`, `DV-REL-007`).
pub(super) fn bolt_group_connects(connects: &[String], self_id: &str) -> Result<(), Diagnostic> {
    if connects.len() < 2 || connects.len() > MAX_RELATIONSHIP_TARGETS {
        return Err(fail(
            ErrorCode::DependencyConflict,
            "a bolt group connects 2 to 1024 parts",
        )
        .with_rule("DV-REL-004"));
    }
    for part in connects {
        if !uuid7(part) {
            return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                .with_rule("DV-BASE-001"));
        }
        if part == self_id {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "a bolt group cannot connect itself",
            )
            .with_rule("DV-REL-006"));
        }
    }
    for (index, part) in connects.iter().enumerate() {
        if connects.iter().skip(index + 1).any(|other| other == part) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "a bolt group connects distinct parts",
            )
            .with_rule("DV-REL-007"));
        }
    }
    Ok(())
}

/// The stored values of an A14.3 group (bolt amendment section 4): two distinct points within
/// the coordinate cap, a unit plane normal, finite position values in range, and a spacing and
/// length record consistent with its mode. The engine's crossing, grip and length decisions
/// have already run; this re-checks the stored form so a damaged or forged record is refused,
/// never guessed.
pub(super) fn bolt_group_values(b: &BoltGroup) -> Result<(), Diagnostic> {
    finite(b.point1_mm)?;
    finite(b.point2_mm)?;
    cap(b.point1_mm, 1e9)?;
    cap(b.point2_mm, 1e9)?;
    let squared = b
        .point2_mm
        .iter()
        .zip(b.point1_mm)
        .map(|(q, p)| {
            let d = q - p;
            d * d
        })
        .sum::<f64>();
    if squared < 1e-6 {
        return Err(invalid());
    }
    finite(b.plane_normal_mm)?;
    let unit = b.plane_normal_mm.iter().map(|v| v * v).sum::<f64>();
    if (unit - 1.0).abs() > 1e-6 {
        return Err(invalid());
    }
    finite([b.rotation_rad, b.on_plane_mm, b.depth_mm, b.offset_start_mm])?;
    cap([b.on_plane_mm, b.depth_mm], 1e6)?;
    if !(0.0..=1e6).contains(&b.offset_start_mm) || !(0.0..=1e6).contains(&b.length_mm) {
        return Err(invalid());
    }
    // The authoritative spacings give 2 to 512 fasteners, positive and capped.
    if b.spacings_mm.is_empty() || b.spacings_mm.len() >= 512 {
        return Err(invalid());
    }
    finite(b.spacings_mm.iter().copied())?;
    cap(b.spacings_mm.iter().copied(), 1e6)?;
    if b.spacings_mm.iter().any(|s| *s <= 0.0) {
        return Err(invalid());
    }
    // The stored pattern agrees with the authoritative list it expanded to: an explicit list
    // carries no value, an equal pitch repeats its value, and an offset-end pattern re-expands
    // from the stored points within a micron.
    match b.spacing_mode.as_str() {
        "spacings" if b.spacing_value_mm == 0.0 => {}
        "pitch" => {
            if b.spacing_value_mm <= 0.0 || b.spacing_value_mm > 1e6 {
                return Err(invalid());
            }
            if b.spacings_mm.iter().any(|s| *s != b.spacing_value_mm) {
                return Err(invalid());
            }
        }
        "offset_end" => {
            if !(0.0..=1e6).contains(&b.spacing_value_mm) {
                return Err(invalid());
            }
            let pitch = (squared.sqrt() - b.offset_start_mm - b.spacing_value_mm)
                / b.spacings_mm.len() as f64;
            let agrees = |stored: f64| (stored - pitch).abs() <= stored.abs() * 1e-9 + 1e-9;
            if pitch <= 0.0 || !b.spacings_mm.iter().all(|s| agrees(*s)) {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    if !matches!(b.length_mode.as_str(), "auto" | "fixed") {
        return Err(invalid());
    }
    if !matches!(b.hole_mode.as_str(), "generate" | "none") {
        return Err(invalid());
    }
    // A generating group may carry its own clearance; a quiet one carries none.
    if matches!(b.hole_mode.as_str(), "none") && b.hole_clearance_mm.is_some() {
        return Err(invalid());
    }
    if let Some(clearance) = b.hole_clearance_mm {
        finite([clearance])?;
        cap([clearance], 1e6)?;
    }
    Ok(())
}

/// Consistency of a stored A14.3 group with its pinned definition (bolt amendment sections 2
/// and 4): the resolved length is one of the definition's standard lengths, a washer switch is
/// true only for a washer the definition carries, and under a nut only for a bolt.
pub(super) fn bolt_group_against(spec: &FastenerSpec, b: &BoltGroup) -> Result<(), Diagnostic> {
    if !spec.lengths_mm.contains(&b.length_mm) {
        return Err(invalid());
    }
    if b.washer_under_head && spec.washer.is_none() {
        return Err(invalid());
    }
    if b.washer_under_nut && (spec.washer.is_none() || spec.kind != FastenerKind::Bolt) {
        return Err(invalid());
    }
    Ok(())
}

/// The fastener definition a group places: one this batch pins as a resource, or the pinned
/// definition the model already holds. Anything else is a dangling reference.
pub(super) fn fastener_definition(
    input: &beamcraft_layer_engine::authored::Prepare,
    prims: &[Prim],
    digest: ContentDigest,
) -> Result<FastenerSpec, Diagnostic> {
    if let Some(found) = input.resources.iter().find(|r| r.digest == digest) {
        return match &found.definition {
            Definition::Discipline(hook) => match hook.0.as_any().downcast_ref::<FastenerDef>() {
                Some(def) => Ok(def.spec.clone()),
                _ => Err(invalid()),
            },
            _ => Err(invalid()),
        };
    }
    pinned_fastener(prims, digest)
}

/// The pinned fastener definition of a stored digest, re-admitted from its canonical bytes.
fn pinned_fastener(prims: &[Prim], digest: ContentDigest) -> Result<FastenerSpec, Diagnostic> {
    let p = resource(prims, digest, &["fastener"], DefinitionCategory::Fastener)?;
    let definition = read_definition(p, "fastener")?.ok_or_else(corrupt)?;
    match definition {
        Definition::Discipline(hook) => match hook.0.as_any().downcast_ref::<FastenerDef>() {
            Some(def) => Ok(def.spec.clone()),
            _ => Err(corrupt()),
        },
        _ => Err(corrupt()),
    }
}

/// The fifteen A14.3 bolt-group attributes of a loaded `connection` (bolt amendment section 4):
/// every one is authored and never mixed with the five A05 attributes; the points, the normal
/// and the spacing and length records follow their stored rules; and the digest resolves to a
/// pinned fastener definition the stored record agrees with. A connection without them is not a
/// group this slice can read, so it is refused rather than guessed.
pub(super) fn bolt_group_properties(p: &Prim, prims: &[Prim]) -> Result<(), Diagnostic> {
    // A stray A05 attribute on an A14.3 group is mixed content this version cannot read.
    if p.properties.iter().any(|v| {
        matches!(
            v.name.as_str(),
            "steel:bolt:diameterMm"
                | "steel:bolt:count"
                | "steel:bolt:pitchMm"
                | "steel:bolt:originMm"
                | "steel:bolt:directionMm"
        )
    }) {
        return Err(invalid());
    }
    let digest = text(p, "steel:bolt:fastener", "string")?
        .parse::<ContentDigest>()
        .map_err(|_| invalid())?;
    let point1 = bolt_vec3(p, "steel:bolt:point1Mm")?;
    let point2 = bolt_vec3(p, "steel:bolt:point2Mm")?;
    let normal = bolt_vec3(p, "steel:bolt:planeNormalMm")?;
    let rotation = number(p, "steel:bolt:rotationRad")?;
    let on_plane = number(p, "steel:bolt:onPlaneMm")?;
    let depth = number(p, "steel:bolt:depthMm")?;
    let offset_start = number(p, "steel:bolt:offsetStartMm")?;
    let spacing_mode = text(p, "steel:bolt:spacingMode", "token")?.to_owned();
    let spacing_value = number(p, "steel:bolt:spacingValueMm")?;
    let spacings = bolt_f64s(p, "steel:bolt:spacingsMm")?;
    let length_mode = text(p, "steel:bolt:lengthMode", "token")?.to_owned();
    let length = number(p, "steel:bolt:lengthMm")?;
    let washer_under_head = matches!(
        required(p, "steel:bolt:washerUnderHead", "bool")?,
        Value::Bool(true)
    );
    let washer_under_nut = matches!(
        required(p, "steel:bolt:washerUnderNut", "bool")?,
        Value::Bool(true)
    );
    let hole_mode = text(p, "steel:bolt:holeMode", "token")?.to_owned();
    let hole_clearance_mm = optional_number(p, "steel:bolt:holeClearanceMm")?;
    let record = BoltGroup {
        connects: Vec::new(),
        fastener: digest,
        point1_mm: point1,
        point2_mm: point2,
        plane_normal_mm: normal,
        rotation_rad: rotation,
        on_plane_mm: on_plane,
        depth_mm: depth,
        offset_start_mm: offset_start,
        spacing_mode,
        spacing_value_mm: spacing_value,
        spacings_mm: spacings,
        length_mode,
        length_mm: length,
        washer_under_head,
        washer_under_nut,
        hole_mode,
        hole_clearance_mm,
    };
    bolt_group_values(&record)?;
    bolt_group_against(&pinned_fastener(prims, digest)?, &record)
}

/// The authored plane-cut records of a created plate or member (A03): finite and within the
/// coordinate cap, with a non-zero normal, in ascending ordinals at most 999. Whether a plane
/// crosses the element is the engine's gate.
pub(super) fn cut_values(cuts: &[CutRecord]) -> Result<(), Diagnostic> {
    let mut last_ordinal = None;
    for cut in cuts {
        // The record admission keeps the model's codes: non-finite and a zero normal are
        // invalid, the coordinate cap is a limit.
        match cut.validate() {
            Ok(()) => (),
            Err(error) if error.code == ErrorCode::LimitExceeded => return Err(limit()),
            Err(_) => return Err(invalid()),
        }
        if cut.ordinal > 999 {
            return Err(limit());
        }
        if last_ordinal.is_some_and(|last| last >= cut.ordinal) {
            return Err(invalid());
        }
        last_ordinal = Some(cut.ordinal);
    }
    Ok(())
}

fn plate_corner_records(p: &Prim) -> Result<(), Diagnostic> {
    use crate::model::command::{CornerTreatment, PlateCorner};
    let fields = [
        ("steel:plate:cornerVertex", "int[]"),
        ("steel:plate:cornerKind", "token[]"),
        ("steel:plate:cornerD1Mm", "double[]"),
        ("steel:plate:cornerD2Mm", "double[]"),
        ("steel:plate:cornerRadiusMm", "double[]"),
    ];
    let mut arrays = Vec::new();
    let mut absent = 0usize;
    for (name, ty) in fields {
        match optional(p, name, ty)? {
            None => absent += 1,
            Some(Value::Array(a)) => arrays.push(a),
            _ => return Err(invalid()),
        }
    }
    if absent == 5 {
        return Ok(());
    }
    if absent != 0 {
        return Err(invalid());
    }
    let [vertices, kinds, d1s, d2s, radii] = arrays.as_slice() else {
        return Err(invalid());
    };
    if vertices.len() > 1024 {
        return Err(limit());
    }
    if [kinds.len(), d1s.len(), d2s.len(), radii.len()]
        .iter()
        .any(|n| *n != vertices.len())
    {
        return Err(invalid());
    }
    let count = points(p, "steel:plate:contourPointsMm")?.len();
    let mut seen = Vec::new();
    for ((((v, k), d1), d2), r) in vertices
        .iter()
        .zip(kinds.iter())
        .zip(d1s.iter())
        .zip(d2s.iter())
        .zip(radii.iter())
    {
        let Value::Int(v) = v else {
            return Err(invalid());
        };
        let v = usize::try_from(*v).map_err(|_| invalid())?;
        if v >= count || seen.contains(&v) {
            return Err(invalid());
        }
        seen.push(v);
        let number = |n: &Value| match n {
            Value::Number(n) => Ok(*n),
            Value::Int(n) => Ok(*n as f64),
            _ => Err(invalid()),
        };
        let (d1, d2, r) = (number(d1)?, number(d2)?, number(r)?);
        let treatment = match k {
            Value::Text(k) if k == "chamfer" && r == 0. => CornerTreatment::Chamfer {
                d1_mm: d1,
                d2_mm: d2,
            },
            Value::Text(k) if k == "round" && d1 == 0. && d2 == 0. => {
                CornerTreatment::Round { radius_mm: r }
            }
            _ => return Err(invalid()),
        };
        PlateCorner {
            vertex: v,
            treatment,
        }
        .validate()
        .map_err(|_| invalid())?;
    }
    Ok(())
}

/// Validates one owned plate, beam or column prim of a loaded layer: the operation history, the
/// shape inputs and the owned records. Moved unchanged from the platform's `snapshot`.
pub(super) fn steel_part(
    kind: &str,
    p: &Prim,
    prims: &[Prim],
    operation_ids: &mut std::collections::BTreeSet<beamcraft_model::id::OpId>,
) -> Result<(), Diagnostic> {
    let hole_history = cut_history::read_holes(p)?;
    let member_history = cut_history::read_member_holes(p)?;
    let history = cut_history::read(p)?;
    for id in history
        .retired
        .into_iter()
        .chain(hole_history.retired)
        .chain(member_history.retired)
        .chain(cut_history::ids(p, "steel:cut:operationId")?)
        .chain(cut_history::ids(p, "steel:hole:operationIds")?)
        .chain(cut_history::ids(p, "steel:mhole:operationIds")?)
    {
        if !operation_ids.insert(id) {
            return Err(invalid());
        }
    }
    match kind {
        "plate" => {
            // The form token discriminates the stored record: a lofted plate (A11)
            // carries the three loft attributes and no plate extrusion inputs; an
            // extruded plate stores neither the token nor the loft attributes.
            let lofted = match p.properties.iter().find(|a| a.name == "steel:plate:form") {
                None => {
                    if p.properties.iter().any(|a| {
                        matches!(
                            a.name.as_str(),
                            "steel:plate:loftStartMm" | "steel:plate:loftEndMm"
                        )
                    }) {
                        return Err(invalid());
                    }
                    false
                }
                Some(_) => {
                    if text(p, "steel:plate:form", "token")? != "lofted" {
                        return Err(invalid());
                    }
                    if p.properties.iter().any(|a| {
                        matches!(
                            a.name.as_str(),
                            "steel:plate:contourPointsMm" | "steel:plate:thicknessMm"
                        )
                    }) {
                        return Err(invalid());
                    }
                    true
                }
            };
            if lofted {
                lofted_plate(
                    &points(p, "steel:plate:loftStartMm")?,
                    &points(p, "steel:plate:loftEndMm")?,
                )?;
            } else {
                plate(
                    &points(p, "steel:plate:contourPointsMm")?,
                    number(p, "steel:plate:thicknessMm")?,
                )?;
            }
            plate_corner_records(p)?;
            plate_holes(p)?;
            plate_hole_records(p)?;
            element_cut(p)?;
        }
        "beam" | "column" => {
            let axis = points(p, "bim:axisPointsMm")?;
            if let Some(prop) = p
                .properties
                .iter()
                .find(|v| v.name == "steel:member:pathType")
            {
                if prop.type_name != "token"
                    || text(p, "steel:member:pathType", "token")? != "arc"
                    || axis.len() != 3
                {
                    return Err(invalid());
                }
                if p.properties
                    .iter()
                    .any(|v| v.name == "steel:member:rollRad")
                {
                    return Err(invalid());
                }
            }
            if axis.len() < 2 || axis.len() > MAX_MEMBER_PATH_POINTS {
                return Err(invalid());
            }
            let profile =
                link(p, prims, "bim:profile", &PROFILE_KINDS, true)?.ok_or_else(invalid)?;
            let profile = read_definition(profile, text(profile, "bim:definitionKind", "token")?)?
                .ok_or_else(invalid)?;
            definition(&profile)?;
            // A member pins a section profile: a rectangle for its width and height, a section
            // for its extent. A material or a fastener definition never hangs off `bim:profile`.
            let (width, height) = match &profile {
                Definition::Discipline(hook) => {
                    if let Some(def) = hook.0.as_any().downcast_ref::<RectangleProfileDef>() {
                        (def.width_mm, def.height_mm)
                    } else if let Some(def) = hook.0.as_any().downcast_ref::<SectionProfileDef>() {
                        def.shape.extent_mm()
                    } else {
                        return Err(invalid());
                    }
                }
                _ => return Err(invalid()),
            };
            // The size reference is the total path length for a polybeam and the one
            // chord for a straight member; both sum the same first chord.
            let mut length = 0.0;
            for (a, b) in axis.iter().zip(axis.iter().skip(1)) {
                length += norm(difference(*b, *a));
            }
            let half_width = width / 2.0;
            let half_height = height / 2.0;
            let size = length.max((half_width * half_width + half_height * half_height).sqrt());
            if size > 1e6 {
                return Err(limit());
            }
            let eps = 0.001f64.max(size / 4194304.0);
            // Every chord, not only the total, must clear the geometric floor.
            let mut short = length < eps;
            for (a, b) in axis.iter().zip(axis.iter().skip(1)) {
                short |= norm(difference(*b, *a)) < eps;
            }
            if short || width < eps || height < eps {
                return Err(invalid());
            }
            if let Some(Value::Number(n)) = p
                .properties
                .iter()
                .find(|v| v.name == "steel:member:rollRad")
                .and_then(|v| v.value.as_ref())
            {
                cap([*n], std::f64::consts::TAU)?;
            }
            element_cut(p)?;
            member_hole_records(p)?;
        }
        _ => {}
    }
    Ok(())
}

/// Validates the properties of a weld or bolt-group connection prim; the rest of a connection
/// is the platform's. Moved unchanged from the platform's `snapshot`.
pub(super) fn steel_connection(p: &Prim, prims: &[Prim]) -> Result<(), Diagnostic> {
    let connection_type = text(p, "steel:connection:type", "token")?;
    let has = |prefix: &str| p.properties.iter().any(|v| v.name.starts_with(prefix));
    let placed = has("steel:bolt:fastener");
    match connection_type {
        "weld" if !has("steel:bolt:") => weld_properties(p)?,
        // An A05 annotation group and an A14.3 group are discriminated by
        // the definition digest; a mixed property set is never guessed.
        "bolt_group" if !has("steel:weld:") && placed => bolt_group_properties(p, prims)?,
        "bolt_group" if !has("steel:weld:") && !has("steel:bolt:point1Mm") => bolt_properties(p)?,
        _ => return Err(invalid().with_rule("DV-ATTR-002")),
    }
    Ok(())
}

/// A06 and A05 `connects` records (domain vocabulary section 5.1): only a connection — a weld or
/// a bolt group — carries one, with exactly two distinct targets (`DV-REL-004`, `DV-REL-007`),
/// never itself (`DV-REL-006`), each an active owned plate, beam or column (`DV-REL-001`
/// dangling, `DV-REL-003` kind). An assembly is a valid target in the vocabulary but not for a
/// weld or a bolt group in this slice.
pub(super) fn connects(prims: &[Prim]) -> Result<(), Diagnostic> {
    let elements: BTreeMap<&str, (&Prim, &str)> = prims
        .iter()
        .filter_map(|p| supported_kind(p).map(|k| (p.path.as_str(), (p, k))))
        .collect();
    for (&path, &(p, kind)) in &elements {
        let Some(r) = relationship(p, "bim:connects") else {
            if kind == "connection" {
                return Err(fail(
                    ErrorCode::DependencyConflict,
                    "a connection connects exactly two parts",
                )
                .with_rule("DV-REL-004"));
            }
            continue;
        };
        if kind != "connection" {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "only a connection carries a connects record",
            )
            .with_rule("DV-REL-003"));
        }
        // An A14.3 bolt group carries 2 to 1024 targets (the vocabulary's own range); a weld
        // and an A05 group carry exactly two.
        let target_cap = if p.properties.iter().any(|v| v.name == "steel:bolt:fastener") {
            MAX_RELATIONSHIP_TARGETS
        } else {
            2
        };
        if r.targets.len() < 2 || r.targets.len() > target_cap {
            return Err(fail(
                ErrorCode::DependencyConflict,
                "a connection connects exactly two parts",
            )
            .with_rule("DV-REL-004"));
        }
        for (index, first) in r.targets.iter().enumerate() {
            if r.targets.iter().skip(index + 1).any(|other| other == first) {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "a connection connects two distinct parts",
                )
                .with_rule("DV-REL-007"));
            }
        }
        for target in &r.targets {
            if target == path {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "a connection cannot connect itself",
                ));
            }
            match elements.get(target.as_str()) {
                Some((t, _)) if !t.active => {
                    return Err(fail(
                        ErrorCode::DanglingRelationship,
                        "connects target is not active",
                    )
                    .with_rule("DV-REL-001"));
                }
                Some((_, kind)) if is_part_kind(kind) => {}
                Some((_, "assembly")) => return Err(unsupported()),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::InvalidArgument,
                        "connects target is not a plate, beam or column",
                    ));
                }
                None if prims
                    .iter()
                    .any(|p| p.path == *target && p.element_id.is_some()) =>
                {
                    return Err(fail(
                        ErrorCode::InvalidArgument,
                        "connects target is not a supported owned part",
                    )
                    .with_rule("DV-REL-003"));
                }
                None => {
                    return Err(fail(
                        ErrorCode::DanglingRelationship,
                        "connects target unavailable",
                    )
                    .with_rule("DV-REL-001"));
                }
            }
        }
    }
    Ok(())
}
