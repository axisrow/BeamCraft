// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A05 bolt groups: the engine's decision on `CreateBolt` (domain vocabulary sections 2.3 and
//! 5.1), following the A06 weld pattern.
//!
//! A bolt group is a `connection` element whose one `connects` record names exactly two distinct
//! parts, the main part first. The engine never touches USD: the adapter reads the kinds of the
//! model's elements into the same [`crate::engine::weld::Parts`] index a weld uses, and the decision
//! here is pure,
//! bounded and deterministic. The layer engine re-checks the structural rules on the authored
//! candidate, so a defect in either place fails closed.
//!
//! Rules: the identity is free (`DUPLICATE_ID`); both parts exist (`NOT_FOUND`) and are plates,
//! beams or columns (`DV-REL-003`, `INVALID_ARGUMENT`); they are distinct and not the group
//! itself (`DV-REL-006`, `DV-REL-007`, admitted by the command). An assembly target is refused as
//! unsupported, as for a weld. Beyond the weld rules, every bolt position
//! `origin + direction * (pitch * i)` must lie within the world bounds of both parts: the
//! adapter supplies conservative bounds and this module checks them; it never proves a real face
//! intersection (see the A05 design note).
//!
//! A14.3 extends the same pattern to the placed groups of the bolt amendment: a group is two
//! points, a reference plane with rotation, position values, offsets and spacings over a pinned
//! fastener definition. The placement decision ([`place_bolt_group`]) is pure and bounded; the
//! reachability of a part stays a conservative box test like A05's, while the grip — the shank
//! length that decides the bolt length — is read face-exactly from the parts' committed solids
//! (the world triangles the adapter supplies), exact under any placement rotation (issue 519
//! defect B): a rotated part no longer inflates the requirement past every real library length.
//! A14.4 adds the hole decision ([`generate_holes`]): with `hole_mode = generate` every
//! connected plate takes one ordinary A04 hole record per crossing fastener, owned by the
//! group; a member part refuses until the G14 contract lands, a plate with a rotated placement
//! refuses with it, and `hole_mode = none` writes no part.

use crate::model::command::{
    BoltGroupInputs, BoltLength, BoltSpacing, CreateBolt, CreateBoltGroup, UpdateBoltGroup,
};
use crate::model::fastener::{FastenerKind, FastenerSpec, HoleMode};
use beamcraft_engine::footprint::Footprint;
use beamcraft_geom::tolerance::ABSOLUTE_FLOOR_MM;
use beamcraft_geom::tolerance::DIMENSION_CAP_MM;
use beamcraft_model::command::CommandError;
use beamcraft_model::command::Placement;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::{BTreeMap, BTreeSet};

/// Conservative world bounds of one part: componentwise minimum, then maximum, both inclusive.
pub type Bounds = [[f64; 3]; 2];

/// World-space triangles of one part's committed solid: the face-exact geometry the grip reads
/// (issue 519 defect B). The adapter derives them from the part's stored scene mesh turned by
/// its placement; a part without an entry grips per its conservative box instead.
pub type Faces = Vec<[[f64; 3]; 3]>;

/// The sin of the angle between the plane normal and the row below which the normal counts as
/// parallel to the row (bolt amendment section 4: parallel is `INVALID_ARGUMENT`).
const PARALLEL_FLOOR: f64 = 1e-6;

/// One component of the shank axis below which it counts as parallel to the slab pair.
const SLAB_PARALLEL: f64 = 1e-12;

/// The orthonormal frame of one group: the row `x`, the plane axis `y` and the shank axis `z`.
type Frame = ([f64; 3], [f64; 3], [f64; 3]);

impl crate::engine::weld::Parts {
    /// Decides a bolt group creation and returns its footprint. Errors leave nothing applied.
    ///
    /// `bounds` gives the world box of every part the adapter knows. A supported part without
    /// bounds, or any bolt position outside one of the two boxes, refuses the group as
    /// `INVALID_ARGUMENT`: a bolt must lie on both joined parts.
    pub fn create_bolt(
        &self,
        command: &CreateBolt,
        bounds: &BTreeMap<ElementId, Bounds>,
    ) -> Result<Footprint, CommandError> {
        command.validate_arguments()?;
        if self.kinds.contains_key(&command.new_id) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "new_id",
                "the identity is already in use",
            ));
        }
        for (id, field) in [
            (&command.main_id, "main_id"),
            (&command.secondary_id, "secondary_id"),
        ] {
            match self.kind(id) {
                None => {
                    return Err(CommandError::new(
                        ErrorCode::NotFound,
                        field,
                        "the part is not in the head revision",
                    ));
                }
                Some(crate::engine::weld::PartKind::Part) => {}
                Some(crate::engine::weld::PartKind::Assembly) => {
                    return Err(CommandError::new(
                        ErrorCode::UnsupportedGeometry,
                        field,
                        "a bolt group joins plates, beams or columns, not assemblies",
                    ));
                }
                Some(crate::engine::weld::PartKind::Other) => {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        field,
                        "a bolt group joins plates, beams or columns",
                    ));
                }
            }
        }
        let bolt = &command.bolt;
        let [dx, dy, dz] = bolt.direction_mm;
        let [ox, oy, oz] = bolt.origin_mm;
        for i in 0..bolt.count {
            let t = bolt.pitch_mm * i as f64;
            let [x, y, z] = [ox + dx * t, oy + dy * t, oz + dz * t];
            let on = |part: &ElementId| match bounds.get(part) {
                Some([[xmin, ymin, zmin], [xmax, ymax, zmax]]) => {
                    x >= *xmin && x <= *xmax && y >= *ymin && y <= *ymax && z >= *zmin && z <= *zmax
                }
                None => false,
            };
            if !on(&command.main_id) || !on(&command.secondary_id) {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "origin_mm",
                    "the bolt group does not lie on both parts",
                ));
            }
        }
        Footprint::for_bolt(command.new_id, [command.main_id, command.secondary_id])
    }
}

/// The row geometry of one bolt group (A14.3): the frame, every fastener bearing point and the
/// authoritative spacings — everything the group's inputs alone decide, before the parts decide
/// the reachability and the length. Carried whole by [`BoltGroupPlaced`] and re-derived from the
/// inputs alone by [`preview_row`], which is what a tool preview draws.
#[derive(Debug, Clone, PartialEq)]
pub struct BoltRow {
    /// Unit axis along the row, from `point1` towards `point2`.
    pub x: [f64; 3],
    /// Unit plane axis at zero rotation (`y = z × x`), slid by `on_plane`.
    pub y: [f64; 3],
    /// Unit shank axis at zero rotation, turned by the rotation about `x`.
    pub z: [f64; 3],
    /// Bearing point of every fastener, in group order.
    pub bearings_mm: Vec<[f64; 3]>,
    /// The expanded spacings the group stores; they stay authoritative when a generator made
    /// them (bolt amendment section 4).
    pub spacings_mm: Vec<f64>,
}

/// The resolved placement of one bolt group (A14.3): the frame, every fastener bearing point,
/// the authoritative spacings, the resolved length and the washer switches. The definition's
/// prototype (A14.2) is placed with one rigid transform per fastener: the bearing face at the
/// bearing point and the stack along `z` towards the nut.
#[derive(Debug, Clone, PartialEq)]
pub struct BoltGroupPlaced {
    /// Unit axis along the row, from `point1` towards `point2`.
    pub x: [f64; 3],
    /// Unit plane axis at zero rotation (`y = z × x`), slid by `on_plane`.
    pub y: [f64; 3],
    /// Unit shank axis at zero rotation, turned by the rotation about `x`.
    pub z: [f64; 3],
    /// Bearing point of every fastener, in group order.
    pub bearings_mm: Vec<[f64; 3]>,
    /// The expanded spacings the group stores; they stay authoritative when a generator made
    /// them (bolt amendment section 4).
    pub spacings_mm: Vec<f64>,
    /// The resolved shank length in millimetres, one for the whole group.
    pub length_mm: f64,
    /// The resolved washer switch under the head.
    pub washer_under_head: bool,
    /// The resolved washer switch under the nut.
    pub washer_under_nut: bool,
}

/// Expands the spacing pattern to the authoritative list (bolt amendment section 4): the list
/// as authored, `count` equal pitches, or `count` equal pitches ending `offset_end` before the
/// row's end. An `offset_end` pattern that does not fit between the points at the given offsets
/// is refused like an explicit spacing out of range.
fn expand_spacings(
    spacing: &BoltSpacing,
    chord_mm: f64,
    offset_start_mm: f64,
) -> Result<Vec<f64>, CommandError> {
    match spacing {
        BoltSpacing::Spacings(spacings) => Ok(spacings.clone()),
        BoltSpacing::Pitch { count, pitch_mm } => Ok(vec![*pitch_mm; count.saturating_sub(1)]),
        BoltSpacing::OffsetEnd {
            count,
            offset_end_mm,
        } => {
            let pitch = (chord_mm - offset_start_mm - offset_end_mm) / (*count as f64 - 1.0);
            if !pitch.is_finite() || pitch <= 0.0 || pitch > DIMENSION_CAP_MM {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "offset_end_mm",
                    "the row does not fit between the points at the given offsets",
                ));
            }
            Ok(vec![pitch; count.saturating_sub(1)])
        }
    }
}

/// The committed placement values of an A14.3 group (bolt amendment section 4) that the view
/// snapshot re-derives its fasteners from (issue 512). The layer's structural read has already
/// validated them against the pinned definition; this re-derivation refuses any row it cannot
/// place rather than drawing a guess.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPlacement {
    /// World start of the row in millimetres.
    pub point1_mm: [f64; 3],
    /// World end of the row.
    pub point2_mm: [f64; 3],
    /// Unit normal of the reference plane at zero rotation.
    pub plane_normal_mm: [f64; 3],
    /// Rotation about the row in radians.
    pub rotation_rad: f64,
    /// Offset of the whole group along the plane axis in millimetres.
    pub on_plane_mm: f64,
    /// Offset of the whole group along the shank axis in millimetres.
    pub depth_mm: f64,
    /// Distance from `point1` to the first fastener in millimetres.
    pub offset_start_mm: f64,
    /// The authoritative distances between neighbouring fasteners.
    pub spacings_mm: Vec<f64>,
    /// The resolved shank length in millimetres.
    pub length_mm: f64,
    /// Whether a washer sits under the head.
    pub washer_under_head: bool,
    /// Whether a washer sits under the nut.
    pub washer_under_nut: bool,
}

/// Unit frame of the group (bolt amendment section 4): `x` along the row, `z` the plane
/// normal's component perpendicular to `x` turned by the rotation about `x`, and `y = z × x`.
/// A normal parallel to the row gives no plane and is `INVALID_ARGUMENT`.
fn frame_of(
    point1: [f64; 3],
    point2: [f64; 3],
    normal: [f64; 3],
    rotation_rad: f64,
) -> Result<Frame, CommandError> {
    let chord = [
        point2[0] - point1[0],
        point2[1] - point1[1],
        point2[2] - point1[2],
    ];
    let length = (chord.iter().map(|v| v * v).sum::<f64>()).sqrt();
    if !length.is_finite() || length < ABSOLUTE_FLOOR_MM {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "point2_mm",
            "the two points of the row are distinct",
        ));
    }
    let x = [chord[0] / length, chord[1] / length, chord[2] / length];
    let dot = x[0] * normal[0] + x[1] * normal[1] + x[2] * normal[2];
    let flat = [
        normal[0] - dot * x[0],
        normal[1] - dot * x[1],
        normal[2] - dot * x[2],
    ];
    let norm = (flat.iter().map(|v| v * v).sum::<f64>()).sqrt();
    if !norm.is_finite() || norm < PARALLEL_FLOOR {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "plane_normal_mm",
            "the plane normal must not be parallel to the row",
        ));
    }
    let (sin, cos) = rotation_rad.sin_cos();
    let at = [flat[0] / norm, flat[1] / norm, flat[2] / norm];
    let turn = [
        x[1] * at[2] - x[2] * at[1],
        x[2] * at[0] - x[0] * at[2],
        x[0] * at[1] - x[1] * at[0],
    ];
    let z = [
        at[0] * cos + turn[0] * sin,
        at[1] * cos + turn[1] * sin,
        at[2] * cos + turn[2] * sin,
    ];
    let y = [
        z[1] * x[2] - z[2] * x[1],
        z[2] * x[0] - z[0] * x[2],
        z[0] * x[1] - z[1] * x[0],
    ];
    Ok((x, y, z))
}

/// One parametric interval in which the shank axis from `origin` runs inside `bounds`, clipped
/// to the non-negative side: parts behind the bearing face grip nothing. `None` when the axis
/// misses the box or only touches it.
fn axis_interval(origin: &[f64; 3], axis: &[f64; 3], bounds: &Bounds) -> Option<(f64, f64)> {
    let [low_axis, high_axis] = *bounds;
    let mut enter = 0.0_f64;
    let mut exit = f64::INFINITY;
    for ((d, at), (low, high)) in axis
        .iter()
        .zip(origin.iter())
        .zip(low_axis.iter().zip(high_axis.iter()))
    {
        let (at, low, high, d) = (*at, *low, *high, *d);
        if d.abs() < SLAB_PARALLEL {
            if at < low || at > high {
                return None;
            }
        } else {
            let mut near = (low - at) / d;
            let mut far = (high - at) / d;
            if near > far {
                core::mem::swap(&mut near, &mut far);
            }
            enter = enter.max(near);
            exit = exit.min(far);
        }
    }
    let start = enter.max(0.0);
    if !exit.is_finite() || exit <= start {
        return None;
    }
    Some((start, exit))
}

/// The grip of one fastener: the union length of the given axis intervals, overlaps merged.
fn grip_of(mut intervals: Vec<(f64, f64)>) -> f64 {
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut total = 0.0;
    let mut run: Option<(f64, f64)> = None;
    for (start, end) in intervals {
        match run {
            Some((from, far)) if start <= far => run = Some((from, far.max(end))),
            Some((from, far)) => {
                total += far - from;
                run = Some((start, end));
            }
            None => run = Some((start, end)),
        }
    }
    if let Some((from, far)) = run {
        total += far - from;
    }
    total
}

/// The dot product of two 3-vectors.
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The cross product of two 3-vectors.
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The number of rays on the bearing ring that measure a part's grip next to the axis itself
/// (issue 555 finding 1): eight, 45 degrees apart, so a ring that touches material on any side
/// of a hole or an outline edge is seen.
pub const BEARING_RAYS: usize = 8;

/// The offsets from a bearing point of the grip rays: the axis itself first, then
/// [`BEARING_RAYS`] points on the bearing ring in the group's `x`/`y` plane. The ring radius
/// lies midway between the clearance hole (nominal plus the resolved clearance) and the outer
/// edge of what bears on the part — the washer when one is resolved under the head, the head
/// itself otherwise, mirroring `near_washer` — and never inside the hole: a face too small to
/// reach past the hole falls back to a fifth of the hole's radius beyond it.
fn bearing_ray_offsets(
    inputs: &BoltGroupInputs,
    washer_under_head: bool,
    x: [f64; 3],
    y: [f64; 3],
) -> Vec<[f64; 3]> {
    ring_offsets(
        &inputs.fastener,
        inputs.resolved_hole_clearance_mm(),
        washer_under_head,
        x,
        y,
    )
}

/// The ray offsets of [`bearing_ray_offsets`] from the definition and the resolved clearance.
fn ring_offsets(
    spec: &FastenerSpec,
    clearance_mm: f64,
    washer_under_head: bool,
    x: [f64; 3],
    y: [f64; 3],
) -> Vec<[f64; 3]> {
    let hole = (spec.nominal_diameter_mm + clearance_mm) / 2.0;
    let bearing = match (washer_under_head, &spec.washer) {
        (true, Some(washer)) => washer.outer_diameter_mm,
        _ => spec.head_across_flats_mm,
    } / 2.0;
    let radius = 0.5 * (hole + bearing.max(hole * 1.2));
    let mut offsets = Vec::with_capacity(BEARING_RAYS + 1);
    offsets.push([0.0; 3]);
    for step in 0..BEARING_RAYS {
        let angle = core::f64::consts::TAU * (step as f64) / (BEARING_RAYS as f64);
        let (sin, cos) = angle.sin_cos();
        offsets.push([
            radius * (cos * x[0] + sin * y[0]),
            radius * (cos * x[1] + sin * y[1]),
            radius * (cos * x[2] + sin * y[2]),
        ]);
    }
    offsets
}

/// The material intervals one part contributes to a fastener's grip: of the axis and the
/// bearing-ring rays (all parallel to the shank, all starting in the bearing plane, so their
/// parameters are comparable), the ray that crosses the most material. Empty when no ray finds
/// any: the fastener does not clamp the part.
fn ring_intervals(
    bearing: &[f64; 3],
    axis: &[f64; 3],
    offsets: &[[f64; 3]],
    faces: &Faces,
) -> Vec<(f64, f64)> {
    let mut best: Vec<(f64, f64)> = Vec::new();
    let mut best_grip = 0.0_f64;
    for offset in offsets {
        let origin = [
            bearing[0] + offset[0],
            bearing[1] + offset[1],
            bearing[2] + offset[2],
        ];
        let found = face_intervals(&origin, axis, faces);
        let grip = grip_of(found.clone());
        if grip > best_grip {
            best_grip = grip;
            best = found;
        }
    }
    best
}

/// The material intervals of one part's solid along the shank half-axis from `origin`: every
/// triangle is intersected double-sided (Möller–Trumbore), the forward hits are sorted, hits
/// that land on one boundary within rounding distance are merged (a ray through a shared
/// triangle edge — the ear-clip diagonal of a cap under a centred bolt, for example — hits
/// both triangles a few ulps apart, and pairing them as zero-length intervals would collapse
/// the grip to nothing), and the rest are paired; an odd count — the bearing point sits
/// inside the part — pairs its first crossing with the bearing face itself. The half-axis
/// never grips behind the bearing. A triangle parallel to the axis within the parallel floor
/// is never crossed, and a ray through an opening crosses no material there (the bearing-ring
/// rays of [`place_bolt_group`] see the material around a clearance hole).
fn face_intervals(origin: &[f64; 3], axis: &[f64; 3], faces: &Faces) -> Vec<(f64, f64)> {
    let mut hits = Vec::new();
    for [a, b, c] in faces {
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let plane = cross(*axis, e2);
        let det = dot(e1, plane);
        let edge = (dot(e1, e1) * dot(e2, e2)).sqrt();
        if !det.is_finite() || det.abs() <= PARALLEL_FLOOR * edge {
            continue;
        }
        let s = [origin[0] - a[0], origin[1] - a[1], origin[2] - a[2]];
        let u = dot(s, plane) / det;
        if !(-SLAB_PARALLEL..=1.0 + SLAB_PARALLEL).contains(&u) {
            continue;
        }
        let v = dot(*axis, cross(s, e1)) / det;
        if v < -SLAB_PARALLEL || u + v > 1.0 + SLAB_PARALLEL {
            continue;
        }
        let t = dot(e2, cross(s, e1)) / det;
        if t >= -SLAB_PARALLEL {
            hits.push(if t < 0.0 { 0.0 } else { t });
        }
    }
    hits.sort_by(f64::total_cmp);
    // One boundary, one hit: ulp-level neighbours are the same crossing seen through two
    // triangles. A micron absorbs the rounding at every magnitude the coordinate cap allows,
    // and no real material face stands within a micron of another.
    let mut merged: Vec<f64> = Vec::with_capacity(hits.len());
    for hit in hits {
        let near = merged
            .last()
            .is_some_and(|last| hit - *last <= ABSOLUTE_FLOOR_MM.max(last.abs() * 1e-9));
        if !near {
            merged.push(hit);
        }
    }
    let mut intervals = Vec::new();
    let mut index = 0usize;
    if merged.len() % 2 == 1 {
        if let Some(first) = merged.first() {
            intervals.push((0.0, *first));
        }
        index = 1;
    }
    while index + 1 < merged.len() {
        if let (Some(start), Some(end)) = (merged.get(index), merged.get(index + 1)) {
            intervals.push((*start, *end));
        }
        index += 2;
    }
    intervals
}

/// The frame of a group and the bearing point of every fastener: the shared placement
/// arithmetic of [`place_bolt_group`] and of a committed group's stored values
/// ([`BoltGroupPlaced::from_stored`]). The bearings start at `offset_start` along the row from
/// the slid first point and step one spacing at a time.
#[allow(clippy::too_many_arguments)]
fn frame_and_bearings(
    point1_mm: [f64; 3],
    point2_mm: [f64; 3],
    plane_normal_mm: [f64; 3],
    rotation_rad: f64,
    on_plane_mm: f64,
    depth_mm: f64,
    offset_start_mm: f64,
    spacings_mm: &[f64],
) -> Result<(Frame, Vec<[f64; 3]>), CommandError> {
    let (x, y, z) = frame_of(point1_mm, point2_mm, plane_normal_mm, rotation_rad)?;
    let first = [
        point1_mm[0] + on_plane_mm * y[0] + depth_mm * z[0],
        point1_mm[1] + on_plane_mm * y[1] + depth_mm * z[1],
        point1_mm[2] + on_plane_mm * y[2] + depth_mm * z[2],
    ];
    let mut bearings = Vec::with_capacity(spacings_mm.len() + 1);
    let mut along = offset_start_mm;
    for _ in 0..=spacings_mm.len() {
        bearings.push([
            first[0] + along * x[0],
            first[1] + along * x[1],
            first[2] + along * x[2],
        ]);
        let Some(next) = spacings_mm.get(bearings.len() - 1) else {
            break;
        };
        along += *next;
    }
    Ok(((x, y, z), bearings))
}

/// Places the group (bolt amendment section 4): the frame, the authoritative spacings, the
/// bearing points and the one length for the whole group. The crossing admission stays the
/// conservative box decision of A05: every listed part must be crossed by the axis of at
/// least one fastener. The grip is the clamped pack between the bearing faces (issue 555
/// finding 1): a fastener always stands in a hole, and the head and the nut bear on the ring
/// of material around it, not on the axis line. So the grip of a part is measured along the
/// axis *and* along [`BEARING_RAYS`] rays parallel to it on the bearing ring (midway between
/// the clearance hole's radius and the washer's — or, without a washer, the head's — outer
/// radius), each through the actual faces of the part (the axis crossings of its world
/// triangles, paired into material intervals — exact under any placement rotation, issue 519
/// defect B); the part contributes the intervals of its ray with the most material. A coaxial
/// clearance hole therefore grips the slab around it whoever cut it — the group itself, another
/// group, `add-hole` or `create-plate --hole-round` — and the length no longer depends on the
/// order of the operations. A part without an entry in `faces` grips per its conservative box,
/// which errs long and never under. A listed part whose box the axes cross but whose bearing
/// ring no fastener ever finds material in (every crossing falls into a larger opening or past
/// the part's outline) is refused: the group would not clamp it. `auto` picks the smallest
/// standard length that covers the largest grip plus the head height, the washers the shank
/// passes through (under the head and under the nut, when placed), the nut and the thread
/// protrusion, so every fastener of the group is identical. The bearing plane is the joint
/// face: the head assembly stacks outside the joint and the nut stack hangs downward from the
/// thread tip, so the covering length seats the nut washer exactly on the far plate (issue
/// 555 audit item 9, the review on the 1111 batch; EN 1993-1-8 3.6 / EN 15048-1 clamp
/// length). A fixed length shorter than that same requirement is refused.
pub fn place_bolt_group(
    inputs: &BoltGroupInputs,
    bounds: &BTreeMap<ElementId, Bounds>,
    faces: &BTreeMap<ElementId, Faces>,
) -> Result<BoltGroupPlaced, CommandError> {
    inputs.validate_arguments()?;
    let spec = &inputs.fastener;
    let washer_under_head = inputs.washer_under_head.unwrap_or(spec.washer_under_head);
    let washer_under_nut = inputs.washer_under_nut.unwrap_or(spec.washer_under_nut);
    if washer_under_head && spec.washer.is_none() {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "washer_under_head",
            "the definition carries no washer to place under the head",
        ));
    }
    if washer_under_nut && (spec.washer.is_none() || spec.kind != FastenerKind::Bolt) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "washer_under_nut",
            "a washer under the nut needs a washer and a bolt",
        ));
    }
    let chord = chord_of(inputs.point1_mm, inputs.point2_mm);
    let spacings = expand_spacings(&inputs.spacing, chord, inputs.offset_start_mm)?;
    let ((x, y, z), bearings) = frame_and_bearings(
        inputs.point1_mm,
        inputs.point2_mm,
        inputs.plane_normal_mm,
        inputs.rotation_rad,
        inputs.on_plane_mm,
        inputs.depth_mm,
        inputs.offset_start_mm,
        &spacings,
    )?;
    // The grip decides the one length of the group; the crossing stays the box admission, the
    // length itself follows the faces on the bearing ring.
    let rays = bearing_ray_offsets(inputs, washer_under_head, x, y);
    let mut crossed = vec![false; inputs.parts.len()];
    let mut clamped = vec![false; inputs.parts.len()];
    let mut max_grip = 0.0_f64;
    for bearing in &bearings {
        let mut intervals = Vec::new();
        for (index, part) in inputs.parts.iter().enumerate() {
            let Some(admitted) = bounds.get(part).and_then(|b| axis_interval(bearing, &z, b))
            else {
                continue;
            };
            if let Some(done) = crossed.get_mut(index) {
                *done = true;
            }
            let found = match faces.get(part) {
                Some(part_faces) => ring_intervals(bearing, &z, &rays, part_faces),
                None => vec![admitted],
            };
            if !found.is_empty()
                && let Some(done) = clamped.get_mut(index)
            {
                *done = true;
            }
            intervals.extend(found);
        }
        max_grip = max_grip.max(grip_of(intervals));
    }
    if crossed.iter().any(|done| !done) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "parts",
            "a listed part is never crossed by the axis of a fastener",
        ));
    }
    if clamped.iter().any(|done| !done) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "parts",
            "a listed part is never clamped: every fastener crossing it falls into an opening \
             or past its outline, so no bearing ring finds its material",
        ));
    }
    let washer_thickness = spec.washer.map(|w| w.thickness_mm).unwrap_or(0.0);
    let near_washer = if washer_under_head {
        washer_thickness
    } else {
        0.0
    };
    let far_washer = if washer_under_nut {
        washer_thickness
    } else {
        0.0
    };
    let nut_height = spec.nut_height_mm.unwrap_or(0.0);
    // The prototype stacks the head assembly on the outside of the joint, below the bearing
    // plane, and the shank starts inside the head (issue 562 item 5): the shank's course
    // through the joint loses the head height before the grip begins, so the length must
    // cover it on top of the grip, the nut washer, the nut and the protrusion — otherwise
    // the nut lands short of the far plate and floats on bare shank.
    let required = max_grip
        + near_washer
        + far_washer
        + nut_height
        + spec.head_height_mm
        + spec.thread_protrusion_mm;
    let length = match inputs.length {
        BoltLength::Auto => spec
            .lengths_mm
            .iter()
            .find(|l| **l >= required)
            .copied()
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "length",
                    "no standard length covers the grip, the nut and the protrusion",
                )
            })?,
        BoltLength::Fixed(length) if length >= required => length,
        BoltLength::Fixed(_) => {
            return Err(CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "length_mm",
                "the fixed length does not cover the grip, the nut and the protrusion",
            ));
        }
    };
    Ok(BoltGroupPlaced {
        x,
        y,
        z,
        bearings_mm: bearings,
        spacings_mm: spacings,
        length_mm: length,
        washer_under_head,
        washer_under_nut,
    })
}

/// The distance between the row's two points, in millimetres: the shared chord of the spacing
/// patterns that fit the row between its ends.
fn chord_of(point1_mm: [f64; 3], point2_mm: [f64; 3]) -> f64 {
    (point2_mm[0] - point1_mm[0])
        .hypot((point2_mm[1] - point1_mm[1]).hypot(point2_mm[2] - point1_mm[2]))
}

/// The far face of the joint each fastener of a committed group clamps, along its shank axis
/// in millimetres from the bearing plane, in bearing order: the furthest material end of the
/// listed parts the placement's bearing-ring rays find (the rays [`place_bolt_group`] measures
/// the grip with). `None` for a fastener whose rays cross none of the parts. The scene seats
/// the nut stack on this face, so a standard length longer than the joint needs protrudes past
/// the nut instead of floating the nut washer off the far plate (issue 562 item 5). The query
/// is pure and bounded like the placement itself.
pub fn far_faces(
    placed: &BoltGroupPlaced,
    spec: &FastenerSpec,
    clearance_mm: f64,
    parts: &[ElementId],
    bounds: &BTreeMap<ElementId, Bounds>,
    faces: &BTreeMap<ElementId, Faces>,
) -> Vec<Option<f64>> {
    let rays = ring_offsets(
        spec,
        clearance_mm,
        placed.washer_under_head,
        placed.x,
        placed.y,
    );
    placed
        .bearings_mm
        .iter()
        .map(|bearing| {
            parts
                .iter()
                .filter_map(|part| {
                    let admitted = bounds
                        .get(part)
                        .and_then(|b| axis_interval(bearing, &placed.z, b))?;
                    let found = match faces.get(part) {
                        Some(part_faces) => ring_intervals(bearing, &placed.z, &rays, part_faces),
                        None => vec![admitted],
                    };
                    found.iter().map(|(_, end)| *end).reduce(f64::max)
                })
                .reduce(f64::max)
        })
        .collect()
}

impl BoltGroupPlaced {
    /// The row this placement places: everything but the length the parts' grip decides.
    pub fn row(&self) -> BoltRow {
        BoltRow {
            x: self.x,
            y: self.y,
            z: self.z,
            bearings_mm: self.bearings_mm.clone(),
            spacings_mm: self.spacings_mm.clone(),
        }
    }

    /// The placement a committed group's stored values re-derive: the same frame and bearing
    /// points [`place_bolt_group`] decided at creation, without the part reachability and
    /// length decision the creation ran (the committed record is authoritative). A row whose
    /// stored values give no frame — a degenerate row or a normal parallel to it — is refused,
    /// never guessed.
    pub fn from_stored(stored: &StoredPlacement) -> Result<Self, CommandError> {
        let ((x, y, z), bearings) = frame_and_bearings(
            stored.point1_mm,
            stored.point2_mm,
            stored.plane_normal_mm,
            stored.rotation_rad,
            stored.on_plane_mm,
            stored.depth_mm,
            stored.offset_start_mm,
            &stored.spacings_mm,
        )?;
        Ok(Self {
            x,
            y,
            z,
            bearings_mm: bearings,
            spacings_mm: stored.spacings_mm.clone(),
            length_mm: stored.length_mm,
            washer_under_head: stored.washer_under_head,
            washer_under_nut: stored.washer_under_nut,
        })
    }
}

/// The preview row of the inputs (bolt amendment section 4): the frame, the bearing points and
/// the expanded spacings [`place_bolt_group`] would decide, without the parts' reachability and
/// length decision their geometry decides. A tool preview draws exactly these bearings and the
/// holes [`generate_holes`] cuts at them; whether the parts admit the row stays the commit's
/// decision, so what the preview draws is still the command's ask (the member ghost's rule).
/// The decision is pure, bounded and deterministic, like every engine query.
pub fn preview_row(inputs: &BoltGroupInputs) -> Result<BoltRow, CommandError> {
    inputs.validate_arguments()?;
    let spacings = expand_spacings(
        &inputs.spacing,
        chord_of(inputs.point1_mm, inputs.point2_mm),
        inputs.offset_start_mm,
    )?;
    let ((x, y, z), bearings) = frame_and_bearings(
        inputs.point1_mm,
        inputs.point2_mm,
        inputs.plane_normal_mm,
        inputs.rotation_rad,
        inputs.on_plane_mm,
        inputs.depth_mm,
        inputs.offset_start_mm,
        &spacings,
    )?;
    Ok(BoltRow {
        x,
        y,
        z,
        bearings_mm: bearings,
        spacings_mm: spacings,
    })
}

/// One generated hole decision (bolt amendment section 5): the plate that owns the record and
/// its authoritative clockwise polygon in the plate's local XY. The record itself is an
/// ordinary A04 hole record; the adapter appends it with a fresh ordinal and replays the
/// plate, so a hole that does not fit refuses the whole command.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedHole {
    /// The plate that will own the record.
    pub part: ElementId,
    /// The round hole polygon in the plate's local XY.
    pub polygon_mm: Vec<[f64; 2]>,
}

/// A world vector in the element-local frame of `placement`: the conjugate (inverse) rotation,
/// normalized first like every placement rotation, so any admitted quaternion is exact.
pub fn to_local_vector(placement: &Placement, world: [f64; 3]) -> [f64; 3] {
    let [x, y, z, w] = placement.orientation_xyzw;
    beamcraft_engine::solid::rotate_vector(world, [-x, -y, -z, w])
}

/// A world point in the element-local frame of `placement`: the translation removed, then the
/// inverse rotation of [`to_local_vector`].
pub fn to_local_point(placement: &Placement, world: [f64; 3]) -> [f64; 3] {
    let t = placement.translation_mm;
    to_local_vector(
        placement,
        [world[0] - t[0], world[1] - t[1], world[2] - t[2]],
    )
}

/// The smallest admitted cosine between the shank axis and a plate's normal for a generated
/// hole: the record is a through hole cut along the plate normal, so an axis more than 60
/// degrees off it would leave the shank outside its own hole for most of the plate.
pub const MIN_HOLE_AXIS_COSINE: f64 = 0.5;

/// Decides the generated holes of a placed group (bolt amendment section 5): with
/// `hole_mode = generate`, one round through hole per crossing fastener on every connected
/// plate, of diameter `nominal + clearance` (the group's own clearance, or the definition's),
/// centred where the shank axis crosses the plate's mid-plane and cut along the plate normal,
/// like any A04 record. The decision runs in the plate's own frame (issue 555 finding 2): the
/// bearing point and the shank direction are carried into element-local coordinates by the
/// inverse of the plate's placement (any rotation), the crossing is decided against the
/// plate-local box `slabs` gives (the contour's extents and the material band of its
/// `thickness_side`, [`crate::engine::command::AuthoredPlate::local_box_mm`]), and the centre
/// is the local point of the axis on the band's mid-plane. A shank axis more than 60 degrees
/// off the plate normal ([`MIN_HOLE_AXIS_COSINE`]) is refused with `UNSUPPORTED_GEOMETRY`. A
/// beam or a column is refused with it too: generated member holes need the kernel slice
/// G14.1. `hole_mode = none` writes no part. The decision is pure: the same inputs give the
/// same holes.
pub fn generate_holes(
    inputs: &BoltGroupInputs,
    row: &BoltRow,
    slabs: &BTreeMap<ElementId, Bounds>,
    placements: &BTreeMap<ElementId, Placement>,
    members: &BTreeSet<ElementId>,
) -> Result<Vec<GeneratedHole>, CommandError> {
    if inputs.resolved_hole_mode() == HoleMode::None {
        return Ok(Vec::new());
    }
    let diameter = inputs.fastener.nominal_diameter_mm + inputs.resolved_hole_clearance_mm();
    let mut holes = Vec::new();
    for part in &inputs.parts {
        if members.contains(part) {
            return Err(CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "hole_mode",
                "a generated hole in a member needs the kernel slice G14.1",
            ));
        }
        let slab = slabs.get(part).ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidArgument,
                "parts",
                "a listed part is unreachable",
            )
        })?;
        let placement = placements.get(part).ok_or_else(|| {
            CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "hole_mode",
                "the part has no placement this slice can read",
            )
        })?;
        // The shank direction in the plate's frame: the inverse rotation alone (no translation).
        let axis = to_local_vector(placement, row.z);
        let mid_z = (slab[0][2] + slab[1][2]) / 2.0;
        for bearing in &row.bearings_mm {
            let local = to_local_point(placement, *bearing);
            if axis_interval(&local, &axis, slab).is_none() {
                continue;
            }
            if axis[2].abs() < MIN_HOLE_AXIS_COSINE {
                return Err(CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "hole_mode",
                    "the shank axis is more than 60 degrees off the plate normal",
                ));
            }
            // The plate-local XY centre: where the shank axis crosses the band's mid-plane.
            let along = (mid_z - local[2]) / axis[2];
            let centre = [local[0] + axis[0] * along, local[1] + axis[1] * along];
            holes.push(GeneratedHole {
                part: *part,
                polygon_mm: crate::model::command::HoleSpec::Round {
                    centre_mm: centre,
                    diameter_mm: diameter,
                }
                .polygon()?,
            });
        }
    }
    Ok(holes)
}

/// Kinds of the listed parts: each exists and is a plate, beam or column (an assembly is
/// unsupported, as for a weld). Their existence decides acceptance; they are only read.
fn check_parts(
    parts: &[ElementId],
    kinds: &crate::engine::weld::Parts,
) -> Result<(), CommandError> {
    for part in parts {
        match kinds.kind(part) {
            None => {
                return Err(CommandError::new(
                    ErrorCode::NotFound,
                    "parts",
                    "a part is not in the head revision",
                ));
            }
            Some(crate::engine::weld::PartKind::Part) => {}
            Some(crate::engine::weld::PartKind::Assembly) => {
                return Err(CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "parts",
                    "a bolt group joins plates, beams or columns, not assemblies",
                ));
            }
            Some(crate::engine::weld::PartKind::Other) => {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "parts",
                    "a bolt group joins plates, beams or columns",
                ));
            }
        }
    }
    Ok(())
}

impl crate::engine::weld::Parts {
    /// Decides an A14.3 bolt group creation and returns its placement and footprint. Errors
    /// leave nothing applied. The reachability and the length follow
    /// [`place_bolt_group`]; the parts and the pinned definition are only read.
    pub fn create_bolt_group(
        &self,
        command: &CreateBoltGroup,
        bounds: &BTreeMap<ElementId, Bounds>,
        faces: &BTreeMap<ElementId, Faces>,
    ) -> Result<(BoltGroupPlaced, Footprint), CommandError> {
        command.validate_arguments()?;
        if self.kinds.contains_key(&command.new_id) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "new_id",
                "the identity is already in use",
            ));
        }
        check_parts(&command.inputs.parts, self)?;
        let placed = place_bolt_group(&command.inputs, bounds, faces)?;
        let footprint = Footprint::for_bolt_group(
            command.new_id,
            &command.inputs.parts,
            command.inputs.fastener_digest,
        )?;
        Ok((placed, footprint))
    }

    /// Decides an A14.3 bolt group edit and returns the new placement and footprint: one atomic
    /// edit and one undo unit, the complete inputs replacing the stored ones. The target must
    /// exist; whether it is a group this slice reads is the adapter's decoded record.
    pub fn update_bolt_group(
        &self,
        command: &UpdateBoltGroup,
        bounds: &BTreeMap<ElementId, Bounds>,
        faces: &BTreeMap<ElementId, Faces>,
    ) -> Result<(BoltGroupPlaced, Footprint), CommandError> {
        command.validate_arguments()?;
        if !self.kinds.contains_key(&command.target_id) {
            return Err(CommandError::new(
                ErrorCode::NotFound,
                "target_id",
                "the group is not in the head revision",
            ));
        }
        check_parts(&command.inputs.parts, self)?;
        let placed = place_bolt_group(&command.inputs, bounds, faces)?;
        let footprint = Footprint::for_bolt_group_edit(
            command.target_id,
            &command.inputs.parts,
            command.inputs.fastener_digest,
            command.name.is_some(),
        )?;
        Ok((placed, footprint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::weld::PartKind;
    use crate::model::command::BoltSpec;

    fn id(n: u32) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000005{n:05}")).unwrap()
    }

    fn spec() -> BoltSpec {
        BoltSpec {
            diameter_mm: 16.0,
            count: 3,
            pitch_mm: 80.0,
            // On the seam of both bounds: the plate is 0..500 in y, the beam +-60, so y = 0.
            origin_mm: [100.0, 0.0, 5.0],
            direction_mm: [1.0, 0.0, 0.0],
        }
    }

    fn command() -> CreateBolt {
        CreateBolt {
            new_id: id(9),
            name: "B".into(),
            main_id: id(1),
            secondary_id: id(2),
            bolt: spec(),
        }
    }

    fn parts() -> crate::engine::weld::Parts {
        let mut parts = crate::engine::weld::Parts::default();
        parts.insert(id(1), PartKind::Part);
        parts.insert(id(2), PartKind::Part);
        parts.insert(id(3), PartKind::Assembly);
        parts.insert(id(4), PartKind::Other);
        parts
    }

    fn bounds(x: f64) -> BTreeMap<ElementId, Bounds> {
        // A plate 0..1000 in x, 0..500 in y, 0..10 in z; a member whose conservative box is the
        // axis line 0..3000 in x expanded to +-60 in y and z.
        let mut map = BTreeMap::new();
        map.insert(id(1), [[0.0, 0.0, 0.0], [x, 500.0, 10.0]]);
        map.insert(id(2), [[-60.0, -60.0, -60.0], [3060.0, 60.0, 60.0]]);
        map
    }

    /// The twelve triangles of one axis-aligned box, outward or inward — the engine pairs the
    /// crossings either way.
    fn box_faces(min: [f64; 3], max: [f64; 3]) -> Faces {
        let corner = |at: [usize; 3]| {
            [
                if at[0] == 0 { min[0] } else { max[0] },
                if at[1] == 0 { min[1] } else { max[1] },
                if at[2] == 0 { min[2] } else { max[2] },
            ]
        };
        let mut faces = Faces::new();
        let mut quad = |a, b, c, d: [usize; 3]| {
            faces.push([corner(a), corner(b), corner(c)]);
            faces.push([corner(a), corner(c), corner(d)]);
        };
        quad([0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]);
        quad([0, 0, 1], [0, 1, 1], [1, 1, 1], [1, 0, 1]);
        quad([0, 0, 0], [0, 0, 1], [1, 0, 1], [1, 0, 0]);
        quad([0, 1, 0], [1, 1, 0], [1, 1, 1], [0, 1, 1]);
        quad([0, 0, 0], [0, 1, 0], [0, 1, 1], [0, 0, 1]);
        quad([1, 0, 0], [1, 0, 1], [1, 1, 1], [1, 1, 0]);
        faces
    }

    /// The exact box faces of every bounds entry: for the axis-aligned prisms of these tests
    /// the box is the true geometry, so the face grip equals the box grip.
    fn box_faces_of(bounds: &BTreeMap<ElementId, Bounds>) -> BTreeMap<ElementId, Faces> {
        bounds
            .iter()
            .map(|(part, slab)| (*part, box_faces(slab[0], slab[1])))
            .collect()
    }

    /// [`place_bolt_group`] over the exact box faces of the given bounds and no own holes: the
    /// default of the axis-aligned tests here. The knee-brace and hole tests supply their own.
    fn place(
        inputs: &BoltGroupInputs,
        bounds: &BTreeMap<ElementId, Bounds>,
    ) -> Result<BoltGroupPlaced, CommandError> {
        place_bolt_group(inputs, bounds, &box_faces_of(bounds))
    }

    #[test]
    fn a_group_reads_both_parts_and_writes_only_itself() {
        let footprint = parts().create_bolt(&command(), &bounds(1000.0)).unwrap();
        for part in [id(1), id(2)] {
            let lifecycle = beamcraft_engine::footprint::Entry {
                target: beamcraft_engine::footprint::Target::Element(part),
                group: beamcraft_engine::footprint::Group::Lifecycle,
            };
            assert!(footprint.reads().contains(&lifecycle));
        }
        assert_eq!(footprint.writes().len(), 7);
        assert!(
            footprint
                .writes()
                .iter()
                .all(|e| e.target == beamcraft_engine::footprint::Target::Element(id(9)))
        );
    }

    #[test]
    fn missing_wrong_kind_and_taken_identities_are_refused() {
        let parts = parts();
        let code = |c: CreateBolt, b: &BTreeMap<ElementId, Bounds>| {
            parts.create_bolt(&c, b).unwrap_err().code
        };
        let unknown = CreateBolt {
            secondary_id: id(7),
            ..command()
        };
        assert_eq!(code(unknown, &bounds(1000.0)), ErrorCode::NotFound);
        let assembly = CreateBolt {
            secondary_id: id(3),
            ..command()
        };
        assert_eq!(
            code(assembly, &bounds(1000.0)),
            ErrorCode::UnsupportedGeometry
        );
        let other = CreateBolt {
            secondary_id: id(4),
            ..command()
        };
        assert_eq!(code(other, &bounds(1000.0)), ErrorCode::InvalidArgument);
        let taken = CreateBolt {
            new_id: id(4),
            ..command()
        };
        assert_eq!(code(taken, &bounds(1000.0)), ErrorCode::DuplicateId);
    }

    #[test]
    fn every_bolt_position_must_lie_on_both_parts() {
        let parts = parts();
        let code = |c: CreateBolt, b: &BTreeMap<ElementId, Bounds>| {
            parts.create_bolt(&c, b).unwrap_err().code
        };
        // Three bolts at x = 100, 180, 260 fit the 1000 mm plate; a 13th bolt at x = 1060
        // leaves it, so the whole group is refused.
        assert!(parts.create_bolt(&command(), &bounds(1000.0)).is_ok());
        let short = CreateBolt {
            bolt: BoltSpec {
                count: 13,
                pitch_mm: 80.0,
                ..spec()
            },
            ..command()
        };
        assert_eq!(code(short, &bounds(1000.0)), ErrorCode::InvalidArgument);
        let out_of_plane = CreateBolt {
            bolt: BoltSpec {
                origin_mm: [100.0, 0.0, 70.0],
                ..spec()
            },
            ..command()
        };
        assert_eq!(
            code(out_of_plane, &bounds(1000.0)),
            ErrorCode::InvalidArgument
        );
        // The first bolt is checked like any other: an origin outside is refused with count 1.
        let single = CreateBolt {
            bolt: BoltSpec {
                count: 1,
                origin_mm: [2000.0, 250.0, 5.0],
                ..spec()
            },
            ..command()
        };
        assert_eq!(code(single, &bounds(1000.0)), ErrorCode::InvalidArgument);
        // A supported part without bounds is unreachable, never guessed.
        let empty = BTreeMap::new();
        assert_eq!(code(command(), &empty), ErrorCode::InvalidArgument);
    }

    /// A washered M16 bolt definition with tight lengths, so the auto boundary is exact.
    fn group_inputs() -> BoltGroupInputs {
        let fastener = crate::model::fastener::FastenerSpec {
            designation: Some("M16".into()),
            standard: None,
            grade: None,
            display_name: None,
            kind: FastenerKind::Bolt,
            head_kind: crate::model::fastener::HeadKind::Hex,
            nominal_diameter_mm: 16.0,
            head_across_flats_mm: 24.0,
            head_height_mm: 10.0,
            nut_across_flats_mm: Some(24.0),
            nut_height_mm: Some(14.8),
            washer: Some(crate::model::fastener::WasherSpec {
                outer_diameter_mm: 30.0,
                inner_diameter_mm: 17.0,
                thickness_mm: 3.0,
            }),
            hole_clearance_mm: 2.0,
            lengths_mm: vec![30.0, 35.0, 40.0, 45.0, 50.0, 60.0, 80.0],
            thread_protrusion_mm: 5.0,
            washer_under_head: true,
            washer_under_nut: true,
            hole_mode: crate::model::fastener::HoleMode::Generate,
            mass_each_kg: None,
        };
        fastener.validate().unwrap();
        BoltGroupInputs {
            parts: vec![id(1), id(2)],
            fastener_digest: beamcraft_model::digest::ContentDigest::from_bytes([3u8; 32]),
            fastener,
            point1_mm: [0.0, 0.0, 0.0],
            point2_mm: [400.0, 0.0, 0.0],
            plane_normal_mm: [0.0, 0.0, 1.0],
            rotation_rad: 0.0,
            on_plane_mm: 0.0,
            depth_mm: 0.0,
            offset_start_mm: 50.0,
            spacing: BoltSpacing::Spacings(vec![80.0, 90.0]),
            washer_under_head: None,
            washer_under_nut: None,
            length: BoltLength::Auto,
            hole_mode: None,
            hole_clearance_mm: None,
        }
    }

    /// Two identical overlapping plates, each 0..x along the row, 0..500 across and 0..10
    /// thick: every shank crosses both for 10 mm, so the merged grip is 10, not 20.
    fn group_bounds(x: f64) -> BTreeMap<ElementId, Bounds> {
        let mut map = BTreeMap::new();
        for part in [id(1), id(2)] {
            map.insert(part, [[0.0, 0.0, 0.0], [x, 500.0, 10.0]]);
        }
        map
    }

    #[test]
    fn the_frame_follows_points_normal_and_rotation() {
        // A box that every frame of the test crosses, and one long standard length, so the
        // placement resolves whatever the frame is.
        let wide = |map: &mut BTreeMap<ElementId, Bounds>| {
            for part in [id(1), id(2)] {
                map.insert(part, [[-1000.0; 3], [1000.0; 3]]);
            }
        };
        let mut frame = group_inputs();
        frame.fastener.lengths_mm = vec![2000.0];
        let mut bounds = BTreeMap::new();
        wide(&mut bounds);
        let placed = place(&frame, &bounds).unwrap();
        assert_eq!(placed.x, [1.0, 0.0, 0.0]);
        assert_eq!(placed.y, [0.0, 1.0, 0.0]);
        assert_eq!(placed.z, [0.0, 0.0, 1.0]);
        // Rotation of a quarter turn turns the plane axis up and the shank axis to -y.
        let turned = place(
            &BoltGroupInputs {
                rotation_rad: core::f64::consts::FRAC_PI_2,
                ..frame.clone()
            },
            &bounds,
        )
        .unwrap();
        // Trig carries a few ulps; the unit frame is compared within a micron.
        let near = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(near(turned.z, [0.0, -1.0, 0.0]));
        assert!(near(turned.y, [0.0, 0.0, 1.0]));
        // on_plane slides across the plane, depth along the shank; both move every bearing.
        let moved = place(
            &BoltGroupInputs {
                on_plane_mm: 12.0,
                depth_mm: -4.0,
                ..frame
            },
            &bounds,
        )
        .unwrap();
        assert_eq!(moved.bearings_mm[0], [50.0, 12.0, -4.0]);
        // The row runs along x from the offset start, then each spacing.
        assert_eq!(
            placed.bearings_mm,
            vec![[50.0, 0.0, 0.0], [130.0, 0.0, 0.0], [220.0, 0.0, 0.0]]
        );
    }

    #[test]
    fn generators_expand_and_offset_end_follows_the_points() {
        let pitch = place(
            &BoltGroupInputs {
                spacing: BoltSpacing::Pitch {
                    count: 3,
                    pitch_mm: 80.0,
                },
                ..group_inputs()
            },
            &group_bounds(1000.0),
        )
        .unwrap();
        assert_eq!(pitch.spacings_mm, vec![80.0, 80.0]);
        let offset_end = place(
            &BoltGroupInputs {
                spacing: BoltSpacing::OffsetEnd {
                    count: 3,
                    offset_end_mm: 50.0,
                },
                ..group_inputs()
            },
            &group_bounds(1000.0),
        )
        .unwrap();
        // 400 - 50 (start) - 50 (end) over two gaps: the last fastener lies at 350.
        assert_eq!(offset_end.spacings_mm, vec![150.0, 150.0]);
        assert_eq!(offset_end.bearings_mm[2], [350.0, 0.0, 0.0]);
        let moved = place(
            &BoltGroupInputs {
                point2_mm: [600.0, 0.0, 0.0],
                spacing: BoltSpacing::OffsetEnd {
                    count: 3,
                    offset_end_mm: 50.0,
                },
                ..group_inputs()
            },
            &group_bounds(1000.0),
        )
        .unwrap();
        assert_eq!(moved.spacings_mm, vec![250.0, 250.0]);
        // A row that does not fit is refused like an explicit spacing.
        let tight = place(
            &BoltGroupInputs {
                offset_start_mm: 500.0,
                spacing: BoltSpacing::OffsetEnd {
                    count: 3,
                    offset_end_mm: 50.0,
                },
                ..group_inputs()
            },
            &group_bounds(1000.0),
        );
        assert_eq!(tight.unwrap_err().field, "offset_end_mm");
    }

    #[test]
    fn auto_picks_the_smallest_length_that_covers_grip_washer_nut_and_protrusion() {
        // Grip 10 (the two overlapping plates count once) + under-head washer 3 + under-nut
        // washer 3 + nut 14.8 + head 10 (the head assembly lives outside the joint, and the
        // shank's course through it comes off the length) + protrusion 5 = 45.8: the
        // smallest listed length is 50. A wrongly summed grip of 20 would pick 60 (issue
        // 555 audit item 9: the bolt length runs from inside the head, so the under-head
        // washer lies on the shank and counts).
        let placed = place(&group_inputs(), &group_bounds(1000.0)).unwrap();
        assert_eq!(placed.length_mm, 50.0);
        assert!(placed.washer_under_head && placed.washer_under_nut);
        // Without the far washer the requirement is 42.8, so 45 covers it.
        let bare = place(
            &BoltGroupInputs {
                washer_under_nut: Some(false),
                ..group_inputs()
            },
            &group_bounds(1000.0),
        )
        .unwrap();
        assert_eq!(bare.length_mm, 45.0);
        // Without either washer the requirement is 39.8, so 40 covers it.
        let none = place(
            &BoltGroupInputs {
                washer_under_head: Some(false),
                washer_under_nut: Some(false),
                ..group_inputs()
            },
            &group_bounds(1000.0),
        )
        .unwrap();
        assert_eq!(none.length_mm, 40.0);
    }

    #[test]
    fn crossing_and_length_refusals_change_nothing() {
        // A part off the axis is never crossed.
        let off = place(
            &BoltGroupInputs {
                parts: vec![id(1), id(3)],
                ..group_inputs()
            },
            &group_bounds(1000.0),
        );
        assert_eq!(off.unwrap_err().field, "parts");
        // No listed length covers the joint.
        let mut short = group_inputs();
        short.fastener.lengths_mm = vec![30.0];
        assert_eq!(
            place(&short, &group_bounds(1000.0)).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
        // A fixed length below the requirement is refused; an adequate one is kept.
        let mut fixed = group_inputs();
        fixed.length = BoltLength::Fixed(45.0);
        assert_eq!(
            place(&fixed, &group_bounds(1000.0)).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
        fixed.length = BoltLength::Fixed(50.0);
        assert_eq!(
            place(&fixed, &group_bounds(1000.0)).unwrap().length_mm,
            50.0
        );
        // Degenerate rows and a normal parallel to the row are refused.
        let same = place(
            &BoltGroupInputs {
                point2_mm: [0.0, 0.0, 0.0],
                ..group_inputs()
            },
            &group_bounds(1000.0),
        );
        assert_eq!(same.unwrap_err().field, "point2_mm");
        let parallel = place(
            &BoltGroupInputs {
                plane_normal_mm: [1.0, 0.0, 0.0],
                ..group_inputs()
            },
            &group_bounds(1000.0),
        );
        assert_eq!(parallel.unwrap_err().field, "plane_normal_mm");
    }

    #[test]
    fn washer_switches_follow_the_definition() {
        // A definition without a washer refuses a switch that needs one.
        let mut bare = group_inputs();
        bare.fastener.washer = None;
        bare.fastener.washer_under_head = false;
        bare.fastener.washer_under_nut = false;
        bare.fastener.validate().unwrap();
        let mut head = bare.clone();
        head.washer_under_head = Some(true);
        assert_eq!(
            place(&head, &group_bounds(1000.0)).unwrap_err().field,
            "washer_under_head"
        );
        let mut nut = bare.clone();
        nut.washer_under_nut = Some(true);
        assert_eq!(
            place(&nut, &group_bounds(1000.0)).unwrap_err().field,
            "washer_under_nut"
        );
        // A screw has no nut, so no washer under one; absent switches take the defaults.
        let mut screw = bare;
        screw.fastener.kind = FastenerKind::Screw;
        screw.fastener.nut_across_flats_mm = None;
        screw.fastener.nut_height_mm = None;
        screw.fastener.validate().unwrap();
        let mut asked = screw.clone();
        asked.washer_under_nut = Some(true);
        assert_eq!(
            place(&asked, &group_bounds(1000.0)).unwrap_err().field,
            "washer_under_nut"
        );
        asked.washer_under_nut = None;
        asked.washer_under_head = None;
        let placed = place(&asked, &group_bounds(1000.0)).unwrap();
        assert!(!placed.washer_under_head && !placed.washer_under_nut);
        // A screw without a nut needs no nut height in the required length.
        let mut long = screw;
        long.fastener.lengths_mm = vec![14.0, 30.0, 45.0];
        long.fastener.validate().unwrap();
        // Grip 10 + protrusion 5 = 15: the smallest listed length is 30.
        assert_eq!(place(&long, &group_bounds(1000.0)).unwrap().length_mm, 30.0);
    }

    /// The placements of the standard two-plate test: identity placements.
    fn hole_args(inputs: &BoltGroupInputs) -> BTreeMap<ElementId, Placement> {
        let mut placements = BTreeMap::new();
        for part in &inputs.parts {
            placements.insert(*part, Placement::IDENTITY);
        }
        placements
    }

    #[test]
    fn generate_holes_cut_every_crossed_plate_at_every_fastener() {
        let inputs = group_inputs();
        let bounds = group_bounds(1000.0);
        let placed = place(&inputs, &bounds).unwrap();
        let translations = hole_args(&inputs);
        let holes = generate_holes(
            &inputs,
            &placed.row(),
            &bounds,
            &translations,
            &BTreeSet::new(),
        )
        .unwrap();
        // Three fasteners, both plates crossed by every axis: one hole each, 18 mm across,
        // centred on the bearing points' XY in the plate's local frame.
        assert_eq!(holes.len(), 6);
        assert!(holes.iter().all(|h| h.polygon_mm.len() >= 16));
        let centres = [(50.0, 0.0), (130.0, 0.0), (220.0, 0.0)];
        for (part_index, chunk) in holes.chunks(3).enumerate() {
            assert_eq!(chunk[0].part, if part_index == 0 { id(1) } else { id(2) });
            for (index, hole) in chunk.iter().enumerate() {
                let (cx, cy) = centres[index];
                let radius = hole
                    .polygon_mm
                    .iter()
                    .map(|p| (p[0] - cx).hypot(p[1] - cy))
                    .fold(0.0_f64, f64::max);
                assert!((radius - 9.0).abs() <= 1e-9, "nominal 16 + clearance 2");
            }
        }
        // The stored records live in the plates' local frames: a translated plate's holes are
        // translated back (the polygon centroid sits on the mapped axis point). Its local box
        // is the world box moved back by the translation.
        let mut moved = translations.clone();
        moved.insert(
            id(1),
            Placement {
                translation_mm: [10.0, 20.0, 0.0],
                ..Placement::IDENTITY
            },
        );
        let mut moved_slabs = bounds.clone();
        moved_slabs.insert(id(1), [[-10.0, -20.0, 0.0], [990.0, 480.0, 10.0]]);
        let local = generate_holes(
            &inputs,
            &placed.row(),
            &moved_slabs,
            &moved,
            &BTreeSet::new(),
        )
        .unwrap();
        let polygon = &local.first().unwrap().polygon_mm;
        let n = polygon.len() as f64;
        let centroid = [
            polygon.iter().map(|p| p[0]).sum::<f64>() / n,
            polygon.iter().map(|p| p[1]).sum::<f64>() / n,
        ];
        assert!((centroid[0] - 40.0).abs() <= 1e-9 && (centroid[1] + 20.0).abs() <= 1e-9);
        // A group's own clearance widens the hole over the definition's.
        let mut wide = inputs.clone();
        wide.hole_clearance_mm = Some(4.0);
        let widened = generate_holes(
            &wide,
            &placed.row(),
            &bounds,
            &translations,
            &BTreeSet::new(),
        )
        .unwrap();
        let radius = widened[0]
            .polygon_mm
            .iter()
            .map(|p| (p[0] - 50.0).hypot(p[1]))
            .fold(0.0_f64, f64::max);
        assert!((radius - 10.0).abs() <= 1e-9);
        // A screw defaults to none and writes no part; an explicit generate overrides it.
        let mut screw = inputs.clone();
        screw.fastener.kind = FastenerKind::Screw;
        screw.fastener.nut_across_flats_mm = None;
        screw.fastener.nut_height_mm = None;
        screw.fastener.washer_under_nut = false;
        screw.fastener.validate().unwrap();
        let quiet = generate_holes(
            &screw,
            &placed.row(),
            &bounds,
            &translations,
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(quiet.is_empty());
        let mut asked = screw.clone();
        asked.hole_mode = Some(HoleMode::Generate);
        let forced = generate_holes(
            &asked,
            &placed.row(),
            &bounds,
            &translations,
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(forced.len(), 6);
    }

    #[test]
    fn a_member_part_or_an_uncrossed_span_refuses_or_skips_holes() {
        let inputs = group_inputs();
        let bounds = group_bounds(1000.0);
        let placed = place(&inputs, &bounds).unwrap();
        let translations = hole_args(&inputs);
        // A member part is refused wholesale until G14.1 (nothing is written).
        let members = BTreeSet::from([id(2)]);
        assert_eq!(
            generate_holes(&inputs, &placed.row(), &bounds, &translations, &members)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        // A shank axis lying in the plate's plane (more than 60 degrees off its normal) is
        // refused: a through hole along the normal would not hold the shank.
        let mut flat = translations.clone();
        flat.insert(
            id(1),
            Placement {
                orientation_xyzw: [1.0, 0.0, 0.0, 1.0],
                ..Placement::IDENTITY
            },
        );
        let mut flat_slabs = bounds.clone();
        flat_slabs.insert(
            id(1),
            [[-1000.0, -1000.0, -1000.0], [1000.0, 1000.0, 1000.0]],
        );
        assert_eq!(
            generate_holes(&inputs, &placed.row(), &flat_slabs, &flat, &BTreeSet::new())
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        // A part only partly under the row takes only the holes the axes actually cross.
        let mut short = group_bounds(1000.0);
        short.insert(id(1), [[0.0, 0.0, 0.0], [100.0, 500.0, 10.0]]);
        let partial = generate_holes(
            &inputs,
            &placed.row(),
            &short,
            &translations,
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(
            partial.len(),
            4,
            "three holes on the long plate, one on the short"
        );
    }

    /// Every generated hole centre, mapped back to world by the plate's placement, lies on a
    /// shank axis of the row: the record is the axis's own crossing (issue 555 finding 2).
    fn assert_holes_on_axes(
        holes: &[GeneratedHole],
        row: &BoltRow,
        placement: &Placement,
        mid_z: f64,
    ) {
        assert!(!holes.is_empty());
        for hole in holes {
            let n = hole.polygon_mm.len() as f64;
            let cx = hole.polygon_mm.iter().map(|p| p[0]).sum::<f64>() / n;
            let cy = hole.polygon_mm.iter().map(|p| p[1]).sum::<f64>() / n;
            let turned =
                beamcraft_engine::solid::rotate_vector([cx, cy, mid_z], placement.orientation_xyzw);
            let world = [
                turned[0] + placement.translation_mm[0],
                turned[1] + placement.translation_mm[1],
                turned[2] + placement.translation_mm[2],
            ];
            let on_axis = row.bearings_mm.iter().any(|b| {
                let d = [world[0] - b[0], world[1] - b[1], world[2] - b[2]];
                let along = dot(d, row.z);
                let off = [
                    d[0] - along * row.z[0],
                    d[1] - along * row.z[1],
                    d[2] - along * row.z[2],
                ];
                dot(off, off).sqrt() <= 1e-6
            });
            assert!(on_axis, "hole centre {world:?} is off every shank axis");
        }
    }

    #[test]
    fn generated_holes_follow_any_plate_rotation() {
        // One plate, 300 x 200 x 10 locally, turned by each placement; the row is laid along
        // the turned plate's local +X at local y = 100 on its top face, with the shank along
        // the turned local -Z. Every crossing fastener cuts one hole whose centre maps back
        // onto its axis, for quarter turns about X and about Y, an arbitrary quaternion and the
        // turn of a `--plane` gusset in the world XZ plane.
        let slab: Bounds = [[0.0, 0.0, 0.0], [300.0, 200.0, 10.0]];
        let cases: [[f64; 4]; 4] = [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.3, -0.5, 0.2, 0.8],
            // The local frame of `--plane 0,0,0,1,0,0,0,0,1`: x -> +X, y -> +Z, z -> -Y.
            [
                core::f64::consts::FRAC_1_SQRT_2,
                0.0,
                0.0,
                core::f64::consts::FRAC_1_SQRT_2,
            ],
        ];
        for (index, orientation) in cases.into_iter().enumerate() {
            let placement = Placement {
                translation_mm: [17.0, -40.0, 250.0],
                orientation_xyzw: orientation,
            };
            let world = |p: [f64; 3]| {
                let turned = beamcraft_engine::solid::rotate_vector(p, orientation);
                [
                    turned[0] + placement.translation_mm[0],
                    turned[1] + placement.translation_mm[1],
                    turned[2] + placement.translation_mm[2],
                ]
            };
            let normal = beamcraft_engine::solid::rotate_vector([0.0, 0.0, -1.0], orientation);
            let mut inputs = group_inputs();
            // Two coincident plates: a group joins at least two parts.
            inputs.parts = vec![id(1), id(2)];
            inputs.point1_mm = world([0.0, 100.0, 10.0]);
            inputs.point2_mm = world([300.0, 100.0, 10.0]);
            inputs.plane_normal_mm = normal;
            let row = preview_row(&inputs).unwrap();
            let slabs = BTreeMap::from([(id(1), slab), (id(2), slab)]);
            let placements = BTreeMap::from([(id(1), placement), (id(2), placement)]);
            let holes =
                generate_holes(&inputs, &row, &slabs, &placements, &BTreeSet::new()).unwrap();
            assert_eq!(holes.len(), 6, "case {index}");
            assert_holes_on_axes(&holes, &row, &placement, 5.0);
        }
    }

    #[test]
    fn a_bottom_sided_rotated_plate_centres_its_holes_on_its_own_band() {
        // A `bottom` plate's material band is local z in [-10, 0]: the hole centre lies on the
        // axis at local z = -5, so an oblique shank (30 degrees off the normal) still cuts the
        // hole on its own axis (issue 555 finding 3, generate slab).
        let placement = Placement {
            translation_mm: [0.0, 0.0, 0.0],
            orientation_xyzw: [0.0, 1.0, 0.0, 1.0],
        };
        let slab: Bounds = [[0.0, 0.0, -10.0], [300.0, 200.0, 0.0]];
        let world =
            |p: [f64; 3]| beamcraft_engine::solid::rotate_vector(p, placement.orientation_xyzw);
        let (sin, cos) = (30.0_f64).to_radians().sin_cos();
        let mut inputs = group_inputs();
        inputs.parts = vec![id(1), id(2)];
        inputs.point1_mm = world([50.0, 100.0, 0.0]);
        inputs.point2_mm = world([250.0, 100.0, 0.0]);
        inputs.plane_normal_mm = world([sin, 0.0, -cos]);
        let row = preview_row(&inputs).unwrap();
        let slabs = BTreeMap::from([(id(1), slab), (id(2), slab)]);
        let placements = BTreeMap::from([(id(1), placement), (id(2), placement)]);
        let holes = generate_holes(&inputs, &row, &slabs, &placements, &BTreeSet::new()).unwrap();
        assert_holes_on_axes(&holes, &row, &placement, -5.0);
    }

    #[test]
    fn create_and_update_decide_identity_parts_and_read_the_definition() {
        let parts = parts();
        let command = CreateBoltGroup {
            new_id: id(9),
            name: "G".into(),
            inputs: group_inputs(),
        };
        let bounds = group_bounds(1000.0);
        let (placed, footprint) = parts
            .create_bolt_group(&command, &bounds, &box_faces_of(&bounds))
            .unwrap();
        assert_eq!(placed.length_mm, 50.0);
        // The parts' lifecycles and the definition are read; the group is written.
        assert!(
            footprint
                .reads()
                .contains(&beamcraft_engine::footprint::Entry {
                    target: beamcraft_engine::footprint::Target::Definition(
                        beamcraft_model::digest::ContentDigest::from_bytes([3u8; 32])
                    ),
                    group: beamcraft_engine::footprint::Group::Relationship,
                })
        );
        // A taken identity outside the parts list is DuplicateId; one of the parts is the
        // model's own-part refusal.
        let taken = CreateBoltGroup {
            new_id: id(4),
            ..command.clone()
        };
        assert_eq!(
            parts
                .create_bolt_group(&taken, &bounds, &box_faces_of(&bounds))
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        let unknown = CreateBoltGroup {
            inputs: BoltGroupInputs {
                parts: vec![id(1), id(7)],
                ..group_inputs()
            },
            ..command.clone()
        };
        assert_eq!(
            parts
                .create_bolt_group(&unknown, &bounds, &box_faces_of(&bounds))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        let assembly = CreateBoltGroup {
            inputs: BoltGroupInputs {
                parts: vec![id(1), id(3)],
                ..group_inputs()
            },
            ..command.clone()
        };
        assert_eq!(
            parts
                .create_bolt_group(&assembly, &bounds, &box_faces_of(&bounds))
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        let other = CreateBoltGroup {
            inputs: BoltGroupInputs {
                parts: vec![id(1), id(4)],
                ..group_inputs()
            },
            ..command.clone()
        };
        assert_eq!(
            parts
                .create_bolt_group(&other, &bounds, &box_faces_of(&bounds))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // The update decision reads before it places and names its target.
        let edit = UpdateBoltGroup {
            target_id: id(9),
            name: None,
            inputs: group_inputs(),
        };
        assert_eq!(
            parts
                .update_bolt_group(&edit, &bounds, &box_faces_of(&bounds))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        let mut with_group = crate::engine::weld::Parts::default();
        with_group.insert(id(1), PartKind::Part);
        with_group.insert(id(2), PartKind::Part);
        with_group.insert(id(9), PartKind::Other);
        let (placed, footprint) = with_group
            .update_bolt_group(&edit, &bounds, &box_faces_of(&bounds))
            .unwrap();
        assert_eq!(placed.bearings_mm.len(), 3);
        // The group's shape inputs and record are written; a repin that pins in the same
        // transaction writes the definition's entry as well.
        let element_writes = footprint
            .writes()
            .iter()
            .filter(|e| e.target == beamcraft_engine::footprint::Target::Element(id(9)))
            .count();
        assert_eq!(element_writes, 2);
        assert_eq!(footprint.writes().len(), 3);
    }

    /// The knee brace of issue 519, at the dimensions of the CLI joint: a 12 mm gusset stood
    /// up in the world XZ plane over x in [450, 850], z in [1400, 1800], and a 60 x 60 brace
    /// swept 800 mm along its local +Z, turned 40 degrees about +Y and set down at
    /// (100, 0, 1100). The row runs along +X with the shanks along +Y, bearings inside both
    /// parts at y = -6.5, so the requirement lands strictly inside a length step and no
    /// rounding can move the pick.
    fn knee_inputs() -> BoltGroupInputs {
        let fastener = crate::model::fastener::FastenerSpec {
            designation: Some("M12".into()),
            standard: None,
            grade: None,
            display_name: None,
            kind: FastenerKind::Bolt,
            head_kind: crate::model::fastener::HeadKind::Hex,
            nominal_diameter_mm: 12.0,
            head_across_flats_mm: 18.0,
            head_height_mm: 7.5,
            nut_across_flats_mm: Some(18.0),
            nut_height_mm: Some(9.0),
            washer: Some(crate::model::fastener::WasherSpec {
                outer_diameter_mm: 24.0,
                inner_diameter_mm: 13.0,
                thickness_mm: 2.0,
            }),
            hole_clearance_mm: 1.5,
            // The face grip is 36.5 (36.5 + washer 2 + nut 9 + head 7.5 + protrusion 3 = 58):
            // the listed 55 refuses and the listed 60 covers, which pins the grip between
            // them.
            lengths_mm: vec![30.0, 40.0, 50.0, 55.0, 60.0, 80.0],
            thread_protrusion_mm: 3.0,
            washer_under_head: true,
            washer_under_nut: true,
            hole_mode: crate::model::fastener::HoleMode::None,
            mass_each_kg: None,
        };
        fastener.validate().unwrap();
        BoltGroupInputs {
            parts: vec![id(1), id(2)],
            fastener_digest: beamcraft_model::digest::ContentDigest::from_bytes([4u8; 32]),
            fastener,
            point1_mm: [500.0, -6.5, 1600.0],
            point2_mm: [650.0, -6.5, 1600.0],
            plane_normal_mm: [0.0, 1.0, 0.0],
            rotation_rad: 0.0,
            on_plane_mm: 0.0,
            depth_mm: 0.0,
            offset_start_mm: 25.0,
            spacing: BoltSpacing::Spacings(vec![100.0]),
            washer_under_head: None,
            washer_under_nut: None,
            length: BoltLength::Auto,
            hole_mode: None,
            hole_clearance_mm: None,
        }
    }

    /// The same joint as the adapter derives it: the plate's exact contour box and the brace's
    /// world axis expanded by half the section's bounding-box diagonal, which a 40-degree
    /// placement inflates far past the solid along the shank.
    fn knee_bounds() -> BTreeMap<ElementId, Bounds> {
        let (sin, cos) = (40.0_f64).to_radians().sin_cos();
        let reach = 0.5 * (60.0f64.hypot(60.0));
        let end = [100.0 + 800.0 * sin, 0.0, 1100.0 + 800.0 * cos];
        let mut map = BTreeMap::new();
        map.insert(
            id(1),
            [
                [
                    (100.0 - reach).min(end[0] - reach),
                    -reach,
                    (1100.0 - reach).min(end[2] - reach),
                ],
                [
                    (100.0 + reach).max(end[0] + reach),
                    reach,
                    (1100.0 + reach).max(end[2] + reach),
                ],
            ],
        );
        map.insert(id(2), [[450.0, -12.0, 1400.0], [850.0, 0.0, 1800.0]]);
        map
    }

    /// The world triangles of the knee-brace joint: the gusset is the exact 12 mm slab, the
    /// brace its square section swept 800 mm along the local +Z turned 40 degrees about +Y.
    /// The faces are double-sided; the engine pairs the crossings either way.
    fn knee_faces() -> BTreeMap<ElementId, Faces> {
        let (sin, cos) = (40.0_f64).to_radians().sin_cos();
        // The brace's corners in its local frame, turned about +Y and set down at
        // (100, 0, 1100): x' = x cos + z sin, z' = -x sin + z cos.
        let at =
            |x: f64, y: f64, z: f64| [x * cos + z * sin + 100.0, y, -x * sin + z * cos + 1100.0];
        let mut faces = Faces::new();
        let mut quad = |a, b, c, d| {
            faces.push([a, b, c]);
            faces.push([a, c, d]);
        };
        let (w, l) = (30.0_f64, 800.0_f64);
        quad(
            at(-w, -w, 0.0),
            at(w, -w, 0.0),
            at(w, w, 0.0),
            at(-w, w, 0.0),
        );
        quad(at(-w, -w, l), at(-w, w, l), at(w, w, l), at(w, -w, l));
        quad(at(-w, -w, 0.0), at(-w, -w, l), at(w, -w, l), at(w, -w, 0.0));
        quad(at(-w, w, 0.0), at(w, w, 0.0), at(w, w, l), at(-w, w, l));
        quad(at(-w, -w, 0.0), at(-w, w, 0.0), at(-w, w, l), at(-w, -w, l));
        quad(at(w, -w, 0.0), at(w, -w, l), at(w, w, l), at(w, w, 0.0));
        let mut map = BTreeMap::new();
        map.insert(id(1), faces);
        map.insert(
            id(2),
            box_faces([450.0, -12.0, 1400.0], [850.0, 0.0, 1800.0]),
        );
        map
    }

    #[test]
    fn the_far_face_is_where_the_joint_ends_on_each_shank() {
        // Issue 562 item 5: the scene seats the nut stack on the joint's far face. For the
        // knee the first fastener's rays leave the brace solid 36.5 mm from the bearing (its
        // face grip); a part list without the brace still finds the plate; an empty list
        // finds nothing.
        let bounds = knee_bounds();
        let faces = knee_faces();
        let inputs = knee_inputs();
        let placed = place_bolt_group(&inputs, &bounds, &faces).unwrap();
        let clearance = inputs.resolved_hole_clearance_mm();
        let far = far_faces(
            &placed,
            &inputs.fastener,
            clearance,
            &inputs.parts,
            &bounds,
            &faces,
        );
        assert_eq!(far.len(), 2);
        let first = far.first().copied().flatten().unwrap();
        assert!((first - 36.5).abs() < 1e-6, "{far:?}");
        // The length (60) covers the face plus the nut stack and the protrusion, so the
        // seated washer sits at or below where the tip-hung one would.
        assert!(first <= placed.length_mm);
        let plate_only = far_faces(
            &placed,
            &inputs.fastener,
            clearance,
            &[id(2)],
            &bounds,
            &faces,
        );
        assert!(plate_only.iter().all(Option::is_some), "{plate_only:?}");
        let none = far_faces(&placed, &inputs.fastener, clearance, &[], &bounds, &faces);
        assert!(none.iter().all(Option::is_none));
    }

    #[test]
    fn a_rotated_knee_brace_requires_its_face_grip_not_its_box_grip() {
        // The brace's conservative box reaches 48.9 mm along the shank where its solid grips
        // 36.5 mm (from the bearing inside it to the far face), so the requirement is
        // 36.5 + washer 2 + nut 9 + head 7.5 + protrusion 3 = 58 and the smallest covering
        // length is 60, not the box-driven 80 that exceeds every real library length
        // (issue 519 defect B). The second fastener's axis runs past the brace solid at
        // x = 625: the box still admits it, it adds no grip, and the first bearing pins the
        // length.
        let bounds = knee_bounds();
        let faces = knee_faces();
        let placed = place_bolt_group(&knee_inputs(), &bounds, &faces).unwrap();
        assert_eq!(placed.length_mm, 60.0);
        // The grip boundary: a listed 55 refuses, a listed 60 covers the 58 requirement.
        let mut short = knee_inputs();
        short.length = BoltLength::Fixed(55.0);
        assert_eq!(
            place_bolt_group(&short, &bounds, &faces).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
        let mut exact = knee_inputs();
        exact.length = BoltLength::Fixed(60.0);
        assert_eq!(
            place_bolt_group(&exact, &bounds, &faces).unwrap().length_mm,
            60.0
        );
        // A part without faces grips per its conservative box, which errs long: 80, never
        // under the real grip.
        let mut without = faces;
        without.remove(&id(1));
        assert_eq!(
            place_bolt_group(&knee_inputs(), &bounds, &without)
                .unwrap()
                .length_mm,
            80.0
        );
    }

    #[test]
    fn the_grip_merges_the_double_hit_of_a_shared_triangle_edge() {
        // A bolt centred on the plate: the ear-clip diagonal of each cap passes through the
        // axis, so both cap triangles of a face hit a few ulps apart and must merge into one
        // crossing. Pairing them as zero-length intervals would collapse the grip to nothing
        // and resolve a far too short length (issue 519 review).
        let mut inputs = group_inputs();
        inputs.point1_mm = [0.0, 0.0, -5.0];
        inputs.point2_mm = [400.0, 200.0, -5.0];
        inputs.offset_start_mm = 400.0;
        inputs.spacing = BoltSpacing::Spacings(vec![100.0]);
        // The row runs along the caps' (0,0)-(1000,500) ear-clip diagonal, so both bearings
        // land on the shared edge of each cap's two triangles, below the plate: grip 10, so
        // the requirement is 10 + washers 2 x 3 + nut 14.8 + head 10 + protrusion 5 = 45.8
        // and the length is 50; collapsed (t, t) pairings would pick 40.
        let placed = place(&inputs, &group_bounds(1000.0)).unwrap();
        assert_eq!(placed.length_mm, 50.0);
    }

    #[test]
    fn a_coaxial_hole_grips_its_bearing_ring_whoever_cut_it() {
        // A part whose mesh carries clearance holes coaxial with the shanks — the group's own,
        // another group's, an `add-hole` record — crosses no face on the axis, but the bearing
        // ring around the hole is solid: the grip is the whole pack (issue 555 finding 1;
        // issue 519 review). The second plate carries 20 x 60 rectangular voids around the
        // bearings (50, 250) and (130, 250); its ring rays at +-x find the slab.
        let mut inputs = group_inputs();
        inputs.point1_mm = [0.0, 250.0, 0.0];
        inputs.point2_mm = [400.0, 250.0, 0.0];
        inputs.offset_start_mm = 50.0;
        inputs.spacing = BoltSpacing::Spacings(vec![80.0]);
        let mut bounds = BTreeMap::new();
        bounds.insert(id(1), [[0.0, 0.0, 0.0], [1000.0, 500.0, 10.0]]);
        bounds.insert(id(2), [[0.0, 0.0, 10.0], [1000.0, 500.0, 20.0]]);
        let mut faces = BTreeMap::new();
        faces.insert(id(1), box_faces([0.0, 0.0, 0.0], [1000.0, 500.0, 10.0]));
        faces.insert(
            id(2),
            box_faces([0.0, 0.0, 10.0], [40.0, 500.0, 20.0])
                .into_iter()
                .chain(box_faces([60.0, 0.0, 10.0], [120.0, 500.0, 20.0]))
                .chain(box_faces([140.0, 0.0, 10.0], [1000.0, 500.0, 20.0]))
                .chain(box_faces([40.0, 0.0, 10.0], [60.0, 220.0, 20.0]))
                .chain(box_faces([40.0, 280.0, 10.0], [60.0, 500.0, 20.0]))
                .chain(box_faces([120.0, 0.0, 10.0], [140.0, 220.0, 20.0]))
                .chain(box_faces([120.0, 280.0, 10.0], [140.0, 500.0, 20.0]))
                .collect(),
        );
        // The grip is 20: 20 + head washer 3 + nut washer 3 + nut 14.8 + head 10 + protrusion
        // 5 = 55.8, so the length is 60 — the same as over the solid plates, whatever order
        // the holes and the group were made in.
        let holed = place_bolt_group(&inputs, &bounds, &faces).unwrap();
        let solid = place(&inputs, &bounds).unwrap();
        assert_eq!(holed.length_mm, solid.length_mm);
        assert_eq!(holed.length_mm, 60.0);
    }

    #[test]
    fn a_part_whose_opening_swallows_the_bearing_ring_is_refused() {
        // A listed plate crossed by the axis through an opening wider than the washer: no
        // ray of the bearing ring finds its material, so the group would not clamp it. The
        // part is refused, never silently dropped from the grip (issue 555 finding 1).
        let mut inputs = group_inputs();
        inputs.point1_mm = [0.0, 250.0, 0.0];
        inputs.point2_mm = [400.0, 250.0, 0.0];
        inputs.offset_start_mm = 50.0;
        inputs.spacing = BoltSpacing::Spacings(vec![80.0]);
        let mut bounds = BTreeMap::new();
        bounds.insert(id(1), [[0.0, 0.0, 0.0], [1000.0, 500.0, 10.0]]);
        bounds.insert(id(2), [[0.0, 0.0, 10.0], [1000.0, 500.0, 20.0]]);
        let mut faces = BTreeMap::new();
        faces.insert(id(1), box_faces([0.0, 0.0, 0.0], [1000.0, 500.0, 10.0]));
        // 200 x 200 opening around both bearings.
        faces.insert(
            id(2),
            box_faces([0.0, 0.0, 10.0], [0.0001, 500.0, 20.0])
                .into_iter()
                .chain(box_faces([200.0, 0.0, 10.0], [1000.0, 500.0, 20.0]))
                .chain(box_faces([0.0001, 0.0, 10.0], [200.0, 150.0, 20.0]))
                .chain(box_faces([0.0001, 350.0, 10.0], [200.0, 500.0, 20.0]))
                .collect(),
        );
        let refused = place_bolt_group(&inputs, &bounds, &faces).unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidArgument);
        assert_eq!(refused.field, "parts");
    }

    #[test]
    fn the_bearing_ring_follows_the_washer_actually_placed_under_the_head() {
        // The head-side plate carries, around each bearing, only a square ring of material
        // between half-sizes 10 and 10.8 (z 0..10): the axis falls into the inner void, the
        // rays at the head ring radius 10.5 hit the ring, the rays at the washer radius 12
        // find nothing. With a washer resolved under the head the part honestly refuses;
        // with `washer_under_head: false` the ring must shrink to the head across flats —
        // probing to a washer OD that is not there gave a false "never clamped" (review on
        // issue 555).
        let mut inputs = group_inputs();
        inputs.point1_mm = [0.0, 250.0, 0.0];
        inputs.point2_mm = [400.0, 250.0, 0.0];
        inputs.offset_start_mm = 50.0;
        inputs.spacing = BoltSpacing::Spacings(vec![80.0]);
        inputs.washer_under_head = Some(false);
        let mut bounds = BTreeMap::new();
        bounds.insert(id(1), [[0.0, 0.0, 0.0], [1000.0, 500.0, 10.0]]);
        bounds.insert(id(2), [[0.0, 0.0, 10.0], [1000.0, 500.0, 20.0]]);
        let mut faces = BTreeMap::new();
        let mut rings = Faces::new();
        for bearing_x in [50.0, 130.0] {
            // The square ring between half-sizes 10 and 10.8 around the bearing: four strips.
            let inner = [bearing_x - 10.0, 240.0, 0.0];
            let inner_max = [bearing_x + 10.0, 260.0, 10.0];
            let outer = [bearing_x - 10.8, 239.2, 0.0];
            let outer_max = [bearing_x + 10.8, 260.8, 10.0];
            rings = rings
                .into_iter()
                .chain(box_faces(
                    [outer[0], outer[1], 0.0],
                    [outer_max[0], inner[1], 10.0],
                ))
                .chain(box_faces(
                    [outer[0], inner_max[1], 0.0],
                    [outer_max[0], outer_max[1], 10.0],
                ))
                .chain(box_faces(
                    [outer[0], inner[1], 0.0],
                    [inner[0], inner_max[1], 10.0],
                ))
                .chain(box_faces(
                    [inner_max[0], inner[1], 0.0],
                    [outer_max[0], inner_max[1], 10.0],
                ))
                .collect();
        }
        faces.insert(id(1), rings);
        faces.insert(id(2), box_faces([0.0, 0.0, 10.0], [1000.0, 500.0, 20.0]));
        // Grip 20: 20 + nut washer 3 + nut 14.8 + head 10 + protrusion 5 = 52.8, so the
        // length is 60.
        let placed = place_bolt_group(&inputs, &bounds, &faces).unwrap();
        assert_eq!(placed.length_mm, 60.0);
        // The same geometry with the washer really under the head probes to its OD and
        // refuses: no ray of the ring finds the part.
        let mut washer = inputs;
        washer.washer_under_head = Some(true);
        let refused = place_bolt_group(&washer, &bounds, &faces).unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidArgument);
        assert_eq!(refused.field, "parts");
    }

    #[test]
    fn a_stored_group_re_derives_the_placement_the_creation_decided() {
        let inputs = group_inputs();
        let placed = place(&inputs, &group_bounds(1000.0)).unwrap();
        let stored = crate::engine::bolt::StoredPlacement {
            point1_mm: inputs.point1_mm,
            point2_mm: inputs.point2_mm,
            plane_normal_mm: inputs.plane_normal_mm,
            rotation_rad: inputs.rotation_rad,
            on_plane_mm: inputs.on_plane_mm,
            depth_mm: inputs.depth_mm,
            offset_start_mm: inputs.offset_start_mm,
            spacings_mm: placed.spacings_mm.clone(),
            length_mm: placed.length_mm,
            washer_under_head: placed.washer_under_head,
            washer_under_nut: placed.washer_under_nut,
        };
        assert_eq!(BoltGroupPlaced::from_stored(&stored).unwrap(), placed);
        // The arithmetic also agrees for a turned, slid and sunk row with a generator pattern.
        let turned = place(
            &BoltGroupInputs {
                rotation_rad: 1.0,
                on_plane_mm: 7.0,
                depth_mm: -3.0,
                spacing: BoltSpacing::Spacings(vec![40.0, 40.0, 40.0]),
                ..inputs.clone()
            },
            &group_bounds(1000.0),
        )
        .unwrap();
        let stored = crate::engine::bolt::StoredPlacement {
            point1_mm: inputs.point1_mm,
            point2_mm: inputs.point2_mm,
            plane_normal_mm: inputs.plane_normal_mm,
            rotation_rad: 1.0,
            on_plane_mm: 7.0,
            depth_mm: -3.0,
            offset_start_mm: inputs.offset_start_mm,
            spacings_mm: turned.spacings_mm.clone(),
            length_mm: turned.length_mm,
            washer_under_head: turned.washer_under_head,
            washer_under_nut: turned.washer_under_nut,
        };
        assert_eq!(BoltGroupPlaced::from_stored(&stored).unwrap(), turned);
        // A committed row that gives no frame is refused, never guessed: a degenerate row and
        // a normal parallel to the row are the creation's own refusals.
        let degenerate = crate::engine::bolt::StoredPlacement {
            point2_mm: inputs.point1_mm,
            ..stored.clone()
        };
        assert_eq!(
            BoltGroupPlaced::from_stored(&degenerate).unwrap_err().field,
            "point2_mm"
        );
        let parallel = crate::engine::bolt::StoredPlacement {
            plane_normal_mm: [1.0, 0.0, 0.0],
            ..stored
        };
        assert_eq!(
            BoltGroupPlaced::from_stored(&parallel).unwrap_err().field,
            "plane_normal_mm"
        );
    }

    #[test]
    fn the_preview_row_matches_the_placement_the_parts_admit() {
        // The preview decides the same row the commit decides: the frame, the bearings and the
        // spacings agree across the plain and the turned, slid and generated placements; the
        // length and the washers stay the parts' decision.
        let inputs = group_inputs();
        let placed = place(&inputs, &group_bounds(1000.0)).unwrap();
        assert_eq!(preview_row(&inputs).unwrap(), placed.row());
        let turned = BoltGroupInputs {
            rotation_rad: 1.0,
            on_plane_mm: 7.0,
            depth_mm: -3.0,
            spacing: BoltSpacing::Pitch {
                count: 3,
                pitch_mm: 40.0,
            },
            ..inputs.clone()
        };
        let placed = place(&turned, &group_bounds(1000.0)).unwrap();
        assert_eq!(preview_row(&turned).unwrap(), placed.row());
    }

    #[test]
    fn the_preview_row_refuses_what_the_command_refuses() {
        // The argument guard runs first, then the row: the same fields the commit names.
        let inputs = group_inputs();
        assert_eq!(
            preview_row(&BoltGroupInputs {
                spacing: BoltSpacing::Spacings(vec![0.0]),
                ..inputs.clone()
            })
            .unwrap_err()
            .field,
            "spacings_mm"
        );
        assert_eq!(
            preview_row(&BoltGroupInputs {
                point2_mm: inputs.point1_mm,
                ..inputs.clone()
            })
            .unwrap_err()
            .field,
            "point2_mm"
        );
        assert_eq!(
            preview_row(&BoltGroupInputs {
                plane_normal_mm: [1.0, 0.0, 0.0],
                ..inputs.clone()
            })
            .unwrap_err()
            .field,
            "plane_normal_mm"
        );
        // An offset-end row that does not fit between the points is refused like the commit's.
        assert_eq!(
            preview_row(&BoltGroupInputs {
                offset_start_mm: 500.0,
                spacing: BoltSpacing::OffsetEnd {
                    count: 3,
                    offset_end_mm: 50.0,
                },
                ..inputs
            })
            .unwrap_err()
            .field,
            "offset_end_mm"
        );
    }
}
