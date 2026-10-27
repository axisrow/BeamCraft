// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A14.3 bolt groups: the engine's decisions on `CreateBoltGroup` (bolt amendment section 4),
//! following the A05 bolt pattern.
//!
//! A bolt group is a `connection` element whose `connects` record names 2 to 1024 distinct
//! parts, the main part first, and whose shape names one pinned fastener definition. The engine
//! never touches USD: the adapter reads the parts' kinds into the same [`crate::weld::Parts`]
//! index a weld uses and supplies conservative world bounds, and the decisions here are pure,
//! bounded and deterministic. The frame follows the bolt amendment: `x` is the unit line
//! `point2 - point1`, the plane normal's component perpendicular to `x` is the shank axis `z`
//! at zero rotation, `y = z × x`, and fastener `i` sits at `offset_start + sum(spacings[..i])`
//! along `x`, slid by `on_plane` along `y` and `depth` along `z`; the shank runs along `+z`
//! towards the nut.
//!
//! Like A05, the bounds are conservative (they contain the real part), so a crossing decision
//! and the grip of the auto length are conservative upper shapes: the engine never proves a
//! real face intersection. The generated holes of `hole_mode = generate` are A14.4 and are
//! refused here.

use crate::bolt::Bounds;
use crate::footprint::{Entry, Footprint, Group, Target};
use crate::weld::PartKind;
use beamcraft_model::command::{BoltGroupPlacement, BoltLengthMode, CommandError, CreateBoltGroup};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::fastener::{FastenerKind, FastenerSpec, HoleMode};
use beamcraft_model::id::ElementId;
use beamcraft_model::limits::COORDINATE_CAP_MM;
use std::collections::BTreeMap;

/// Everything the adapter needs to author the group: the resolved settings and the footprint.
#[derive(Debug, Clone, PartialEq)]
pub struct BoltGroupDecision {
    /// The shank length the group was given: the fixed value or the auto choice, in
    /// millimetres.
    pub length_mm: f64,
    /// The resolved washer under the head.
    pub washer_under_head: bool,
    /// The resolved washer under the nut.
    pub washer_under_nut: bool,
    /// The resolved hole clearance in millimetres (the group's own or the definition's).
    pub hole_clearance_mm: f64,
    /// What the create writes and reads.
    pub footprint: Footprint,
}

fn error(code: ErrorCode, field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(code, field, message)
}

/// The unit vector and its input length, or `None` for a zero-length vector.
fn unit(v: &[f64; 3]) -> Option<[f64; 3]> {
    let squared = v.iter().map(|v| v * v).sum::<f64>();
    if squared <= 0.0 {
        return None;
    }
    let norm = squared.sqrt();
    Some([v[0] / norm, v[1] / norm, v[2] / norm])
}

/// The group frame of the bolt amendment: the line direction `x`, the shank axis `z` (the
/// plane normal's component perpendicular to `x`, rotated by `rotation_rad` about `x`) and
/// `y = z × x`.
pub fn frame(placement: &BoltGroupPlacement) -> Result<([f64; 3], [f64; 3], [f64; 3]), CommandError> {
    let [x1, y1, z1] = placement.point1_mm;
    let [x2, y2, z2] = placement.point2_mm;
    let x = unit(&[x2 - x1, y2 - y1, z2 - z1]).ok_or_else(|| {
        error(
            ErrorCode::InvalidArgument,
            "point2_mm",
            "the two points of the group line are distinct",
        )
    })?;
    let dot = placement.plane_normal[0] * x[0]
        + placement.plane_normal[1] * x[1]
        + placement.plane_normal[2] * x[2];
    let perpendicular = [
        placement.plane_normal[0] - dot * x[0],
        placement.plane_normal[1] - dot * x[1],
        placement.plane_normal[2] - dot * x[2],
    ];
    let normal_unit = unit(&perpendicular).ok_or_else(|| {
        error(
            ErrorCode::InvalidArgument,
            "plane_normal",
            "the plane normal is parallel to the group line",
        )
    })?;
    // Rodrigues about x: z' = z cos r + (x × z) sin r (x is unit and the parts of z along x
    // vanish, so the axis term drops).
    let (sin, cos) = placement.rotation_rad.sin_cos();
    let x_cross_z = [
        x[1] * normal_unit[2] - x[2] * normal_unit[1],
        x[2] * normal_unit[0] - x[0] * normal_unit[2],
        x[0] * normal_unit[1] - x[1] * normal_unit[0],
    ];
    let z = [
        normal_unit[0] * cos + x_cross_z[0] * sin,
        normal_unit[1] * cos + x_cross_z[1] * sin,
        normal_unit[2] * cos + x_cross_z[2] * sin,
    ];
    let y = [
        z[1] * x[2] - z[2] * x[1],
        z[2] * x[0] - z[0] * x[2],
        z[0] * x[1] - z[1] * x[0],
    ];
    Ok((x, y, z))
}

/// Whether the segment `from + t * direction` for `t` in `[0, length]` meets the box.
fn segment_meets_box(from: &[f64; 3], direction: &[f64; 3], length: f64, bounds: &Bounds) -> bool {
    let [[xmin, ymin, zmin], [xmax, ymax, zmax]] = *bounds;
    let (mut t_min, mut t_max) = (0.0_f64, length);
    for axis in 0..3 {
        let d = direction[axis];
        let o = from[axis];
        let (lo, hi) = match axis {
            0 => (xmin, xmax),
            1 => (ymin, ymax),
            _ => (zmin, zmax),
        };
        if d.abs() < 1e-12 {
            if o < lo || o > hi {
                return false;
            }
        } else {
            let mut t0 = (lo - o) / d;
            let mut t1 = (hi - o) / d;
            if t0 > t1 {
                core::mem::swap(&mut t0, &mut t1);
            }
            t_min = t_min.max(t0);
            t_max = t_max.min(t1);
            if t_min > t_max {
                return false;
            }
        }
    }
    true
}

/// The length of the segment `from + t * direction` inside the box, or `None`.
fn segment_inside_length(
    from: &[f64; 3],
    direction: &[f64; 3],
    length: f64,
    bounds: &Bounds,
) -> Option<f64> {
    let [[xmin, ymin, zmin], [xmax, ymax, zmax]] = *bounds;
    let (mut t_min, mut t_max) = (0.0_f64, length);
    for axis in 0..3 {
        let d = direction[axis];
        let o = from[axis];
        let (lo, hi) = match axis {
            0 => (xmin, xmax),
            1 => (ymin, ymax),
            _ => (zmin, zmax),
        };
        if d.abs() < 1e-12 {
            if o < lo || o > hi {
                return None;
            }
        } else {
            let mut t0 = (lo - o) / d;
            let mut t1 = (hi - o) / d;
            if t0 > t1 {
                core::mem::swap(&mut t0, &mut t1);
            }
            t_min = t_min.max(t0);
            t_max = t_max.min(t1);
            if t_min > t_max {
                return None;
            }
        }
    }
    Some(t_max - t_min)
}

impl crate::weld::Parts {
    /// Decides a bolt group creation and returns what the adapter authors. Errors leave nothing
    /// applied.
    ///
    /// `spec` is the pinned fastener definition the command names; `bounds` gives the
    /// conservative world box of every part. Every listed part must be crossed by the shank of
    /// at least one fastener (`GEO_OP_NO_EFFECT`), the auto length must find a standard length
    /// covering grip, washers, nut and protrusion (`UNSUPPORTED_GEOMETRY`), and the washer
    /// switches must fit the definition (`INVALID_ARGUMENT`).
    pub fn create_bolt_group(
        &self,
        command: &CreateBoltGroup,
        spec: &FastenerSpec,
        bounds: &BTreeMap<ElementId, Bounds>,
    ) -> Result<BoltGroupDecision, CommandError> {
        command.validate_arguments()?;
        if self.kinds.contains_key(&command.new_id) {
            return Err(error(
                ErrorCode::DuplicateId,
                "new_id",
                "the identity is already in use",
            ));
        }
        for part in command.parts.iter() {
            match self.kind(part) {
                None => {
                    return Err(error(
                        ErrorCode::NotFound,
                        "parts",
                        "a part is not in the head revision",
                    ))
                }
                Some(PartKind::Part) => {}
                Some(PartKind::Assembly) => {
                    return Err(error(
                        ErrorCode::UnsupportedGeometry,
                        "parts",
                        "a bolt group joins plates, beams or columns, not assemblies",
                    ))
                }
                Some(PartKind::Other) => {
                    return Err(error(
                        ErrorCode::InvalidArgument,
                        "parts",
                        "a bolt group joins plates, beams or columns",
                    ))
                }
            }
        }
        let washer_under_head = command.washer_under_head.unwrap_or(spec.washer_under_head);
        let washer_under_nut = command.washer_under_nut.unwrap_or(spec.washer_under_nut);
        if washer_under_head && spec.washer.is_none() {
            return Err(error(
                ErrorCode::InvalidArgument,
                "washer_under_head",
                "the definition carries no washer to place under the head",
            ));
        }
        if washer_under_nut && (spec.washer.is_none() || spec.kind != FastenerKind::Bolt) {
            return Err(error(
                ErrorCode::InvalidArgument,
                "washer_under_nut",
                "a washer under the nut needs a washer and a bolt",
            ));
        }
        if command.hole_mode == HoleMode::Generate {
            // A14.4: generated holes are operation records of the connected parts, written and
            // regenerated with the group. Until that slice lands nothing is written.
            return Err(error(
                ErrorCode::UnsupportedGeometry,
                "hole_mode",
                "generated holes are not supported yet",
            ));
        }
        let hole_clearance_mm = command.hole_clearance_mm.unwrap_or(spec.hole_clearance_mm);
        let (x, y, z) = frame(&command.placement)?;
        // The stack below the shank: washers under the head, then the head.
        let head_offset = spec.head_height_mm
            + if washer_under_head {
                spec.washer.as_ref().map_or(0.0, |w| w.thickness_mm)
            } else {
                0.0
            };
        let stack_over_shank = if spec.kind == FastenerKind::Bolt {
            spec.nut_height_mm.unwrap_or_default()
        } else {
            0.0
        } + if washer_under_nut {
            spec.washer.as_ref().map_or(0.0, |w| w.thickness_mm)
        } else {
            0.0
        };
        let anchors = || -> Result<Vec<[f64; 3]>, CommandError> {
            let [px, py, pz] = command.placement.point1_mm;
            let mut anchors = Vec::with_capacity(command.count());
            let mut walked = 0.0;
            for _ in 0..command.count() {
                let s = command.offset_start_mm + walked;
                let anchor = [
                    px + s * x[0]
                        + command.placement.on_plane_mm * y[0]
                        + command.placement.depth_mm * z[0],
                    py + s * x[1]
                        + command.placement.on_plane_mm * y[1]
                        + command.placement.depth_mm * z[1],
                    pz + s * x[2]
                        + command.placement.on_plane_mm * y[2]
                        + command.placement.depth_mm * z[2],
                ];
                if anchor.iter().any(|v| v.abs() > COORDINATE_CAP_MM) {
                    return Err(error(
                        ErrorCode::LimitExceeded,
                        "spacings_mm",
                        "a fastener position exceeds the coordinate cap",
                    ));
                }
                anchors.push(anchor);
                walked += command.spacings_mm.get(anchors.len() - 1).copied().unwrap_or(0.0);
            }
            Ok(anchors)
        };
        let shanks = |length_mm: f64, anchors: &[[f64; 3]]| -> Vec<([f64; 3], f64)> {
            anchors
                .iter()
                .map(|anchor| {
                    let start = [
                        anchor[0] + head_offset * z[0],
                        anchor[1] + head_offset * z[1],
                        anchor[2] + head_offset * z[2],
                    ];
                    (start, length_mm)
                })
                .collect()
        };
        let resolved = |length_mm: f64| -> Result<BoltGroupDecision, CommandError> {
            let anchors = anchors()?;
            let shanks = shanks(length_mm, &anchors);
            // Every listed part is crossed by at least one shank.
            for part in &command.parts {
                let Some(box_bounds) = bounds.get(part) else {
                    return Err(error(
                        ErrorCode::InvalidArgument,
                        "parts",
                        "a part has no bounds to cross",
                    ));
                };
                let crossed = shanks
                    .iter()
                    .any(|(start, length)| segment_meets_box(start, &z, *length, box_bounds));
                if !crossed {
                    return Err(error(
                        ErrorCode::InvalidArgument,
                        "parts",
                        "the shank never crosses a listed part",
                    ));
                }
            }
            let footprint = Footprint::for_bolt_group(command.new_id, &command.parts, command.fastener)?;
            Ok(BoltGroupDecision {
                length_mm,
                washer_under_head,
                washer_under_nut,
                hole_clearance_mm,
                footprint,
            })
        };
        match command.length_mode {
            BoltLengthMode::Fixed => {
                let length = command.fixed_length_mm.unwrap_or_default();
                if !spec.lengths_mm.contains(&length) {
                    return Err(error(
                        ErrorCode::InvalidArgument,
                        "fixed_length_mm",
                        "the fixed length is not one of the definition's standard lengths",
                    ));
                }
                resolved(length)
            }
            BoltLengthMode::Auto => {
                // The grip of one fastener: the shank length inside the listed parts, summed
                // over the parts it crosses (conservative: the boxes contain the real parts,
                // and overlapping boxes can only overcount).
                let mut grip = 0.0;
                for part in &command.parts {
                    let Some(box_bounds) = bounds.get(part) else {
                        return Err(error(
                            ErrorCode::InvalidArgument,
                            "parts",
                            "a part has no bounds to cross",
                        ));
                    };
                    let mut longest = 0.0;
                    // The largest length of the definition is the longest shank the group could
                    // lay down, so the grip never undercounts.
                    if let Some(longest_standard) = spec.lengths_mm.last() {
                        let anchors = anchors()?;
                        for (start, length) in shanks(*longest_standard, &anchors) {
                            longest = longest.max(
                                segment_inside_length(&start, &z, length, box_bounds).unwrap_or(0.0),
                            );
                        }
                    }
                    grip += longest;
                }
                let needed = grip + stack_over_shank + spec.thread_protrusion_mm;
                let length = spec.lengths_mm.iter().copied().find(|l| *l >= needed).ok_or_else(|| {
                    error(
                        ErrorCode::UnsupportedGeometry,
                        "length_mode",
                        "no standard length is long enough for the grip",
                    )
                })?;
                resolved(length)
            }
        }
    }
}
