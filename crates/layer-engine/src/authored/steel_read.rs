// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The steel plate readers (#388 slice): the loaded-content checks of a plate's contour,
//! holes, hole records, member-hole records and cut records. Extracted from `validate.rs`
//! unchanged; the discipline reversal moves this file to `crates/steel`.

use super::steel_defs::read_definition;
use super::validate::{cap, finite, number, optional_number, points, required, resource, text};
use super::*;
use beamcraft_model::id::OpId;
use beamcraft_model::limits::{MAX_RELATIONSHIP_TARGETS, MIN_WELD_SEGMENT_MM};
use steel::model::{command::KeepSide, fastener::FastenerKind, fastener::FastenerSpec};

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
    input: &Prepare,
    prims: &[Prim],
    digest: ContentDigest,
) -> Result<FastenerSpec, Diagnostic> {
    if let Some(found) = input.resources.iter().find(|r| r.digest == digest) {
        return match &found.definition {
            Definition::Fastener { spec } => Ok(spec.clone()),
            _ => Err(invalid()),
        };
    }
    pinned_fastener(prims, digest)
}

/// The pinned fastener definition of a stored digest, re-admitted from its canonical bytes.
fn pinned_fastener(prims: &[Prim], digest: ContentDigest) -> Result<FastenerSpec, Diagnostic> {
    let p = resource(prims, digest, &["fastener"], DefinitionCategory::Fastener)?;
    match read_definition(p, "fastener")? {
        Definition::Fastener { spec } => Ok(spec),
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
