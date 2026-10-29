// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A13 fabrication operations as owned A-2 operation records: line and polygon cuts, a cut by
//! another part, fitting an end to another part, chamfers, split and combine.
//!
//! Everything reuses the two certified kernel operations. A line cut, a fit and a chamfer
//! derive one cutting plane at admission and store an ordinary `plane_cut` record (A03); a
//! polygon cut or a cut by a part stores a through-cut record (A04 form, kind `polygon_cut`)
//! replayed by the kernel's certified prism subtraction. Split appends the matching `plane_cut`
//! record to each half and creates the second element; combine extends the survivor member over
//! the union of the two collinear axis ranges and deletes the consumed element, with the effect
//! carried by the resulting authored state and the revision ledger.
//!
//! Derived references are resolved once, at admission: the stored record never points at the
//! tool element, so moving the tool afterwards never updates the cut. Feature stability is the
//! kernel's; one accepted command is one revision and one undo unit, and a refusal changes
//! nothing.

use crate::command::AuthoredPlate;
use crate::copy::{copied_hole, copied_records};
use crate::cut::{CutByPlaneOutcome, CutTarget, admit_record};
use crate::footprint::{Entry, Footprint, Group, Target};
use crate::member::AuthoredMember;
use crate::solid::rotate_vector;
use beamcraft_model::command::{
    ChamferPart, CombineParts, CommandError, CutByLine, CutByPart, CutByPolygon, CutRecord,
    FitEndToPart, HoleKind, HoleRecord, KeepSide, MAX_HOLES, Placement, SplitPart,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_model::limits::CONTOUR_POINT_CAP;

/// How far the tool's mapped prism must reach past both faces of the target, in millimetres.
const TOOL_REACH_MM: f64 = 1e-6;

/// The next authored state of a cut or split target: exactly one of the two is set.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthoredPart {
    /// A plate with the record appended.
    Plate(AuthoredPlate),
    /// A beam or column with the record appended.
    Member(AuthoredMember),
}

impl AuthoredPart {
    /// The element the record was appended to.
    pub const fn id(&self) -> ElementId {
        match self {
            Self::Plate(authored) => authored.id,
            Self::Member(authored) => authored.id,
        }
    }
}

/// Everything one accepted `CutByPolygon` or `CutByPart` produces for the adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct PolygonCutOutcome {
    /// The target's authored state with the record appended.
    pub authored: AuthoredPlate,
    /// The ordinal the record took in the shared through-cut counter.
    pub ordinal: usize,
    /// Declared read and write sets.
    pub footprint: Footprint,
    /// Digest of the typed arguments, for replay and reuse decisions.
    pub fingerprint: ContentDigest,
}

/// Everything one accepted `SplitPart` produces for the adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitOutcome {
    /// The target's half, with the kept `plane_cut` record appended.
    pub kept: AuthoredPart,
    /// The new element's authored state, with the complementary record appended.
    pub created: AuthoredPart,
    /// The ordinal the split record took.
    pub ordinal: u16,
    /// Declared read and write sets.
    pub footprint: Footprint,
    /// Digest of the typed arguments, for replay and reuse decisions.
    pub fingerprint: ContentDigest,
}

/// Everything one accepted `CombineParts` produces for the adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct CombineOutcome {
    /// The survivor's authored state, extended over the union of the two axis ranges.
    pub survivor: AuthoredMember,
    /// Declared read and write sets.
    pub footprint: Footprint,
    /// Digest of the typed arguments, for replay and reuse decisions.
    pub fingerprint: ContentDigest,
}

/// Applies `CutByLine` to the target's authored state in memory: the derived plane is appended
/// as an ordinary `plane_cut` record. Nothing is evaluated here; the adapter re-evaluates the
/// body and refuses the whole command if the replay does.
pub fn cut_by_line(
    command: &CutByLine,
    plate: Option<&AuthoredPlate>,
    member: Option<&AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<CutByPlaneOutcome, CommandError> {
    let (point_mm, normal) = command.plane()?;
    append_plane_record(
        command.target_id,
        command.operation_id,
        point_mm,
        normal,
        command.keep,
        command.content_fingerprint()?,
        plate,
        member,
        operation_used,
        next_ordinal,
    )
}

/// Appends one derived `plane_cut` record after the shared admission, as `CutByPlane` does.
#[allow(clippy::too_many_arguments)]
fn append_plane_record(
    target_id: ElementId,
    operation_id: OpId,
    point_mm: [f64; 3],
    normal: [f64; 3],
    keep: KeepSide,
    fingerprint: ContentDigest,
    plate: Option<&AuthoredPlate>,
    member: Option<&AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<CutByPlaneOutcome, CommandError> {
    if next_ordinal > 999 {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "kernel_capacity",
            "cut ordinal capacity exhausted",
        ));
    }
    let (found, ordinal) = admit_record(target_id, operation_id, plate, member, operation_used)?;
    let record = CutRecord {
        operation_id,
        ordinal,
        point_mm,
        normal,
        keep,
    };
    let outcome = CutByPlaneOutcome {
        target: found.appended(record),
        ordinal,
        footprint: Footprint::for_update(target_id, false, false, true, false, None)?,
        fingerprint,
    };
    crate::cut::pin_next_ordinal(outcome, next_ordinal)
}

/// Admits a through-cut polygon on a plate: the shared record identity, the `MAX_HOLES`
/// capacity of the element, the admitted persistent next ordinal of the shared through-cut
/// counter and the declared footprint (the shape rewrite plus the read of a tool element when
/// there is one).
fn admit_through_polygon(
    head: &AuthoredPlate,
    operation_id: OpId,
    polygon: Vec<[f64; 2]>,
    kind: HoleKind,
    tool_id: Option<ElementId>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<(AuthoredPlate, usize, Footprint), CommandError> {
    if next_ordinal > 999 {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "kernel_capacity",
            "cut ordinal capacity exhausted",
        ));
    }
    if operation_used(&operation_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "operation_id",
            "operation record id is already owned",
        ));
    }
    if head.holes_mm.len().saturating_add(head.hole_records.len()) >= MAX_HOLES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "hole",
            "more than 64 through cuts on one plate",
        )
        .with_rule("DV-LIM-001"));
    }
    let derived = head
        .hole_records
        .last()
        .and_then(|last| last.ordinal.checked_add(1))
        .unwrap_or(0);
    if usize::from(next_ordinal) < derived {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "next_ordinal",
            "cut counter regressed",
        ));
    }
    let mut footprint = Footprint::for_update(head.id, false, false, true, false, None)?;
    if let Some(tool_id) = tool_id {
        let read = Footprint::from_entries(
            &[Entry {
                target: Target::Element(tool_id),
                group: Group::ShapeInputs,
            }],
            &[],
        )?;
        footprint = footprint.union(read)?;
    }
    let mut next = head.clone();
    next.hole_records.push(HoleRecord {
        operation_id,
        ordinal: usize::from(next_ordinal),
        polygon_mm: polygon,
        kind,
    });
    Ok((next, usize::from(next_ordinal), footprint))
}

/// Applies `CutByPolygon` to the plate's authored state in memory: the polygon is appended as a
/// through-cut record. The kernel decides at replay whether the polygon is clockwise, simple,
/// strictly inside the outline and clear of the other records; a refusal refuses the whole
/// command and changes nothing.
pub fn cut_by_polygon(
    command: &CutByPolygon,
    plate: Option<&AuthoredPlate>,
    member: Option<&AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<PolygonCutOutcome, CommandError> {
    command.validate_arguments()?;
    let head = found_plate(plate, member, command.target_id)?;
    let (authored, ordinal, footprint) = admit_through_polygon(
        head,
        command.operation_id,
        command.polygon()?,
        HoleKind::PolygonCut,
        None,
        operation_used,
        next_ordinal,
    )?;
    Ok(PolygonCutOutcome {
        authored,
        ordinal,
        footprint,
        fingerprint: command.content_fingerprint()?,
    })
}

/// The plate with the given identity, or the typed refusal: a member target is the documented
/// `UNSUPPORTED_GEOMETRY` (through-cuts subtract Z-prisms), an absent one is `NOT_FOUND`.
fn found_plate<'a>(
    plate: Option<&'a AuthoredPlate>,
    member: Option<&'a AuthoredMember>,
    target_id: ElementId,
) -> Result<&'a AuthoredPlate, CommandError> {
    if member.is_some_and(|authored| authored.id == target_id) {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "target_id",
            "a member target is not supported for this cut",
        ));
    }
    plate.filter(|head| head.id == target_id).ok_or_else(|| {
        CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "the element is not in the head revision",
        )
    })
}

/// Applies `CutByPart`: the tool plate's prism, mapped into the target's local frame, becomes a
/// through-cut record. The tool must lie parallel to the target (a relative rotation about the
/// target normal) and its mapped prism must reach past both target faces; everything else is
/// the kernel's replay decision.
pub fn cut_by_part(
    command: &CutByPart,
    plate: Option<&AuthoredPlate>,
    member: Option<&AuthoredMember>,
    tool: Option<&AuthoredPlate>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<PolygonCutOutcome, CommandError> {
    command.validate_arguments()?;
    let head = found_plate(plate, member, command.target_id)?;
    let tool = tool
        .filter(|tool| tool.id == command.tool_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "tool_id",
                "the tool is not a plate in the head revision",
            )
        })?;
    if tool.id == head.id {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "tool_id",
            "a plate cannot cut with itself",
        ));
    }
    let (polygon, crosses) = tool_cavity(head, tool)?;
    if !crosses {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "tool_id",
            "the tool does not cross the target",
        ));
    }
    let (authored, ordinal, footprint) = admit_through_polygon(
        head,
        command.operation_id,
        polygon,
        HoleKind::PolygonCut,
        Some(command.tool_id),
        operation_used,
        next_ordinal,
    )?;
    Ok(PolygonCutOutcome {
        authored,
        ordinal,
        footprint,
        fingerprint: command.content_fingerprint()?,
    })
}

/// The tool plate's outline mapped into the target's local XY (clockwise, as the through-cut
/// admission requires) and whether its mapped prism reaches past both target faces. A tool
/// whose relative rotation is not about the target normal is refused.
fn tool_cavity(
    target: &AuthoredPlate,
    tool: &AuthoredPlate,
) -> Result<(Vec<[f64; 2]>, bool), CommandError> {
    let relative = quat_multiply(
        quat_conjugate(target.placement.orientation_xyzw),
        tool.placement.orientation_xyzw,
    );
    let axis = rotate_vector([0.0, 0.0, 1.0], relative);
    if !(axis[0].abs() <= 1e-6 && axis[1].abs() <= 1e-6) {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "tool_id",
            "the tool must be parallel to the target plate",
        ));
    }
    let flipped = axis[2] < 0.0;
    let mut polygon: Vec<[f64; 2]> = tool
        .contour_points_mm
        .iter()
        .map(|[x, y, _]| {
            let world = local_to_world([*x, *y, 0.0], &tool.placement);
            let local = world_to_local(world, &target.placement);
            [local[0], local[1]]
        })
        .collect();
    if polygon.len() > CONTOUR_POINT_CAP {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "tool_id",
            "the tool contour exceeds the point cap",
        )
        .with_rule("DV-LIM-001"));
    }
    // The tool contour is counter-clockwise; the target's cutters are clockwise. A relative
    // rotation with a positive normal component preserves the XY orientation, so the mapped
    // polygon must be reversed; a flipped one is already clockwise.
    if !flipped {
        polygon.reverse();
    }
    let offset = world_to_local(tool.placement.translation_mm, &target.placement);
    let far = offset[2] + tool.thickness_mm * if flipped { -1.0 } else { 1.0 };
    let (from, to) = if offset[2] <= far {
        (offset[2], far)
    } else {
        (far, offset[2])
    };
    let crosses = from < -TOOL_REACH_MM && to > target.thickness_mm + TOOL_REACH_MM;
    Ok((polygon, crosses))
}

/// Applies `FitEndToPart`: the target is trimmed flush with the tool face nearest the target's
/// extent midpoint, keeping the side that midpoint lies on. The derived plane is appended as an
/// ordinary `plane_cut` record; whether it actually crosses the target is the kernel's replay
/// decision.
pub fn fit_end_to_part(
    command: &FitEndToPart,
    plate: Option<&AuthoredPlate>,
    member: Option<&AuthoredMember>,
    tool: Option<&AuthoredPlate>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<CutByPlaneOutcome, CommandError> {
    command.validate_arguments()?;
    if plate.filter(|p| p.id == command.target_id).is_none()
        && member.filter(|m| m.id == command.target_id).is_none()
    {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "the element is not in the head revision",
        ));
    }
    let tool = tool
        .filter(|tool| tool.id == command.tool_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "tool_id",
                "the tool is not a plate in the head revision",
            )
        })?;
    if plate.filter(|p| p.id == command.tool_id).is_some() {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "tool_id",
            "a part cannot be fitted to itself",
        ));
    }
    // The target's extent midpoint decides the nearer tool face: a plate's contour centre at
    // half the thickness, a member's axis midpoint.
    let target = match (plate, member) {
        (Some(authored), _) => {
            let (cx, cy) = bbox_center(authored.contour_points_mm.iter().map(|p| [p[0], p[1]]));
            let reference = [cx, cy, authored.thickness_mm / 2.0];
            Some((
                &authored.placement,
                authored.placement.orientation_xyzw,
                reference,
            ))
        }
        (_, Some(authored)) => {
            // A fit of a polybeam end is a later slice; the straight midpoint decides.
            let (Some(p1), Some(p2)) = (
                authored.axis_points_mm.first(),
                authored.axis_points_mm.last(),
            ) else {
                return Err(CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "target_id",
                    "the member has no axis points",
                ));
            };
            let reference = [
                (p1[0] + p2[0]) / 2.0,
                (p1[1] + p2[1]) / 2.0,
                (p1[2] + p2[2]) / 2.0,
            ];
            Some((
                &authored.placement,
                authored.placement.orientation_xyzw,
                reference,
            ))
        }
        _ => None,
    };
    let (point_mm, normal, keep) = fit_plane(target, tool)?;
    let mut footprint = Footprint::for_update(command.target_id, false, false, true, false, None)?;
    // The fit reads the tool's shape inputs: its faces decide the derived plane.
    footprint = footprint.union(Footprint::from_entries(
        &[Entry {
            target: Target::Element(command.tool_id),
            group: Group::ShapeInputs,
        }],
        &[],
    )?)?;
    append_plane_record(
        command.target_id,
        command.operation_id,
        point_mm,
        normal,
        keep,
        command.content_fingerprint()?,
        plate,
        member,
        operation_used,
        next_ordinal,
    )
    .map(|mut outcome| {
        outcome.footprint = footprint;
        outcome
    })
}

/// The tool face plane nearest the target's extent midpoint, in target-local coordinates, with
/// the kept side the midpoint lies on. The target's placement is the one of the given state.
fn fit_plane(
    target: Option<(&Placement, [f64; 4], [f64; 3])>,
    tool: &AuthoredPlate,
) -> Result<([f64; 3], [f64; 3], KeepSide), CommandError> {
    let (target_placement, target_orientation, reference) = target.ok_or_else(|| {
        CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "the element is not in the head revision",
        )
    })?;
    let (face_a, face_b, normal_world) = tool_faces(tool);
    // The reference in world coordinates; both tool faces and the normal live there too.
    let reference_world = local_to_world(reference, target_placement);
    let distance = |face: [f64; 3]| {
        (reference_world[0] - face[0]) * normal_world[0]
            + (reference_world[1] - face[1]) * normal_world[1]
            + (reference_world[2] - face[2]) * normal_world[2]
    };
    let (near, signed) = {
        let (a, b) = (distance(face_a), distance(face_b));
        if a.abs() <= b.abs() {
            (face_a, a)
        } else {
            (face_b, b)
        }
    };
    let point_mm = world_to_local(near, target_placement);
    let normal = rotate_vector(normal_world, quat_conjugate(target_orientation));
    Ok((
        point_mm,
        normal,
        if signed >= 0.0 {
            KeepSide::Positive
        } else {
            KeepSide::Negative
        },
    ))
}

/// The two tool face plane points (the contour centre at each face) and the face normal in
/// world coordinates.
fn tool_faces(tool: &AuthoredPlate) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let (cx, cy) = bbox_center(tool.contour_points_mm.iter().map(|p| [p[0], p[1]]));
    let normal = rotate_vector([0.0, 0.0, 1.0], tool.placement.orientation_xyzw);
    (
        local_to_world([cx, cy, 0.0], &tool.placement),
        local_to_world([cx, cy, tool.thickness_mm], &tool.placement),
        normal,
    )
}

/// The centre of the axis-aligned bounding box of the given planar points.
fn bbox_center(points: impl Iterator<Item = [f64; 2]>) -> (f64, f64) {
    let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in points {
        lo_x = lo_x.min(p[0]);
        lo_y = lo_y.min(p[1]);
        hi_x = hi_x.max(p[0]);
        hi_y = hi_y.max(p[1]);
    }
    ((lo_x + hi_x) / 2.0, (lo_y + hi_y) / 2.0)
}

/// Applies `ChamferPart`: the cutting plane runs through the two points one chamfer distance
/// back along each contour edge at the vertex, and the corner side is removed. The derived
/// plane is appended as an ordinary `plane_cut` record.
pub fn chamfer_part(
    command: &ChamferPart,
    plate: Option<&AuthoredPlate>,
    member: Option<&AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<CutByPlaneOutcome, CommandError> {
    command.validate_arguments()?;
    let head = found_plate(plate, member, command.target_id)?;
    let points = &head.contour_points_mm;
    let count = points.len();
    if count < 3 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "contour_points_mm",
            "a chamfer needs a contour of at least three points",
        ));
    }
    let vertex = points.get(command.vertex_index).copied().ok_or_else(|| {
        CommandError::new(
            ErrorCode::NotFound,
            "vertex_index",
            "the contour has no vertex at the index",
        )
    })?;
    let previous = points
        .get((command.vertex_index + count - 1) % count)
        .copied()
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "vertex_index",
                "the contour has no vertex at the index",
            )
        })?;
    let next = points
        .get((command.vertex_index + 1) % count)
        .copied()
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "vertex_index",
                "the contour has no vertex at the index",
            )
        })?;
    let toward =
        |from: [f64; 3], distance: f64, field: &'static str| -> Result<[f64; 3], CommandError> {
            let edge = [
                from[0] - vertex[0],
                from[1] - vertex[1],
                from[2] - vertex[2],
            ];
            let length = (edge[0] * edge[0] + edge[1] * edge[1] + edge[2] * edge[2]).sqrt();
            if length <= 0.0 || distance >= length {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    field,
                    "the chamfer distance must be shorter than the edge",
                ));
            }
            Ok([
                vertex[0] + edge[0] * distance / length,
                vertex[1] + edge[1] * distance / length,
                vertex[2] + edge[2] * distance / length,
            ])
        };
    let first = toward(previous, command.first_distance_mm, "first_distance_mm")?;
    let second = toward(next, command.second_distance_mm, "second_distance_mm")?;
    let edge = [
        second[0] - first[0],
        second[1] - first[1],
        second[2] - first[2],
    ];
    // The plane through the two edge points and the plate normal (element-local Z).
    let mut normal = [edge[1], -edge[0], 0.0];
    let length = (normal[0] * normal[0] + normal[1] * normal[1]).sqrt();
    if length <= 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "contour_points_mm",
            "the chamfer edges are degenerate",
        ));
    }
    normal = [normal[0] / length, normal[1] / length, 0.0];
    // Keep the far side: the vertex itself must fall into the removed half-space.
    let side = (vertex[0] - first[0]) * normal[0] + (vertex[1] - first[1]) * normal[1];
    if side == 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "contour_points_mm",
            "the cutting plane passes through the vertex",
        ));
    }
    let keep = if side > 0.0 {
        KeepSide::Negative
    } else {
        KeepSide::Positive
    };
    append_plane_record(
        command.target_id,
        command.operation_id,
        first,
        normal,
        keep,
        command.content_fingerprint()?,
        plate,
        None,
        operation_used,
        next_ordinal,
    )
}

/// Applies `SplitPart`: the target keeps its half and gains the matching `plane_cut` record;
/// the new element receives a copy of the target's authored state with fresh record identities
/// and the complementary cut. Nothing is evaluated here; the adapter re-evaluates both halves
/// and refuses the whole command if either replay does.
pub fn split_part(
    command: &SplitPart,
    plate: Option<&AuthoredPlate>,
    member: Option<&AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
    id_used: &dyn Fn(&ElementId) -> bool,
    next_ordinal: u16,
) -> Result<SplitOutcome, CommandError> {
    command.validate_arguments()?;
    if next_ordinal > 999 {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "kernel_capacity",
            "cut ordinal capacity exhausted",
        ));
    }
    if id_used(&command.new_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "new_id",
            "element id is already owned",
        ));
    }
    let (found, ordinal) = admit_record(
        command.target_id,
        command.operation_id,
        plate,
        member,
        operation_used,
    )?;
    if next_ordinal < ordinal {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "next_ordinal",
            "cut counter regressed",
        ));
    }
    let ordinal = next_ordinal;
    let kept_record = CutRecord {
        operation_id: command.operation_id,
        ordinal,
        point_mm: command.point_mm,
        normal: command.normal,
        keep: command.keep,
    };
    let flip = |keep| match keep {
        KeepSide::Positive => KeepSide::Negative,
        KeepSide::Negative => KeepSide::Positive,
    };
    let other_record = CutRecord {
        keep: flip(command.keep),
        ..kept_record
    };
    let (kept, created) = match found {
        crate::cut::Found::Plate(authored) => {
            let mut other_cuts = copied_records(&authored.cuts, &command.new_id)?;
            other_cuts.extend(copied_records(
                core::slice::from_ref(&other_record),
                &command.new_id,
            )?);
            let mut created = authored.clone();
            created.id = command.new_id;
            created.name = command.new_name.clone();
            created.hole_records = authored
                .hole_records
                .iter()
                .map(|record| copied_hole(record, &command.new_id))
                .collect::<Result<Vec<_>, _>>()?;
            created.cuts = other_cuts;
            let kept = crate::cut::Found::Plate(authored).appended(kept_record);
            let CutTarget::Plate(kept) = kept else {
                return Err(CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "target_id",
                    "the split target changed kind",
                ));
            };
            (AuthoredPart::Plate(kept), AuthoredPart::Plate(created))
        }
        crate::cut::Found::Member(authored) => {
            let mut other_cuts = copied_records(&authored.cuts, &command.new_id)?;
            other_cuts.extend(copied_records(
                core::slice::from_ref(&other_record),
                &command.new_id,
            )?);
            let mut created = authored.clone();
            created.id = command.new_id;
            created.name = command.new_name.clone();
            created.cuts = other_cuts;
            let kept = crate::cut::Found::Member(authored).appended(kept_record);
            let CutTarget::Member(kept) = kept else {
                return Err(CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "target_id",
                    "the split target changed kind",
                ));
            };
            (AuthoredPart::Member(kept), AuthoredPart::Member(created))
        }
    };
    // A creation writes lifecycle and every authored group; the target keeps the split record.
    let mut footprint = Footprint::for_update(command.target_id, false, false, true, false, None)?;
    let material = match &kept {
        AuthoredPart::Plate(authored) => authored.material,
        AuthoredPart::Member(authored) => authored.material,
    };
    let create = match &created {
        AuthoredPart::Plate(_) => Footprint::for_create(command.new_id, material)?,
        AuthoredPart::Member(authored) => {
            Footprint::for_create_member(command.new_id, authored.profile, material)?
        }
    };
    footprint = footprint.union(create)?;
    Ok(SplitOutcome {
        kept,
        created,
        ordinal,
        footprint,
        fingerprint: command.content_fingerprint()?,
    })
}

/// Applies `CombineParts`: the survivor member extends over the union of the two world axis
/// ranges and the consumed member is deleted. The two members need the same pinned profile,
/// orientation, roll and material, collinear axes and touching or overlapping ranges; a
/// consumed member with cut records is refused (its records would be lost).
pub fn combine_parts(
    command: &CombineParts,
    survivor: Option<&AuthoredMember>,
    consumed: Option<&AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
) -> Result<CombineOutcome, CommandError> {
    command.validate_arguments()?;
    if operation_used(&command.operation_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "operation_id",
            "operation record id is already owned",
        ));
    }
    let head = survivor
        .filter(|authored| authored.id == command.survivor_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "survivor_id",
                "the element is not in the head revision",
            )
        })?;
    let other = consumed
        .filter(|authored| authored.id == command.consumed_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "consumed_id",
                "the element is not in the head revision",
            )
        })?;
    if head.profile != other.profile {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the two members have different pinned profiles",
        ));
    }
    // A combine extends one straight axis; polybeam paths are a later slice.
    if head.axis_points_mm.len() != 2 || other.axis_points_mm.len() != 2 {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "consumed_id",
            "combining a polybeam is not supported",
        ));
    }
    if head.material != other.material {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the two members have different materials",
        ));
    }
    if head.roll_rad != other.roll_rad {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the two members have different rolls",
        ));
    }
    if !same_rotation(
        head.placement.orientation_xyzw,
        other.placement.orientation_xyzw,
    ) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the two members have different orientations",
        ));
    }
    if head.section_placement != other.section_placement {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the two members have different section placements",
        ));
    }
    if !other.cuts.is_empty() {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the consumed member has cut records",
        ));
    }
    // The straight-axis admission above guarantees exactly two points per path.
    let unwrap_point = |path: &[[f64; 3]], at: usize| path.get(at).copied().unwrap_or([0.0; 3]);
    let (p1, p2) = (
        local_to_world(unwrap_point(&head.axis_points_mm, 0), &head.placement),
        local_to_world(unwrap_point(&head.axis_points_mm, 1), &head.placement),
    );
    let (c1, c2) = (
        local_to_world(unwrap_point(&other.axis_points_mm, 0), &other.placement),
        local_to_world(unwrap_point(&other.axis_points_mm, 1), &other.placement),
    );
    let axis = [p2[0] - p1[0], p2[1] - p1[1], p2[2] - p1[2]];
    let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if length <= 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "axis_points_mm",
            "the survivor axis is degenerate",
        ));
    }
    let direction = [axis[0] / length, axis[1] / length, axis[2] / length];
    let other_axis = [c2[0] - c1[0], c2[1] - c1[1], c2[2] - c1[2]];
    let other_length = (other_axis[0] * other_axis[0]
        + other_axis[1] * other_axis[1]
        + other_axis[2] * other_axis[2])
        .sqrt();
    if other_length <= 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the consumed axis is degenerate",
        ));
    }
    let other_direction = [
        other_axis[0] / other_length,
        other_axis[1] / other_length,
        other_axis[2] / other_length,
    ];
    let aligned = direction[0] * other_direction[0]
        + direction[1] * other_direction[1]
        + direction[2] * other_direction[2];
    if aligned.abs() < 1.0 - 1e-6 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the two axes are not collinear",
        ));
    }
    // Parallel is not enough: the consumed axis must lie on the survivor's line, so the
    // offset between the two start points may have no component across the direction.
    let offset = [c1[0] - p1[0], c1[1] - p1[1], c1[2] - p1[2]];
    let along = offset[0] * direction[0] + offset[1] * direction[1] + offset[2] * direction[2];
    let across = (offset[0] - direction[0] * along).powi(2)
        + (offset[1] - direction[1] * along).powi(2)
        + (offset[2] - direction[2] * along).powi(2);
    if across.sqrt() > 1e-6 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the two axes are parallel but not on one line",
        ));
    }
    let start = along;
    let end = start + other_length * aligned;
    let (low, high) = (start.min(end), start.max(end));
    if low > length || high < 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "consumed_id",
            "the two members do not overlap or touch",
        ));
    }
    let shift = low.min(0.0);
    let combined = high.max(length) - shift;
    let translation = [
        head.placement.translation_mm[0] + direction[0] * shift,
        head.placement.translation_mm[1] + direction[1] * shift,
        head.placement.translation_mm[2] + direction[2] * shift,
    ];
    let placement = Placement {
        translation_mm: translation,
        orientation_xyzw: head.placement.orientation_xyzw,
    };
    placement.validate()?;
    // The local axis direction is the survivor's own; the straight-axis admission above
    // guarantees the two points.
    let (local_start, local_end) = (
        unwrap_point(&head.axis_points_mm, 0),
        unwrap_point(&head.axis_points_mm, 1),
    );
    let local = [
        local_end[0] - local_start[0],
        local_end[1] - local_start[1],
        local_end[2] - local_start[2],
    ];
    let unit = [local[0] / length, local[1] / length, local[2] / length];
    let mut next = head.clone();
    next.placement = placement;
    next.axis_points_mm = vec![
        local_start,
        [
            local_start[0] + unit[0] * combined,
            local_start[1] + unit[1] * combined,
            local_start[2] + unit[2] * combined,
        ],
    ];
    let footprint = Footprint::for_update(command.survivor_id, false, false, true, true, None)?
        .union(Footprint::for_delete(command.consumed_id)?)?;
    Ok(CombineOutcome {
        survivor: next,
        footprint,
        fingerprint: command.content_fingerprint()?,
    })
}

/// Whether two admitted quaternions describe the same rotation (possibly up to the sign).
fn same_rotation(a: [f64; 4], b: [f64; 4]) -> bool {
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    dot.abs() >= 1.0 - 1e-6
}

/// The element-local point in world coordinates.
fn local_to_world(point: [f64; 3], placement: &Placement) -> [f64; 3] {
    let rotated = rotate_vector(point, placement.orientation_xyzw);
    [
        rotated[0] + placement.translation_mm[0],
        rotated[1] + placement.translation_mm[1],
        rotated[2] + placement.translation_mm[2],
    ]
}

/// The world point in element-local coordinates.
fn world_to_local(point: [f64; 3], placement: &Placement) -> [f64; 3] {
    let relative = [
        point[0] - placement.translation_mm[0],
        point[1] - placement.translation_mm[1],
        point[2] - placement.translation_mm[2],
    ];
    rotate_vector(relative, quat_conjugate(placement.orientation_xyzw))
}

/// The conjugate quaternion (the inverse rotation).
fn quat_conjugate(q: [f64; 4]) -> [f64; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// The Hamilton product `a * b`: rotate by `b` first, then by `a`.
fn quat_multiply(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::evaluate_plate;
    use beamcraft_model::digest::Sha256;

    fn id(byte: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{byte:02x}")).unwrap()
    }

    fn op(byte: u8) -> OpId {
        OpId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{byte:02x}")).unwrap()
    }

    fn digest(tag: &[u8]) -> ContentDigest {
        Sha256::digest(tag).unwrap()
    }

    /// A `1000 x 500 x 10` plate at the origin rectangle contour.
    fn plate() -> AuthoredPlate {
        AuthoredPlate {
            id: id(5),
            name: "Plate".into(),
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material: None,
            class: 0,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    /// A `w x l` tool plate with the given thickness and placement.
    fn tool(id_byte: u8, w: f64, l: f64, t: f64, placement: Placement) -> AuthoredPlate {
        AuthoredPlate {
            id: id(id_byte),
            name: "Tool".into(),
            contour_points_mm: vec![[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, l, 0.0], [0.0, l, 0.0]],
            thickness_mm: t,
            thickness_side: None,
            corners: Vec::new(),
            placement,
            material: None,
            class: 0,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn member(
        id_byte: u8,
        length: f64,
        translation: [f64; 3],
        profile: ContentDigest,
    ) -> AuthoredMember {
        AuthoredMember {
            id: id(id_byte),
            kind: beamcraft_model::command::MemberKind::Beam,
            name: "Member".into(),
            class: 0,
            axis_points_mm: vec![[0.0, 0.0, 0.0], [length, 0.0, 0.0]],
            curved: false,
            profile,
            roll_rad: None,
            section_placement: None,
            placement: Placement {
                translation_mm: translation,
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
            material: None,
            cuts: Vec::new(),
        }
    }

    fn none(_: &OpId) -> bool {
        false
    }

    fn no_id(_: &ElementId) -> bool {
        false
    }

    /// A clockwise square in the plate's local XY.
    fn square(x: f64, y: f64, side: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x, y + side], [x + side, y + side], [x + side, y]]
    }

    #[test]
    fn a_line_cut_replays_as_the_derived_plane() {
        let source = plate();
        let command = CutByLine {
            target_id: id(5),
            operation_id: op(200),
            point_a_mm: [0.0, 250.0, 0.0],
            point_b_mm: [1000.0, 250.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            keep: KeepSide::Positive,
        };
        let outcome = cut_by_line(&command, Some(&source), None, &none, 0).unwrap();
        let CutTarget::Plate(authored) = &outcome.target else {
            panic!("expected the plate target");
        };
        assert_eq!(authored.cuts.len(), 1);
        assert_eq!(outcome.ordinal, 0);
        // The plane contains the line and Z, so it keeps `y >= 250`.
        let body = evaluate_plate(authored).unwrap();
        assert!((body.quantities().volume_mm3 - 2_500_000.0).abs() <= 1e-6);
        // A parallel axis is refused before anything is appended.
        let parallel = CutByLine {
            axis: [1.0, 0.0, 0.0],
            ..command
        };
        assert_eq!(
            cut_by_line(&parallel, Some(&source), None, &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            cut_by_line(&command, Some(&source), None, &|o: &OpId| *o == op(200), 0)
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
    }

    #[test]
    fn a_polygon_cut_appends_a_through_cut_record() {
        let source = plate();
        let command = CutByPolygon {
            target_id: id(5),
            operation_id: op(201),
            polygon_mm: square(100.0, 100.0, 100.0),
        };
        let outcome = cut_by_polygon(&command, Some(&source), None, &none, 0).unwrap();
        assert_eq!(outcome.ordinal, 0);
        // A member target is the documented unsupported geometry, never a missing element.
        let beam = member(6, 6000.0, [0.0; 3], digest(b"profile"));
        let on_member = CutByPolygon {
            target_id: id(6),
            ..command.clone()
        };
        assert_eq!(
            cut_by_polygon(&on_member, Some(&source), Some(&beam), &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        assert_eq!(outcome.authored.hole_records[0].kind, HoleKind::PolygonCut);
        let body = evaluate_plate(&outcome.authored).unwrap();
        assert!((body.quantities().volume_mm3 - 4_900_000.0).abs() <= 1e-6);
        // A second record takes the next ordinal of the shared through-cut counter.
        let next = CutByPolygon {
            operation_id: op(202),
            polygon_mm: square(300.0, 100.0, 100.0),
            ..command.clone()
        };
        assert_eq!(
            cut_by_polygon(&next, Some(&outcome.authored), None, &none, 1)
                .unwrap()
                .ordinal,
            1
        );
        // A polygon crossing the outline is refused by the replay, leaving nothing written.
        let crossing = CutByPolygon {
            operation_id: op(203),
            polygon_mm: square(900.0, 100.0, 200.0),
            ..command.clone()
        };
        let attempted = cut_by_polygon(&crossing, Some(&source), None, &none, 0).unwrap();
        assert!(evaluate_plate(&attempted.authored).is_err());
    }

    #[test]
    fn a_cut_by_part_maps_the_tool_prism_into_the_target_frame() {
        let source = plate();
        let cutter = tool(
            6,
            100.0,
            100.0,
            20.0,
            Placement {
                translation_mm: [150.0, 150.0, -5.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
        );
        let command = CutByPart {
            target_id: id(5),
            operation_id: op(204),
            tool_id: id(6),
        };
        let outcome = cut_by_part(&command, Some(&source), None, Some(&cutter), &none, 0).unwrap();
        assert_eq!(outcome.ordinal, 0);
        let body = evaluate_plate(&outcome.authored).unwrap();
        assert!((body.quantities().volume_mm3 - 4_900_000.0).abs() <= 1e-6);
        // The tool's footprint read is declared.
        assert!(
            outcome
                .footprint
                .reads()
                .iter()
                .any(|e| e.target == Target::Element(id(6)) && e.group == Group::ShapeInputs)
        );
        // A tool that does not reach past both faces is refused.
        let shallow = tool(
            6,
            100.0,
            100.0,
            20.0,
            Placement {
                translation_mm: [150.0, 150.0, 5.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
        );
        assert_eq!(
            cut_by_part(&command, Some(&source), None, Some(&shallow), &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        // A tool turned across the target is refused.
        let sideways = AuthoredPlate {
            placement: Placement {
                translation_mm: [150.0, 150.0, -5.0],
                orientation_xyzw: [
                    0.0,
                    (std::f64::consts::FRAC_PI_4).sin(),
                    0.0,
                    (std::f64::consts::FRAC_PI_4).cos(),
                ],
            },
            ..cutter.clone()
        };
        assert_eq!(
            cut_by_part(&command, Some(&source), None, Some(&sideways), &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
    }

    #[test]
    fn a_fit_trims_the_target_at_the_nearest_tool_face() {
        let source = plate();
        let stiffener = tool(
            6,
            200.0,
            200.0,
            20.0,
            Placement {
                translation_mm: [100.0, 100.0, 5.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
        );
        let command = FitEndToPart {
            target_id: id(5),
            operation_id: op(205),
            tool_id: id(6),
        };
        let outcome =
            fit_end_to_part(&command, Some(&source), None, Some(&stiffener), &none, 0).unwrap();
        let CutTarget::Plate(authored) = &outcome.target else {
            panic!("expected the plate target");
        };
        // The near face lies at `z = 5`, and the midpoint side is kept.
        let body = evaluate_plate(authored).unwrap();
        assert!((body.quantities().volume_mm3 - 2_500_000.0).abs() <= 1e-6);
        // Without the tool there is nothing to fit to.
        assert_eq!(
            fit_end_to_part(&command, Some(&source), None, None, &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn a_chamfer_removes_the_corner_triangle() {
        let source = plate();
        let command = ChamferPart {
            target_id: id(5),
            operation_id: op(206),
            vertex_index: 0,
            first_distance_mm: 100.0,
            second_distance_mm: 100.0,
        };
        let outcome = chamfer_part(&command, Some(&source), None, &none, 0).unwrap();
        let CutTarget::Plate(authored) = &outcome.target else {
            panic!("expected the plate target");
        };
        let body = evaluate_plate(authored).unwrap();
        assert!((body.quantities().volume_mm3 - 4_950_000.0).abs() <= 1e-6);
        // A distance past the edge is refused.
        let long = ChamferPart {
            first_distance_mm: 500.0,
            ..command
        };
        assert_eq!(
            chamfer_part(&long, Some(&source), None, &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // An out-of-range vertex is a not-found result.
        let missing = ChamferPart {
            vertex_index: 9,
            ..command
        };
        assert_eq!(
            chamfer_part(&missing, Some(&source), None, &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn a_split_appends_the_matching_records_to_both_halves() {
        let source = plate();
        let command = SplitPart {
            target_id: id(5),
            new_id: id(7),
            operation_id: op(207),
            new_name: "Other half".into(),
            point_mm: [250.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        };
        let outcome = split_part(&command, Some(&source), None, &none, &no_id, 0).unwrap();
        let AuthoredPart::Plate(kept) = &outcome.kept else {
            panic!("expected the kept plate");
        };
        let AuthoredPart::Plate(created) = &outcome.created else {
            panic!("expected the created plate");
        };
        assert_eq!(created.id, id(7));
        assert_eq!(created.name, "Other half");
        let kept_volume = evaluate_plate(kept).unwrap().quantities().volume_mm3;
        let created_volume = evaluate_plate(created).unwrap().quantities().volume_mm3;
        assert!((kept_volume - 1_250_000.0).abs() <= 1e-6);
        assert!((created_volume - 3_750_000.0).abs() <= 1e-6);
        // The halves keep the full volume, and the created records carry fresh identities.
        assert!((kept_volume + created_volume - 5_000_000.0).abs() <= 1e-6);
        assert_ne!(created.cuts[0].operation_id, op(207));
        assert_eq!(created.cuts[0].ordinal, 0);
        // The new identity must be unused.
        assert_eq!(
            split_part(&command, Some(&source), None, &none, &|_| true, 0)
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
    }

    #[test]
    fn a_combine_extends_the_survivor_over_the_union_range() {
        let profile = digest(b"profile");
        let survivor = member(5, 6000.0, [0.0; 3], profile);
        let consumed = member(6, 4000.0, [5000.0, 0.0, 0.0], profile);
        let command = CombineParts {
            survivor_id: id(5),
            consumed_id: id(6),
            operation_id: op(208),
        };
        let outcome = combine_parts(&command, Some(&survivor), Some(&consumed), &none).unwrap();
        assert_eq!(outcome.survivor.axis_points_mm[1], [9000.0, 0.0, 0.0]);
        assert_eq!(outcome.survivor.placement.translation_mm, [0.0; 3]);
        // A consumed member extending past the start shifts the placement back.
        let before = member(6, 4000.0, [-3000.0, 0.0, 0.0], profile);
        let outcome = combine_parts(&command, Some(&survivor), Some(&before), &none).unwrap();
        assert_eq!(outcome.survivor.axis_points_mm[1], [9000.0, 0.0, 0.0]);
        assert_eq!(
            outcome.survivor.placement.translation_mm,
            [-3000.0, 0.0, 0.0]
        );
        // Different profiles, gaps and cut records are refused.
        let other = member(6, 4000.0, [5000.0, 0.0, 0.0], digest(b"other"));
        assert_eq!(
            combine_parts(&command, Some(&survivor), Some(&other), &none)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        let apart = member(6, 4000.0, [10000.0, 0.0, 0.0], profile);
        assert_eq!(
            combine_parts(&command, Some(&survivor), Some(&apart), &none)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // A parallel member offset across the survivor's line is refused, not combined.
        let offset = member(6, 4000.0, [5000.0, 300.0, 0.0], profile);
        assert_eq!(
            combine_parts(&command, Some(&survivor), Some(&offset), &none)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        let mut cut = member(6, 4000.0, [5000.0, 0.0, 0.0], profile);
        cut.cuts.push(CutRecord {
            operation_id: op(209),
            ordinal: 0,
            point_mm: [2000.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        });
        assert_eq!(
            combine_parts(&command, Some(&survivor), Some(&cut), &none)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }
}
