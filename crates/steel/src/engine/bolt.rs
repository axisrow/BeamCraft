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
//! reachability of a part stays a conservative box test like A05's, with the grip read as the
//! union length of the shank axis inside the boxes. A14.4 adds the hole decision
//! ([`generate_holes`]): with `hole_mode = generate` every connected plate takes one ordinary
//! A04 hole record per crossing fastener, owned by the group; a member part refuses until the
//! G14 contract lands, and `hole_mode = none` writes no part.

use crate::model::command::{
    BoltGroupInputs, BoltLength, BoltSpacing, CreateBolt, CreateBoltGroup, UpdateBoltGroup,
};
use crate::model::fastener::{FastenerKind, HoleMode};
use beamcraft_engine::footprint::Footprint;
use beamcraft_geom::tolerance::ABSOLUTE_FLOOR_MM;
use beamcraft_geom::tolerance::DIMENSION_CAP_MM;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::{BTreeMap, BTreeSet};

/// Conservative world bounds of one part: componentwise minimum, then maximum, both inclusive.
pub type Bounds = [[f64; 3]; 2];

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

/// Places the group (bolt amendment section 4): the frame, the authoritative spacings, the
/// bearing points and the one length for the whole group. The grip of a fastener is the union
/// length of its axis inside the listed parts; `auto` picks the smallest standard length that
/// covers the largest grip plus the far washers, the nut and the thread protrusion, so every
/// fastener of the group is identical. The under-head washer lies outside the grip, before the
/// shank starts, and adds nothing; the under-nut washer lies between the grip and the nut and
/// adds its thickness. A fixed length shorter than that same requirement is refused. Every
/// listed part must be crossed by the axis of at least one fastener; the length choice covers
/// every crossed part, so the crossing check on the unbounded axis is enough. The parts are
/// decided by the same conservative boxes as A05; the boxes overlap in mid-air and the grip is
/// never a face-exact value (see the A05 design note).
pub fn place_bolt_group(
    inputs: &BoltGroupInputs,
    bounds: &BTreeMap<ElementId, Bounds>,
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
    let (x, y, z) = frame_of(
        inputs.point1_mm,
        inputs.point2_mm,
        inputs.plane_normal_mm,
        inputs.rotation_rad,
    )?;
    let chord = (inputs.point2_mm[0] - inputs.point1_mm[0]).hypot(
        (inputs.point2_mm[1] - inputs.point1_mm[1])
            .hypot(inputs.point2_mm[2] - inputs.point1_mm[2]),
    );
    let spacings = expand_spacings(&inputs.spacing, chord, inputs.offset_start_mm)?;
    let first = [
        inputs.point1_mm[0] + inputs.on_plane_mm * y[0] + inputs.depth_mm * z[0],
        inputs.point1_mm[1] + inputs.on_plane_mm * y[1] + inputs.depth_mm * z[1],
        inputs.point1_mm[2] + inputs.on_plane_mm * y[2] + inputs.depth_mm * z[2],
    ];
    let mut bearings = Vec::with_capacity(spacings.len() + 1);
    let mut along = inputs.offset_start_mm;
    for _ in 0..=spacings.len() {
        bearings.push([
            first[0] + along * x[0],
            first[1] + along * x[1],
            first[2] + along * x[2],
        ]);
        let Some(next) = spacings.get(bearings.len() - 1) else {
            break;
        };
        along += *next;
    }
    // The grip decides the one length of the group; every part must be crossed by an axis.
    let mut crossed = vec![false; inputs.parts.len()];
    let mut max_grip = 0.0_f64;
    for bearing in &bearings {
        let mut intervals = Vec::new();
        for (index, part) in inputs.parts.iter().enumerate() {
            if let Some(found) = bounds.get(part).and_then(|b| axis_interval(bearing, &z, b)) {
                if let Some(done) = crossed.get_mut(index) {
                    *done = true;
                }
                intervals.push(found);
            }
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
    let far_washer = if washer_under_nut {
        spec.washer.map(|w| w.thickness_mm).unwrap_or(0.0)
    } else {
        0.0
    };
    let nut_height = spec.nut_height_mm.unwrap_or(0.0);
    let required = max_grip + far_washer + nut_height + spec.thread_protrusion_mm;
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

/// Decides the generated holes of a placed group (bolt amendment section 5): with
/// `hole_mode = generate`, one round through hole per crossing fastener on every connected
/// plate, of diameter `nominal + clearance` (the group's own clearance, or the definition's),
/// centred where the shank axis crosses the plate's mid-plane and cut along the plate normal,
/// like any A04 record. A beam or a column is refused with `UNSUPPORTED_GEOMETRY`:
/// generated member holes need the kernel slice G14.1. `hole_mode = none` writes no part.
/// The decision is pure: the same inputs give the same holes.
pub fn generate_holes(
    inputs: &BoltGroupInputs,
    placed: &BoltGroupPlaced,
    bounds: &BTreeMap<ElementId, Bounds>,
    translations: &BTreeMap<ElementId, [f64; 3]>,
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
        let slab = bounds.get(part).ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidArgument,
                "parts",
                "a listed part is unreachable",
            )
        })?;
        let translation = translations.get(part).ok_or_else(|| {
            CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "hole_mode",
                "the part has no placement this slice can read",
            )
        })?;
        for bearing in &placed.bearings_mm {
            let Some((start, exit)) = axis_interval(bearing, &placed.z, slab) else {
                continue;
            };
            // The plate-local XY centre: where the shank axis crosses the slab's mid-plane
            // (a plate's placement rotation is identity in this slice, so world minus
            // translation is the plate-local frame).
            let mid = (start + exit) / 2.0;
            let centre = [
                bearing[0] + placed.z[0] * mid - translation[0],
                bearing[1] + placed.z[1] * mid - translation[1],
            ];
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
        let placed = place_bolt_group(&command.inputs, bounds)?;
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
        let placed = place_bolt_group(&command.inputs, bounds)?;
        let footprint = Footprint::for_bolt_group_edit(
            command.target_id,
            &command.inputs.parts,
            command.inputs.fastener_digest,
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
            lengths_mm: vec![30.0, 35.0, 45.0],
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
        let placed = place_bolt_group(&frame, &bounds).unwrap();
        assert_eq!(placed.x, [1.0, 0.0, 0.0]);
        assert_eq!(placed.y, [0.0, 1.0, 0.0]);
        assert_eq!(placed.z, [0.0, 0.0, 1.0]);
        // Rotation of a quarter turn turns the plane axis up and the shank axis to -y.
        let turned = place_bolt_group(
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
        let moved = place_bolt_group(
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
        let pitch = place_bolt_group(
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
        let offset_end = place_bolt_group(
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
        let moved = place_bolt_group(
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
        let tight = place_bolt_group(
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
        // Grip 10 (the two overlapping plates count once) + under-nut washer 3 + nut 14.8 +
        // protrusion 5 = 32.8: the smallest listed length is 35. A wrongly summed grip of 20
        // would pick 45. The under-head washer lies outside the grip and adds nothing.
        let placed = place_bolt_group(&group_inputs(), &group_bounds(1000.0)).unwrap();
        assert_eq!(placed.length_mm, 35.0);
        assert!(placed.washer_under_head && placed.washer_under_nut);
        // Without the far washer the requirement is 29.8, so 30 covers it.
        let bare = place_bolt_group(
            &BoltGroupInputs {
                washer_under_nut: Some(false),
                ..group_inputs()
            },
            &group_bounds(1000.0),
        )
        .unwrap();
        assert_eq!(bare.length_mm, 30.0);
    }

    #[test]
    fn crossing_and_length_refusals_change_nothing() {
        // A part off the axis is never crossed.
        let off = place_bolt_group(
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
            place_bolt_group(&short, &group_bounds(1000.0))
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        // A fixed length below the requirement is refused; an adequate one is kept.
        let mut fixed = group_inputs();
        fixed.length = BoltLength::Fixed(30.0);
        assert_eq!(
            place_bolt_group(&fixed, &group_bounds(1000.0))
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        fixed.length = BoltLength::Fixed(35.0);
        assert_eq!(
            place_bolt_group(&fixed, &group_bounds(1000.0))
                .unwrap()
                .length_mm,
            35.0
        );
        // Degenerate rows and a normal parallel to the row are refused.
        let same = place_bolt_group(
            &BoltGroupInputs {
                point2_mm: [0.0, 0.0, 0.0],
                ..group_inputs()
            },
            &group_bounds(1000.0),
        );
        assert_eq!(same.unwrap_err().field, "point2_mm");
        let parallel = place_bolt_group(
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
            place_bolt_group(&head, &group_bounds(1000.0))
                .unwrap_err()
                .field,
            "washer_under_head"
        );
        let mut nut = bare.clone();
        nut.washer_under_nut = Some(true);
        assert_eq!(
            place_bolt_group(&nut, &group_bounds(1000.0))
                .unwrap_err()
                .field,
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
            place_bolt_group(&asked, &group_bounds(1000.0))
                .unwrap_err()
                .field,
            "washer_under_nut"
        );
        asked.washer_under_nut = None;
        asked.washer_under_head = None;
        let placed = place_bolt_group(&asked, &group_bounds(1000.0)).unwrap();
        assert!(!placed.washer_under_head && !placed.washer_under_nut);
        // A screw without a nut needs no nut height in the required length.
        let mut long = screw;
        long.fastener.lengths_mm = vec![14.0, 30.0, 45.0];
        long.fastener.validate().unwrap();
        // Grip 10 + protrusion 5 = 15: the smallest listed length is 30.
        assert_eq!(
            place_bolt_group(&long, &group_bounds(1000.0))
                .unwrap()
                .length_mm,
            30.0
        );
    }

    /// The translations of the standard two-plate test: identity placements.
    fn hole_args(inputs: &BoltGroupInputs) -> BTreeMap<ElementId, [f64; 3]> {
        let mut translations = BTreeMap::new();
        for part in &inputs.parts {
            translations.insert(*part, [0.0, 0.0, 0.0]);
        }
        translations
    }

    #[test]
    fn generate_holes_cut_every_crossed_plate_at_every_fastener() {
        let inputs = group_inputs();
        let bounds = group_bounds(1000.0);
        let placed = place_bolt_group(&inputs, &bounds).unwrap();
        let translations = hole_args(&inputs);
        let holes =
            generate_holes(&inputs, &placed, &bounds, &translations, &BTreeSet::new()).unwrap();
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
        // translated back (the polygon centroid sits on the mapped axis point).
        let mut moved = translations.clone();
        moved.insert(id(1), [10.0, 20.0, 0.0]);
        let local = generate_holes(&inputs, &placed, &bounds, &moved, &BTreeSet::new()).unwrap();
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
        let widened =
            generate_holes(&wide, &placed, &bounds, &translations, &BTreeSet::new()).unwrap();
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
        let quiet =
            generate_holes(&screw, &placed, &bounds, &translations, &BTreeSet::new()).unwrap();
        assert!(quiet.is_empty());
        let mut asked = screw.clone();
        asked.hole_mode = Some(HoleMode::Generate);
        let forced =
            generate_holes(&asked, &placed, &bounds, &translations, &BTreeSet::new()).unwrap();
        assert_eq!(forced.len(), 6);
    }

    #[test]
    fn a_member_part_or_an_uncrossed_span_refuses_or_skips_holes() {
        let inputs = group_inputs();
        let bounds = group_bounds(1000.0);
        let placed = place_bolt_group(&inputs, &bounds).unwrap();
        let translations = hole_args(&inputs);
        // A member part is refused wholesale until G14.1 (nothing is written).
        let members = BTreeSet::from([id(2)]);
        assert_eq!(
            generate_holes(&inputs, &placed, &bounds, &translations, &members)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        // A part only partly under the row takes only the holes the axes actually cross.
        let mut short = group_bounds(1000.0);
        short.insert(id(1), [[0.0, 0.0, 0.0], [100.0, 500.0, 10.0]]);
        let partial =
            generate_holes(&inputs, &placed, &short, &translations, &BTreeSet::new()).unwrap();
        assert_eq!(
            partial.len(),
            4,
            "three holes on the long plate, one on the short"
        );
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
        let (placed, footprint) = parts.create_bolt_group(&command, &bounds).unwrap();
        assert_eq!(placed.length_mm, 35.0);
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
            parts.create_bolt_group(&taken, &bounds).unwrap_err().code,
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
            parts.create_bolt_group(&unknown, &bounds).unwrap_err().code,
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
                .create_bolt_group(&assembly, &bounds)
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
            parts.create_bolt_group(&other, &bounds).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        // The update decision reads before it places and names its target.
        let edit = UpdateBoltGroup {
            target_id: id(9),
            inputs: group_inputs(),
        };
        assert_eq!(
            parts.update_bolt_group(&edit, &bounds).unwrap_err().code,
            ErrorCode::NotFound
        );
        let mut with_group = crate::engine::weld::Parts::default();
        with_group.insert(id(1), PartKind::Part);
        with_group.insert(id(2), PartKind::Part);
        with_group.insert(id(9), PartKind::Other);
        let (placed, footprint) = with_group.update_bolt_group(&edit, &bounds).unwrap();
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
}
