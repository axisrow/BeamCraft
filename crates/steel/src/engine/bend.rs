// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A10 bent plates: fold and roll operation records and their certified bodies.
//!
//! The authored contour of a bent plate is its developed mid-surface outline (the unfold) in
//! the element-local plane `z = 0`; the plate carries bend operation records (A-2) that say how
//! the mid-surface leaves the plane. Bending about the mid-surface preserves the development,
//! so the unfolded outline is the authored contour and the analytic volume stays
//! `outline area x thickness`: one bend's annular-sector cross-section area is
//! `|angle| x thickness x (radius + thickness / 2)`, exactly the thickness times the developed
//! length the bend consumes.
//!
//! The body is one G13 ruled loft of the derived cross-section and its translation along the
//! fold-line or roll-axis direction, so a constant cross-section along that direction is a
//! precondition:
//!
//! * **Fold** records bend the sheet about lines parallel to one side pair of a rectangular
//!   blank. Several parallel folds are allowed; their bend regions (developed length
//!   `radius x |angle|` about the fold line) must fit the blank and must not overlap.
//! * A **roll** record rolls the sheet about the axis line into a cylinder section (a
//!   rectangular blank; the wrap angle follows from the developed length) or, with a second
//!   radius, into a cone section (the blank is the developed annular sector about the axis
//!   start, whose arc centre is the cone apex projection; the axis lies in the blank plane).
//!   The wrap angle stays below a full turn: the sealed body is a genus-0 shell.
//!
//! Both forms refuse corner treatments, holes, cuts and a thickness side: the unfold is the
//! mid-surface, so nothing of the flat-plate evaluation applies. The bend arcs are faceted to
//! at most [`ARC_CHORD_DEVIATION_MM`] and the kept deviation is reported as
//! `arc_deviation_mm`; the bent quantities are those of the faceted body.

use crate::engine::command::{AuthoredPlate, PlateBody};
use crate::kernel::ruled::{Loft, LoftFailure, LoftResult, MAX_LOFT_VERTICES, RuledContour};
use crate::model::command::{BendPlate, BendRecord, BendSpec, MAX_BENDS};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::solid::SolidMesh;
use beamcraft_geom::{Point3, Vec3};
use beamcraft_kernel::profile::Point2;
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::OpId;

/// The largest chord deviation a faceted bend arc keeps (the round-hole rule of F06 A-2 D-2).
pub const ARC_CHORD_DEVIATION_MM: f64 = 0.05;

/// Absolute tolerance in millimetres for authored blank coordinates: a rectangle's corners and
/// a sector's arcs are recognized within this band, far below fabrication precision and far
/// above the rounding of authored decimal input.
const BLANK_EPSILON_MM: f64 = 1e-6;

/// Points closer than this to their predecessor are the same vertex: tangent junctions are
/// computed through different formulas for the flat and the arc offsets and must not become
/// zero-length backtracking edges.
const DEDUPE_EPSILON_MM: f64 = 1e-9;

/// Vertex budget of one loft contour (two rings share the F06 mesh cap of 4096).
const CONTOUR_VERTEX_BUDGET: usize = (MAX_LOFT_VERTICES / 2) as usize;

/// A full turn, the wrap angle a sealed genus-0 roll stays below.
const FULL_TURN_RAD: f64 = core::f64::consts::TAU;

fn refused(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::UnsupportedGeometry, field, message)
}

fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "bend", message)
}

fn limited(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::LimitExceeded, "bend", message).with_rule("DV-LIM-001")
}

/// Maps a loft refusal onto the bend command surface; the codes keep their F04 meaning and the
/// message names the stage that refused the derived body.
fn map_loft(failure: &LoftFailure) -> CommandError {
    use beamcraft_kernel::ops::OpDefect;
    let message = match failure {
        LoftFailure::Contour(_) => "the bent cross-section was refused",
        LoftFailure::Profile(_) => "the bent cross-section is not a valid simple outline",
        LoftFailure::Body(f) => match f.defect {
            OpDefect::SectionPinched | OpDefect::SectionNotAdmissible | OpDefect::SectionOpen => {
                "the bent cross-section pinches or does not close"
            }
            OpDefect::ResultDegenerate => "the bent plate body is degenerate",
            OpDefect::ResultOverlapping => "the bent plate body meets itself",
            OpDefect::ResultTopology | OpDefect::ResultShells => {
                "the bent plate body is not one closed shell"
            }
            OpDefect::ResultVolume => "the bent plate volume failed its analytic self-check",
            _ => "the bent plate body failed certification",
        },
    };
    CommandError::new(failure.f04_code(), "bend", message)
}

/// Everything one accepted `BendPlate` produces for the adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct BendOutcome {
    /// The target's authored state with the record appended.
    pub authored: AuthoredPlate,
    /// Declared read and write sets.
    pub footprint: Footprint,
    /// Digest of the typed arguments, for replay and reuse decisions.
    pub fingerprint: ContentDigest,
}

/// Applies `BendPlate` to the target's authored state in memory. Nothing is evaluated here;
/// the adapter re-evaluates the body and refuses the whole command if the replay does.
pub fn bend_plate(
    command: &BendPlate,
    plate: Option<&AuthoredPlate>,
    operation_used: &dyn Fn(&OpId) -> bool,
) -> Result<BendOutcome, CommandError> {
    command.validate_arguments()?;
    let Some(authored) = plate else {
        return Err(refused("target_id", "bends apply to plates only"));
    };
    if authored.id != command.target_id {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "no such plate",
        ));
    }
    if operation_used(&command.operation_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "operation_id",
            "operation id already names a record of the project",
        ));
    }
    if authored.bends.len() >= MAX_BENDS {
        return Err(limited("the plate already carries the bend record cap"));
    }
    let rolling = matches!(command.spec, BendSpec::Roll { .. });
    let rolled = authored
        .bends
        .iter()
        .any(|r| matches!(r.spec, BendSpec::Roll { .. }));
    if rolled || (rolling && !authored.bends.is_empty()) {
        return Err(refused(
            "bend",
            "a rolled plate carries exactly one bend record and no fold",
        ));
    }
    let mut next = authored.clone();
    next.bends.push(BendRecord {
        operation_id: command.operation_id,
        spec: command.spec,
    });
    Ok(BendOutcome {
        footprint: Footprint::for_update(command.target_id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
        authored: next,
    })
}

/// Evaluates a plate that carries bend records. The caller routes here from
/// [`crate::engine::command::evaluate_plate`] because `bends` is not empty.
pub fn evaluate_bent(authored: &AuthoredPlate) -> Result<PlateBody, CommandError> {
    if !authored.corners.is_empty() {
        return Err(refused(
            "bend",
            "corner treatments on a bent plate are not evaluated",
        ));
    }
    if !authored.holes_mm.is_empty() || !authored.hole_records.is_empty() {
        return Err(refused("bend", "holes on a bent plate are not evaluated"));
    }
    if !authored.cuts.is_empty() {
        return Err(refused("bend", "cuts on a bent plate are not evaluated"));
    }
    if authored.thickness_side.is_some() {
        return Err(refused(
            "bend",
            "a bent plate unfolds about the mid-surface; a thickness side is not evaluated",
        ));
    }
    if authored.contour_points_mm.iter().any(|p| p[2] != 0.0) {
        return Err(refused(
            "contour_points_mm",
            "the developed blank of a bent plate lies in the plane z = 0",
        ));
    }
    // The flat paths refuse a clockwise contour in the G05 admission and the analytic volume
    // is the shoelace area's sign included: a bent plate refuses it too, before the body.
    let area = shoelace(&authored.contour_points_mm);
    if !area.is_finite() || area <= 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "contour_points_mm",
            "contour winding is clockwise or the contour is degenerate",
        ));
    }
    let mut folds: Vec<&BendSpec> = Vec::new();
    let mut rolls: Vec<&BendSpec> = Vec::new();
    for record in &authored.bends {
        // A committed record is untrusted input at replay; its arguments are re-admitted here.
        record.validate()?;
        match &record.spec {
            BendSpec::Fold { .. } => folds.push(&record.spec),
            BendSpec::Roll { .. } => rolls.push(&record.spec),
        }
    }
    let (loft, deviation) = match (folds.is_empty(), rolls.is_empty()) {
        (false, true) => fold_loft(authored, &folds)?,
        (true, false) => {
            let Some(roll) = rolls.first().copied() else {
                return Err(refused("bend", "a roll record is required"));
            };
            roll_loft(authored, roll)?
        }
        _ => {
            return Err(refused(
                "bend",
                "a rolled plate carries exactly one bend record and no fold",
            ));
        }
    };
    let solid = SolidMesh::from_operation(loft.op());
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for v in solid.vertices() {
        low[0] = low[0].min(v[0]);
        low[1] = low[1].min(v[1]);
        high[0] = high[0].max(v[0]);
        high[1] = high[1].max(v[1]);
    }
    Ok(PlateBody {
        result: None,
        solid,
        gross_volume_mm3: authored.thickness_mm * shoelace(&authored.contour_points_mm),
        width_mm: high[0] - low[0],
        length_mm: high[1] - low[1],
        thickness_mm: authored.thickness_mm,
        hole_count: 0,
        cut_applied: false,
        corner_treated: false,
        bent: true,
        outline_mm: authored
            .contour_points_mm
            .iter()
            .map(|p| [p[0], p[1]])
            .collect(),
        arc_deviation_mm: deviation,
    })
}

/// Shoelace area of a `z = 0` contour, the analytic unfolded area of the blank.
fn shoelace(points: &[[f64; 3]]) -> f64 {
    let mut total = 0.0;
    for i in 0..points.len() {
        let j = (i + 1) % points.len();
        let (a, b) = (
            points.get(i).copied().unwrap_or_default(),
            points.get(j).copied().unwrap_or_default(),
        );
        total += a[0] * b[1] - b[0] * a[1];
    }
    total / 2.0
}

/// The `(axis, cross)` unit frame of a bend line in the `z = 0` plane: `axis` along the line,
/// `cross` its left perpendicular, so `axis x cross = +Z`.
fn line_frame(a: [f64; 3], b: [f64; 3]) -> Result<([f64; 2], [f64; 2]), CommandError> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if !len.is_finite() || len <= BLANK_EPSILON_MM {
        return Err(invalid("the bend line needs two distinct points"));
    }
    Ok(([dx / len, dy / len], [-dy / len, dx / len]))
}

/// The recognized blank of a fold set or a cylinder roll: a rectangle whose one side pair is
/// parallel to the bend direction. Returns the extents along the axis and across it.
fn rectangle_blank(
    points: &[[f64; 3]],
    axis: [f64; 2],
    cross: [f64; 2],
) -> Result<(f64, f64), CommandError> {
    let mut s = [f64::INFINITY, f64::NEG_INFINITY];
    let mut m = [f64::INFINITY, f64::NEG_INFINITY];
    for p in points {
        let (ps, pm) = (dot2(*p, axis), dot2(*p, cross));
        s[0] = s[0].min(ps);
        s[1] = s[1].max(ps);
        m[0] = m[0].min(pm);
        m[1] = m[1].max(pm);
    }
    let at = |v: f64, bound: f64| (v - bound).abs() <= BLANK_EPSILON_MM;
    for p in points {
        let (ps, pm) = (dot2(*p, axis), dot2(*p, cross));
        if !((at(ps, s[0]) || at(ps, s[1])) && (at(pm, m[0]) || at(pm, m[1]))) {
            return Err(refused(
                "contour_points_mm",
                "the blank of a bent plate must be a rectangle with one side pair parallel to the bend line",
            ));
        }
    }
    let (along, across) = (s[1] - s[0], m[1] - m[0]);
    if along <= BLANK_EPSILON_MM || across <= BLANK_EPSILON_MM {
        return Err(refused("contour_points_mm", "the blank has no extent"));
    }
    Ok((along, across))
}

fn dot2(p: [f64; 3], d: [f64; 2]) -> f64 {
    p[0] * d[0] + p[1] * d[1]
}

/// One element of the developed mid-line: a flat run or a true arc of the mid-surface.
enum Elem {
    /// A straight run from `p0` to `p1`.
    Flat { p0: [f64; 2], p1: [f64; 2] },
    /// A circular arc of the mid-surface, `sweep` signed (positive turns left).
    Arc {
        centre: [f64; 2],
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

/// Facet segments of one arc: at most [`ARC_CHORD_DEVIATION_MM`] sagitta, at least two
/// segments, the rule of the round hole and A12.2 corner arcs.
fn facet_segments(radius: f64, sweep: f64) -> usize {
    if radius <= ARC_CHORD_DEVIATION_MM {
        return 2;
    }
    let step = 2.0 * (1.0 - ARC_CHORD_DEVIATION_MM / radius).acos();
    ((sweep / step).ceil() as usize).max(2)
}

/// Pushes a point unless it repeats the last one within the dedupe tolerance.
fn push_point(out: &mut Vec<[f64; 2]>, p: [f64; 2]) {
    if let Some(last) = out.last() {
        let (dx, dy) = (p[0] - last[0], p[1] - last[1]);
        if (dx * dx + dy * dy).sqrt() <= DEDUPE_EPSILON_MM {
            return;
        }
    }
    out.push(p);
}

/// Arc points from `start` over `sweep` (signed) in `segments` chords at `radius`.
fn arc_points(
    centre: [f64; 2],
    radius: f64,
    start: f64,
    sweep: f64,
    segments: usize,
) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(segments + 1);
    for k in 0..=segments {
        let angle = start + sweep * (k as f64) / (segments as f64);
        let (sin, cos) = angle.sin_cos();
        out.push([centre[0] + radius * cos, centre[1] + radius * sin]);
    }
    out
}

/// Builds the closed counter-clockwise cross-section of the sheet from its mid-line elements:
/// the right offset walked forward, the end cut, the left offset walked backward and the start
/// cut. Returns the polygon and the largest chord deviation any faceted arc keeps.
fn assemble_section(
    elems: &[Elem],
    thickness_mm: f64,
) -> Result<(Vec<[f64; 2]>, f64), CommandError> {
    let half = thickness_mm / 2.0;
    let mut right: Vec<[f64; 2]> = Vec::new();
    let mut left: Vec<[f64; 2]> = Vec::new();
    let mut deviation = 0.0_f64;
    let mut budget = CONTOUR_VERTEX_BUDGET;
    for elem in elems {
        match *elem {
            Elem::Flat { p0, p1 } => {
                let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
                let len = (dx * dx + dy * dy).sqrt();
                if !len.is_finite() || len < DEDUPE_EPSILON_MM {
                    continue;
                }
                let (rx, ry) = (dy / len, -dx / len);
                push_point(&mut right, [p0[0] + rx * half, p0[1] + ry * half]);
                push_point(&mut right, [p1[0] + rx * half, p1[1] + ry * half]);
                push_point(&mut left, [p0[0] - rx * half, p0[1] - ry * half]);
                push_point(&mut left, [p1[0] - rx * half, p1[1] - ry * half]);
                budget = budget.checked_sub(2).ok_or_else(|| {
                    limited("the bent cross-section needs more points than the loft budget")
                })?;
            }
            Elem::Arc {
                centre,
                radius,
                start,
                sweep,
            } => {
                // The offset on the centre side keeps `radius - t/2`, the far side
                // `radius + t/2`; the larger of the two governs the facet count.
                let (left_radius, right_radius) = if sweep > 0.0 {
                    (radius - half, radius + half)
                } else {
                    (radius + half, radius - half)
                };
                if left_radius <= 0.0 || right_radius <= 0.0 {
                    return Err(refused(
                        "bend",
                        "the sheet does not fit inside its own bend radius",
                    ));
                }
                let segments = facet_segments(left_radius.max(right_radius), sweep.abs());
                budget = budget.checked_sub(2 * (segments + 1)).ok_or_else(|| {
                    limited("the bent cross-section needs more points than the loft budget")
                })?;
                deviation = deviation.max(
                    left_radius.max(right_radius)
                        * (1.0 - (sweep.abs() / (2.0 * segments as f64)).cos()),
                );
                for p in arc_points(centre, right_radius, start, sweep, segments) {
                    push_point(&mut right, p);
                }
                for p in arc_points(centre, left_radius, start, sweep, segments) {
                    push_point(&mut left, p);
                }
            }
        }
    }
    if right.len() < 2 || left.len() < 2 {
        return Err(refused("bend", "the bent cross-section is empty"));
    }
    let mut points = right;
    let Some(end) = left.last().copied() else {
        return Err(refused("bend", "the bent cross-section is empty"));
    };
    points.push(end);
    let interior = left.len() - 1;
    points.extend(left.drain(..interior).rev());
    if points.len() < 3 {
        return Err(refused("bend", "the bent cross-section is degenerate"));
    }
    Ok((points, deviation))
}

/// Places a cross-section polygon as a loft contour in the element-local frame: the section's
/// `x` runs along `x_axis`, its `y` along `y_axis`, both anchored at `origin`.
fn contour_of(
    origin: [f64; 3],
    x_axis: [f64; 3],
    y_axis: [f64; 3],
    points: &[[f64; 2]],
) -> Result<RuledContour, CommandError> {
    let vx = Vec3::new(x_axis[0], x_axis[1], x_axis[2]).map_err(|_| non_finite_bend())?;
    let vy = Vec3::new(y_axis[0], y_axis[1], y_axis[2]).map_err(|_| non_finite_bend())?;
    let at = Point3::try_new(origin[0], origin[1], origin[2]).map_err(|_| non_finite_bend())?;
    let mut flat = Vec::with_capacity(points.len());
    for p in points {
        flat.push(Point2::new(p[0], p[1]));
    }
    RuledContour::polygon(at, vx, vy, flat).map_err(|_| non_finite_bend())
}

fn non_finite_bend() -> CommandError {
    CommandError::new(ErrorCode::NonFiniteNumber, "bend", "value is not finite")
        .with_rule("DV-ATTR-003")
}

/// Seals the loft of a derived cross-section and its translation along the body axis. The
/// section's `x` runs along the axis' left perpendicular `cross`, its `y` along `+Z`, so the
/// contour frame normal is the axis itself.
fn seal_loft(
    section: &[[f64; 2]],
    origin: [f64; 3],
    axis: [f64; 2],
    length_mm: f64,
) -> Result<LoftResult, CommandError> {
    let x_axis = [-axis[1], axis[0], 0.0];
    let y_axis = [0.0, 0.0, 1.0];
    let start = contour_of(origin, x_axis, y_axis, section)?;
    let end_origin = [
        origin[0] + axis[0] * length_mm,
        origin[1] + axis[1] * length_mm,
        origin[2],
    ];
    let end = contour_of(end_origin, x_axis, y_axis, section)?;
    loft(&Loft { start, end })
}

fn loft(input: &Loft) -> Result<LoftResult, CommandError> {
    crate::kernel::ruled::loft(input).map_err(|f| map_loft(&f))
}

/// The folded body of a plate with fold records: the bent cross-section derived from the
/// rectangular blank and the folds, lofted along the fold direction.
///
/// The mid-line walk is the isometric image of the developed blank: flats keep their developed
/// lengths and each bend is one true arc of the mid radius whose developed length is
/// `mid_radius x |angle|` about the fold line.
fn fold_loft(
    authored: &AuthoredPlate,
    folds: &[&BendSpec],
) -> Result<(LoftResult, f64), CommandError> {
    let mut specs: Vec<([f64; 3], [f64; 3], f64, f64)> = Vec::with_capacity(folds.len());
    for spec in folds {
        if let BendSpec::Fold {
            a_mm,
            b_mm,
            radius_mm,
            angle_rad,
        } = **spec
        {
            specs.push((a_mm, b_mm, radius_mm, angle_rad));
        }
    }
    let (first_a, first_b) = match specs.first() {
        Some((a, b, _, _)) => (*a, *b),
        None => return Err(refused("bend", "a fold record is required")),
    };
    let (axis, cross) = line_frame(first_a, first_b)?;
    let (along, across) = rectangle_blank(&authored.contour_points_mm, axis, cross)?;
    // Every fold line is parallel to the first, at its own cross offset.
    let mut min_cross = f64::INFINITY;
    for p in &authored.contour_points_mm {
        min_cross = min_cross.min(dot2(*p, cross));
    }
    let mut positions: Vec<(f64, f64, f64)> = Vec::with_capacity(specs.len());
    for (a, b, radius, angle) in &specs {
        let (line_axis, _) = line_frame(*a, *b)?;
        let sine = (line_axis[0] * axis[1] - line_axis[1] * axis[0]).abs();
        if sine > 1e-9 || (dot2(*a, cross) - dot2(*b, cross)).abs() > BLANK_EPSILON_MM {
            return Err(refused("bend", "the folds of one plate must be parallel"));
        }
        positions.push((dot2(*a, cross) - min_cross, *radius, *angle));
    }
    positions.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(core::cmp::Ordering::Equal));
    // Walk the developed mid-line: the bend regions (developed length `mid radius x |angle|`
    // about the fold line) must fit the blank and must not overlap.
    let thickness = authored.thickness_mm;
    let mut elems: Vec<Elem> = Vec::new();
    let mut cursor = [0.0, 0.0];
    let mut heading = [1.0, 0.0];
    let mut developed = 0.0_f64;
    for (offset, radius, angle) in &positions {
        let mid_radius = radius + thickness / 2.0;
        let half = mid_radius * angle.abs() / 2.0;
        let (region_start, region_end) = (offset - half, offset + half);
        if region_start <= BLANK_EPSILON_MM || region_end >= across - BLANK_EPSILON_MM {
            return Err(refused("bend", "a bend region does not fit the blank"));
        }
        if region_start < developed {
            return Err(refused("bend", "the bend regions overlap"));
        }
        let flat = region_start - developed;
        elems.push(Elem::Flat {
            p0: cursor,
            p1: [cursor[0] + heading[0] * flat, cursor[1] + heading[1] * flat],
        });
        cursor = [cursor[0] + heading[0] * flat, cursor[1] + heading[1] * flat];
        // Arc: the centre sits on the left of travel for a positive angle, on the right
        // otherwise; the sweep is the authored signed angle.
        let side = if *angle > 0.0 {
            [-heading[1], heading[0]]
        } else {
            [heading[1], -heading[0]]
        };
        let centre = [
            cursor[0] + side[0] * mid_radius,
            cursor[1] + side[1] * mid_radius,
        ];
        let start_angle = (cursor[1] - centre[1]).atan2(cursor[0] - centre[0]);
        elems.push(Elem::Arc {
            centre,
            radius: mid_radius,
            start: start_angle,
            sweep: *angle,
        });
        cursor = [
            centre[0] + mid_radius * (start_angle + angle).cos(),
            centre[1] + mid_radius * (start_angle + angle).sin(),
        ];
        let (sin, cos) = angle.sin_cos();
        heading = [
            heading[0] * cos - heading[1] * sin,
            heading[0] * sin + heading[1] * cos,
        ];
        developed = region_end;
    }
    let tail = across - developed;
    elems.push(Elem::Flat {
        p0: cursor,
        p1: [cursor[0] + heading[0] * tail, cursor[1] + heading[1] * tail],
    });
    let (section, deviation) = assemble_section(&elems, thickness)?;
    // The body axis is the fold direction, anchored at the blank's along-axis minimum.
    let origin = [
        axis[0] * along_min(&authored.contour_points_mm, axis),
        axis[1] * along_min(&authored.contour_points_mm, axis),
        0.0,
    ];
    Ok((seal_loft(&section, origin, axis, along)?, deviation))
}

/// The rolled body of a plate with one roll record: a cylinder or cone section about the axis.
fn roll_loft(authored: &AuthoredPlate, spec: &BendSpec) -> Result<(LoftResult, f64), CommandError> {
    let (a, b, radius, radius_b) = match *spec {
        BendSpec::Roll {
            a_mm,
            b_mm,
            radius_mm,
            radius_b_mm,
        } => (a_mm, b_mm, radius_mm, radius_b_mm),
        BendSpec::Fold { .. } => return Err(refused("bend", "a roll record is required")),
    };
    let thickness = authored.thickness_mm;
    let (axis, cross) = line_frame(a, b)?;
    match radius_b {
        None => {
            let (along, across) = rectangle_blank(&authored.contour_points_mm, axis, cross)?;
            let mid_radius = radius + thickness / 2.0;
            let sweep = across / mid_radius;
            if !sweep.is_finite() || sweep >= FULL_TURN_RAD {
                return Err(refused("bend", "the roll reaches a full turn"));
            }
            let segments = facet_segments(radius + thickness, sweep);
            let section = sector_points(radius, radius + thickness, sweep, segments);
            // The section is centred on its own origin, so the loft origin anchors on the
            // authored axis line at the blank's along minimum: two records for the same axis
            // line publish the same body wherever their endpoints sit on the line, and the
            // body spans the blank along the axis.
            let shift = along_min(&authored.contour_points_mm, axis) - dot2(a, axis);
            let origin = [a[0] + axis[0] * shift, a[1] + axis[1] * shift, 0.0];
            let result = seal_loft(&section, origin, axis, along)?;
            let deviation = (radius + thickness) * (1.0 - (sweep / (2.0 * segments as f64)).cos());
            Ok((result, deviation))
        }
        Some(radius_b) => cone_loft(authored, a, b, axis, radius, radius_b, thickness),
    }
}

fn along_min(points: &[[f64; 3]], axis: [f64; 2]) -> f64 {
    let mut low = f64::INFINITY;
    for p in points {
        low = low.min(dot2(*p, axis));
    }
    low
}

/// The closed counter-clockwise annular sector over `[0, sweep]`: the outer arc forward, the
/// end cut in, the inner arc backward, the start cut out.
fn sector_points(r_inner: f64, r_outer: f64, sweep: f64, segments: usize) -> Vec<[f64; 2]> {
    let mut points: Vec<[f64; 2]> = Vec::with_capacity(2 * segments + 2);
    for p in arc_points([0.0, 0.0], r_outer, 0.0, sweep, segments) {
        push_point(&mut points, p);
    }
    for p in arc_points([0.0, 0.0], r_inner, sweep, -sweep, segments) {
        push_point(&mut points, p);
    }
    points
}

/// The cone section: the blank is the developed annular sector about the apex `a`, the axis
/// runs in the blank plane from `a` towards `b`, and the sheet's mid radii at the two end
/// sections are the authored `radius` and `radius_b`.
#[allow(clippy::too_many_arguments)]
fn cone_loft(
    authored: &AuthoredPlate,
    a: [f64; 3],
    b: [f64; 3],
    axis: [f64; 2],
    radius: f64,
    radius_b: f64,
    thickness: f64,
) -> Result<(LoftResult, f64), CommandError> {
    let points = &authored.contour_points_mm;
    // Every boundary point sits on one of the two developed arcs about the apex.
    let mut s_in = f64::INFINITY;
    let mut s_out = f64::NEG_INFINITY;
    let mut angles: Vec<f64> = Vec::with_capacity(points.len());
    for p in points {
        let (dx, dy) = (p[0] - a[0], p[1] - a[1]);
        let distance = (dx * dx + dy * dy).sqrt();
        s_in = s_in.min(distance);
        s_out = s_out.max(distance);
        angles.push(dy.atan2(dx));
    }
    if s_out - s_in <= BLANK_EPSILON_MM {
        return Err(refused(
            "contour_points_mm",
            "the developed sector of a cone roll has no radial extent",
        ));
    }
    let on_arc = |distance: f64| {
        (distance - s_in).abs() <= BLANK_EPSILON_MM || (distance - s_out).abs() <= BLANK_EPSILON_MM
    };
    let mut on_arcs_only = true;
    for p in points {
        let (dx, dy) = (p[0] - a[0], p[1] - a[1]);
        if !on_arc((dx * dx + dy * dy).sqrt()) {
            on_arcs_only = false;
        }
    }
    if !on_arcs_only {
        return Err(refused(
            "contour_points_mm",
            "the blank of a cone roll must be an annular sector about the roll axis start",
        ));
    }
    // Cone consistency: one common half-aperture from both radii, and the axial span agrees
    // with the slant span (slant squared = axial squared + radial step squared).
    let sin_a = radius / s_in;
    let sin_b = radius_b / s_out;
    if !(0.0 < sin_a && sin_a < 1.0) || (sin_a - sin_b).abs() > 1e-9 {
        return Err(refused("bend", "the roll radii do not share one cone apex"));
    }
    let sin_phi = sin_a;
    let cos_phi = (1.0 - sin_phi * sin_phi).sqrt();
    let axial = (b[0] - a[0]).hypot(b[1] - a[1]);
    let slant = s_out - s_in;
    let radial_step = radius_b - radius;
    let expected = slant * slant - radial_step * radial_step;
    if !cos_phi.is_finite()
        || cos_phi <= 0.0
        || (axial * axial - expected).abs() > 1e-6 * (1.0 + expected.abs())
    {
        return Err(refused(
            "bend",
            "the roll axis span disagrees with the developed sector",
        ));
    }
    // The developed angular extent: the complement of the largest gap about the apex. The
    // section carries the seam on its own +x direction, so the phase itself is not carried.
    angles.sort_by(|x, y| x.partial_cmp(y).unwrap_or(core::cmp::Ordering::Equal));
    let mut largest_gap = -1.0_f64;
    for k in 0..angles.len() {
        let next = angles.get((k + 1) % angles.len()).copied().unwrap_or(0.0);
        let current = angles.get(k).copied().unwrap_or(0.0);
        let gap = if k + 1 == angles.len() {
            next + FULL_TURN_RAD - current
        } else {
            next - current
        };
        if gap > largest_gap {
            largest_gap = gap;
        }
    }
    let sweep_developed = FULL_TURN_RAD - largest_gap;
    let sweep = sweep_developed / sin_phi;
    if sweep >= FULL_TURN_RAD {
        return Err(refused("bend", "the roll reaches a full turn"));
    }
    // The face radii of the two end sections; the inner faces must stay positive.
    let face = thickness / (2.0 * cos_phi);
    let (in_a, out_a) = (radius - face, radius + face);
    let (in_b, out_b) = (radius_b - face, radius_b + face);
    if in_a <= 0.0 || in_b <= 0.0 {
        return Err(refused("bend", "the sheet does not fit inside the cone"));
    }
    let segments = facet_segments(out_a.max(out_b), sweep);
    let section_a = sector_points(in_a, out_a, sweep, segments);
    let section_b = sector_points(in_b, out_b, sweep, segments);
    let x_axis = [-axis[1], axis[0], 0.0];
    let y_axis = [0.0, 0.0, 1.0];
    let start = contour_of([a[0], a[1], 0.0], x_axis, y_axis, &section_a)?;
    let end = contour_of([b[0], b[1], 0.0], x_axis, y_axis, &section_b)?;
    let result = loft(&Loft { start, end })?;
    let deviation = out_a.max(out_b) * (1.0 - (sweep / (2.0 * segments as f64)).cos());
    Ok((result, deviation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::command::PlateSide;
    use beamcraft_model::id::{ElementId, OpId};

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn op(n: u8) -> OpId {
        OpId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn plate(contour: Vec<[f64; 3]>, thickness: f64) -> AuthoredPlate {
        AuthoredPlate {
            id: id(1),
            name: "Bent".into(),
            class: 0,
            contour_points_mm: contour,
            thickness_mm: thickness,
            thickness_side: None,
            corners: Vec::new(),
            placement: beamcraft_model::command::Placement {
                translation_mm: [0.0; 3],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        }
    }

    fn rectangle(along: f64, across: f64) -> Vec<[f64; 3]> {
        vec![
            [0.0, 0.0, 0.0],
            [along, 0.0, 0.0],
            [along, across, 0.0],
            [0.0, across, 0.0],
        ]
    }

    fn fold(m: f64, radius: f64, angle: f64) -> BendSpec {
        BendSpec::Fold {
            a_mm: [0.0, m, 0.0],
            b_mm: [100.0, m, 0.0],
            radius_mm: radius,
            angle_rad: angle,
        }
    }

    fn close(actual: f64, expected: f64, relative: f64) {
        assert!(
            (actual - expected).abs() <= relative * expected.abs(),
            "{actual} vs {expected}"
        );
    }

    /// A right-angle fold keeps the mid-surface invariance: the volume is the thickness times
    /// the developed blank area, the outline reports the unfold, and the deviation is kept and
    /// reported.
    #[test]
    fn a_right_angle_fold_reports_the_unfold_and_the_invariant_volume() {
        let developed = 100.0 + 9.0 * core::f64::consts::FRAC_PI_2;
        let mut authored = plate(rectangle(100.0, developed), 2.0);
        authored.bends = vec![BendRecord {
            operation_id: op(2),
            spec: fold(50.0, 8.0, core::f64::consts::FRAC_PI_2),
        }];
        let body = crate::engine::command::evaluate_plate(&authored).unwrap();
        assert_eq!(
            body.algorithm(),
            crate::engine::command::BENT_VOLUME_ALGORITHM
        );
        // The faceted bend arcs keep the chords inside, so the mesh volume sits a little below
        // the analytic value, bounded by the reported deviation.
        close(body.quantities().volume_mm3, 2.0 * 100.0 * developed, 2e-3);
        close(body.gross_volume_mm3, 2.0 * 100.0 * developed, 1e-9);
        close(body.width_mm, 100.0, 1e-9);
        // The section runs from the blank's cross minimum: the outer face of the bend reaches
        // `region start + outer radius` across.
        let extent = 50.0 - 9.0 * core::f64::consts::FRAC_PI_4 + 10.0;
        close(body.length_mm, extent, 1e-9);
        assert_eq!(body.outline_mm.len(), 4);
        assert!(body.arc_deviation_mm > 0.0 && body.arc_deviation_mm <= 0.05);
        assert!(!body.is_rectangular_box());
    }

    /// Two radii on the same developed blank bend the same volume out of it: the radii only
    /// redistribute the material.
    #[test]
    fn two_bend_radii_keep_the_same_volume_on_one_blank() {
        let mut thin = plate(rectangle(100.0, 200.0), 2.0);
        thin.bends = vec![BendRecord {
            operation_id: op(2),
            spec: fold(50.0, 8.0, core::f64::consts::FRAC_PI_2),
        }];
        let mut thick = plate(rectangle(100.0, 200.0), 2.0);
        thick.bends = vec![BendRecord {
            operation_id: op(3),
            spec: fold(100.0, 20.0, core::f64::consts::FRAC_PI_2),
        }];
        let thin_volume = crate::engine::command::evaluate_plate(&thin)
            .unwrap()
            .quantities()
            .volume_mm3;
        let thick_volume = crate::engine::command::evaluate_plate(&thick)
            .unwrap()
            .quantities()
            .volume_mm3;
        close(thin_volume, 40_000.0, 2e-3);
        close(thick_volume, 40_000.0, 2e-3);
    }

    /// A cylinder roll of a rectangular blank: the wrap angle follows from the developed
    /// length, and the volume is the thickness times the developed area.
    #[test]
    fn a_cylinder_roll_wraps_the_developed_length() {
        let mut authored = plate(rectangle(100.0, 20.0), 2.0);
        authored.bends = vec![BendRecord {
            operation_id: op(2),
            spec: BendSpec::Roll {
                a_mm: [0.0, 0.0, 0.0],
                b_mm: [100.0, 0.0, 0.0],
                radius_mm: 8.0,
                radius_b_mm: None,
            },
        }];
        let body = crate::engine::command::evaluate_plate(&authored).unwrap();
        // The wrap's single arc is faceted to the deviation rule, so the mesh keeps its chords
        // inside the true cylinder section.
        close(body.quantities().volume_mm3, 2.0 * 20.0 * 100.0, 1.5e-2);
        close(body.width_mm, 100.0, 1e-9);
    }

    /// The cylinder ring stays centred on the authored axis line, not on the element origin:
    /// an axis offset across the blank translates the ring by that offset.
    #[test]
    fn a_cylinder_roll_centres_the_ring_on_the_authored_axis() {
        let mut authored = plate(rectangle(100.0, 20.0), 2.0);
        authored.bends = vec![BendRecord {
            operation_id: op(2),
            spec: BendSpec::Roll {
                a_mm: [0.0, 500.0, 0.0],
                b_mm: [100.0, 500.0, 0.0],
                radius_mm: 8.0,
                radius_b_mm: None,
            },
        }];
        let body = crate::engine::command::evaluate_plate(&authored).unwrap();
        // The section is centred on the axis at the loft origin `a`, so the ring's vertices
        // sit within one outer radius of the axis line across, and along the axis the body
        // spans the blank.
        let mut cross_low = f64::INFINITY;
        let mut cross_high = f64::NEG_INFINITY;
        for v in body.solid.vertices() {
            cross_low = cross_low.min(v[1]);
            cross_high = cross_high.max(v[1]);
        }
        // The ring around (y = 500) reaches from 500 - 10 to 500 + 10; nothing else would put
        // the body there.
        assert!(
            cross_low > 480.0 && cross_high < 520.0,
            "{cross_low}..{cross_high}"
        );
        assert!(cross_low > 0.0, "the ring is not at the origin");
        close(body.width_mm, 100.0, 1e-9);
        // Two records for the same axis line publish the same body wherever their endpoints
        // sit on the line: the anchor is the line point at the blank's along minimum.
        let mut same_line = plate(rectangle(100.0, 20.0), 2.0);
        same_line.bends = vec![BendRecord {
            operation_id: op(2),
            spec: BendSpec::Roll {
                a_mm: [50.0, 500.0, 0.0],
                b_mm: [150.0, 500.0, 0.0],
                radius_mm: 8.0,
                radius_b_mm: None,
            },
        }];
        let other = crate::engine::command::evaluate_plate(&same_line).unwrap();
        assert_eq!(other.solid.blob(), body.solid.blob());
    }

    /// A cone roll of a sector blank about the apex: the volume matches the band closed form,
    /// which the development alone gives to machine precision.
    #[test]
    fn a_cone_roll_matches_the_band_closed_form() {
        let sweep = 0.8_f64;
        let mut points: Vec<[f64; 3]> = Vec::new();
        for k in 0..=16u32 {
            let angle = sweep * f64::from(k) / 16.0;
            let (sin, cos) = angle.sin_cos();
            points.push([190.0 * cos, 190.0 * sin, 0.0]);
        }
        for k in (0..=16u32).rev() {
            let angle = sweep * f64::from(k) / 16.0;
            let (sin, cos) = angle.sin_cos();
            points.push([90.0 * cos, 90.0 * sin, 0.0]);
        }
        let mut authored = plate(points, 2.0);
        authored.bends = vec![BendRecord {
            operation_id: op(2),
            spec: BendSpec::Roll {
                a_mm: [0.0, 0.0, 0.0],
                b_mm: [80.0, 0.0, 0.0],
                radius_mm: 54.0,
                radius_b_mm: Some(114.0),
            },
        }];
        let body = crate::engine::command::evaluate_plate(&authored).unwrap();
        close(body.quantities().volume_mm3, 22_400.0, 2e-3);
    }

    /// Every documented refusal keeps its code: the blank shape, the region fit, the mixed
    /// bend kinds, the flat-plate operations and the full-turn roll.
    #[test]
    fn unsupported_bends_are_refused_with_typed_errors() {
        let bend = |spec: BendSpec| BendRecord {
            operation_id: op(2),
            spec,
        };
        // The blank must be a rectangle with one side pair parallel to the fold line.
        let mut l_shape = plate(
            vec![
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 60.0, 0.0],
                [40.0, 60.0, 0.0],
                [40.0, 114.0, 0.0],
                [0.0, 114.0, 0.0],
            ],
            2.0,
        );
        l_shape.bends = vec![bend(fold(50.0, 8.0, core::f64::consts::FRAC_PI_2))];
        let error = crate::engine::command::evaluate_plate(&l_shape).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert_eq!(error.field, "contour_points_mm");
        // The bend regions must fit the blank and must not overlap.
        let mut edge = plate(rectangle(100.0, 200.0), 2.0);
        edge.bends = vec![bend(fold(3.0, 8.0, core::f64::consts::FRAC_PI_2))];
        assert_eq!(
            crate::engine::command::evaluate_plate(&edge)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        let mut overlap = plate(rectangle(100.0, 200.0), 2.0);
        overlap.bends = vec![
            bend(fold(50.0, 8.0, core::f64::consts::FRAC_PI_2)),
            BendRecord {
                operation_id: op(3),
                spec: fold(52.0, 8.0, core::f64::consts::FRAC_PI_2),
            },
        ];
        assert_eq!(
            crate::engine::command::evaluate_plate(&overlap)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        // A thickness side, holes, treatments and cuts never combine with bends.
        let mut sided = plate(rectangle(100.0, 200.0), 2.0);
        sided.thickness_side = Some(PlateSide::Middle);
        sided.bends = vec![bend(fold(50.0, 8.0, core::f64::consts::FRAC_PI_2))];
        assert_eq!(
            crate::engine::command::evaluate_plate(&sided)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        let mut holed = plate(rectangle(100.0, 200.0), 2.0);
        holed.holes_mm = vec![vec![[10.0, 10.0], [20.0, 10.0], [15.0, 20.0]]];
        holed.bends = vec![bend(fold(50.0, 8.0, core::f64::consts::FRAC_PI_2))];
        assert_eq!(
            crate::engine::command::evaluate_plate(&holed)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        // A roll must stay below a full turn: a long blank wraps past the seam.
        let mut long = plate(rectangle(100.0, 60.0), 2.0);
        long.bends = vec![bend(BendSpec::Roll {
            a_mm: [0.0, 0.0, 0.0],
            b_mm: [100.0, 0.0, 0.0],
            radius_mm: 8.0,
            radius_b_mm: None,
        })];
        assert_eq!(
            crate::engine::command::evaluate_plate(&long)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        // Cone consistency: the two radii must share one apex ratio with the sector.
        let mut cone = plate(rectangle(100.0, 20.0), 2.0);
        cone.bends = vec![bend(BendSpec::Roll {
            a_mm: [0.0, 0.0, 0.0],
            b_mm: [80.0, 0.0, 0.0],
            radius_mm: 54.0,
            radius_b_mm: Some(114.0),
        })];
        assert_eq!(
            crate::engine::command::evaluate_plate(&cone)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        // A zero fold angle is refused at the argument level.
        let mut flat = plate(rectangle(100.0, 200.0), 2.0);
        flat.bends = vec![bend(fold(50.0, 8.0, 0.0))];
        assert_eq!(
            crate::engine::command::evaluate_plate(&flat)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // A clockwise blank publishes a negative analytic area: refused like the flat paths
        // refuse one.
        let mut clockwise = plate(
            vec![
                [0.0, 0.0, 0.0],
                [0.0, 200.0, 0.0],
                [100.0, 200.0, 0.0],
                [100.0, 0.0, 0.0],
            ],
            2.0,
        );
        clockwise.bends = vec![bend(fold(50.0, 8.0, core::f64::consts::FRAC_PI_2))];
        let error = crate::engine::command::evaluate_plate(&clockwise).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(error.field, "contour_points_mm");
        // A cone whose two radii are equal is refused: it is a cylinder, and the stored
        // encoding keeps one radius for a cylinder, so it would wedge its own sector blank.
        let mut equal_radii = plate(rectangle(100.0, 20.0), 2.0);
        equal_radii.bends = vec![bend(BendSpec::Roll {
            a_mm: [0.0, 0.0, 0.0],
            b_mm: [80.0, 0.0, 0.0],
            radius_mm: 54.0,
            radius_b_mm: Some(54.0),
        })];
        assert_eq!(
            crate::engine::command::evaluate_plate(&equal_radii)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    /// The admission appends the record, declares the shape footprint, refuses a used or
    /// duplicated identity, the record cap and mixing a roll into an already bent plate.
    #[test]
    fn the_admission_appends_one_record_and_refuses_reuse() {
        let authored = plate(rectangle(100.0, 400.0), 2.0);
        let command = BendPlate {
            target_id: id(1),
            operation_id: op(2),
            spec: fold(20.0, 1.0, core::f64::consts::FRAC_PI_2),
        };
        let outcome = bend_plate(&command, Some(&authored), &|_| false).unwrap();
        assert_eq!(outcome.authored.bends.len(), 1);
        assert_eq!(outcome.fingerprint, command.content_fingerprint().unwrap());
        // A retry with the same identity under other content is a reuse.
        let error = bend_plate(
            &BendPlate {
                operation_id: op(2),
                spec: fold(30.0, 1.0, core::f64::consts::FRAC_PI_2),
                ..command
            },
            Some(&authored),
            &|id| *id == op(2),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::DuplicateId);
        // A missing target plate is refused.
        assert_eq!(
            bend_plate(&command, None, &|_| false).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
        // The record cap binds at sixteen; a roll never joins a bent plate.
        let mut full = authored.clone();
        for k in 0..MAX_BENDS {
            full.bends.push(BendRecord {
                operation_id: op(2),
                spec: fold(20.0 + 10.0 * k as f64, 1.0, core::f64::consts::FRAC_PI_2),
            });
        }
        assert_eq!(
            bend_plate(&command, Some(&full), &|_| false)
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
        let mut rolled = plate(rectangle(100.0, 20.0), 2.0);
        rolled.bends = vec![BendRecord {
            operation_id: op(2),
            spec: BendSpec::Roll {
                a_mm: [0.0, 0.0, 0.0],
                b_mm: [100.0, 0.0, 0.0],
                radius_mm: 8.0,
                radius_b_mm: None,
            },
        }];
        assert_eq!(
            bend_plate(
                &BendPlate {
                    target_id: id(1),
                    operation_id: op(3),
                    spec: fold(10.0, 1.0, core::f64::consts::FRAC_PI_2),
                },
                Some(&rolled),
                &|_| false
            )
            .unwrap_err()
            .code,
            ErrorCode::UnsupportedGeometry
        );
    }

    /// The bent body reaches the viewport unchanged: the sealed loft passes the scene face
    /// table rules and builds a view snapshot.
    #[test]
    fn a_bent_plate_builds_a_view_snapshot() {
        use beamcraft_engine::element::ElementKind;
        use beamcraft_engine::query::snapshot::ViewSnapshot;
        use beamcraft_geom::Translation3;
        let developed = 100.0 + 9.0 * core::f64::consts::FRAC_PI_2;
        let mut authored = plate(rectangle(100.0, developed), 2.0);
        authored.bends = vec![BendRecord {
            operation_id: op(2),
            spec: fold(50.0, 8.0, core::f64::consts::FRAC_PI_2),
        }];
        let body = crate::engine::command::evaluate_plate(&authored).unwrap();
        let element = beamcraft_engine::scene::SceneElement::new(
            authored.id,
            ElementKind::Plate,
            Translation3::new(0.0, 0.0, 0.0).unwrap(),
            body.solid.scene_mesh(),
        )
        .with_orientation([0.0, 0.0, 0.0, 1.0]);
        ViewSnapshot::build(
            beamcraft_engine::query::snapshot::ViewPin::new(
                beamcraft_model::digest::Sha256::digest(b"pin").unwrap(),
                1,
            ),
            vec![element],
            &beamcraft_engine::limits::Limits::default(),
        )
        .unwrap();
    }
}
