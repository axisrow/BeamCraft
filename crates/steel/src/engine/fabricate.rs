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

use crate::engine::command::AuthoredPlate;
use crate::engine::copy::{copied_hole, copied_records};
use crate::engine::cut::{CutByPlaneOutcome, CutTarget, admit_record};
use crate::engine::member::AuthoredMember;
use crate::model::command::{
    ChamferPart, CombineParts, CutByLine, CutByPart, CutByPolygon, CutRecord, FitEndToPart,
    HoleKind, HoleRecord, KeepSide, MAX_HOLES, SplitPart,
};
use beamcraft_engine::footprint::{Entry, Footprint, Group, Target};
use beamcraft_engine::solid::rotate_vector;
use beamcraft_model::command::{CommandError, Placement};
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
    /// The target's authored state with the record appended: a plate's through-cut record
    /// for the plate form, a member's record (a polygon cut or a part cut) for the member
    /// form.
    pub authored: AuthoredPart,
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
    crate::engine::cut::pin_next_ordinal(outcome, next_ordinal)
}

/// Admits a through-cut polygon on a plate: the shared record identity, the `MAX_HOLES`
/// capacity of the element, the admitted persistent next ordinal of the shared through-cut
/// counter and the declared footprint (the shape rewrite plus the read of a tool element when
/// there is one). `axis` is the record's prism axis in the plate frame; the vertical kinds
/// carry `(0, 0, 1)`.
#[allow(clippy::too_many_arguments)]
fn admit_through_polygon(
    head: &AuthoredPlate,
    operation_id: OpId,
    polygon: Vec<[f64; 2]>,
    kind: HoleKind,
    axis: [f64; 3],
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
        axis_direction_mm: axis,
        owner: None,
    });
    Ok((next, usize::from(next_ordinal), footprint))
}

/// Applies `CutByPolygon`: the plate form cuts a plate with the prism of the polygon in its
/// local XY; the member form (the member frame fields present) cuts a member with the prism of
/// the polygon in the member-hole frame along a section-plane direction — an owned record of
/// kind `polygon_cut` replayed by the certified member prism subtraction (epic #556 slice 1).
/// The kernel decides at replay whether the polygon is clockwise, simple, strictly inside the
/// walls or outline and clear of the other records; a refusal refuses the whole command and
/// changes nothing.
pub fn cut_by_polygon(
    command: &CutByPolygon,
    plate: Option<&AuthoredPlate>,
    member: Option<&AuthoredMember>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<PolygonCutOutcome, CommandError> {
    command.validate_arguments()?;
    let frame = command.member_frame()?;
    let head = found_cut_target(plate, member, command.target_id)?;
    let fingerprint = command.content_fingerprint()?;
    match (head, frame) {
        (CutHead::Plate(head), None) => {
            let (authored, ordinal, footprint) = admit_through_polygon(
                head,
                command.operation_id,
                command.polygon()?,
                HoleKind::PolygonCut,
                [0.0, 0.0, 1.0],
                None,
                operation_used,
                next_ordinal,
            )?;
            Ok(PolygonCutOutcome {
                authored: AuthoredPart::Plate(authored),
                ordinal,
                footprint,
                fingerprint,
            })
        }
        (CutHead::Member(head), Some((axis_point_mm, axis_direction))) => {
            let outcome = crate::engine::member_hole::admit_member_polygon_cut(
                command.target_id,
                command.operation_id,
                head,
                axis_point_mm,
                axis_direction,
                command.polygon()?,
                fingerprint,
                &|id: OpId| operation_used(&id),
                usize::from(next_ordinal),
            )?;
            Ok(PolygonCutOutcome {
                authored: AuthoredPart::Member(outcome.authored),
                ordinal: outcome.ordinal,
                footprint: outcome.footprint,
                fingerprint: outcome.fingerprint,
            })
        }
        // The frame fields must name the target's kind: a member form on a plate and a plate
        // form on a member are argument errors, not geometry ones.
        (CutHead::Plate(_), Some(_)) => Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "axis_point_mm",
            "a plate cut takes no --axis-point or --axis-direction",
        )),
        (CutHead::Member(_), None) => Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "axis_point_mm",
            "a member cut takes --axis-point and --axis-direction",
        )),
    }
}

/// The cut target resolved to its authored state: the plate with the given identity, or the
/// member with it. An absent element is `NOT_FOUND`; the form checks between the two kinds are
/// the caller's (`cut_by_polygon` pairs the target with the command's frame fields).
enum CutHead<'a> {
    Plate(&'a AuthoredPlate),
    Member(&'a AuthoredMember),
}

fn found_cut_target<'a>(
    plate: Option<&'a AuthoredPlate>,
    member: Option<&'a AuthoredMember>,
    target_id: ElementId,
) -> Result<CutHead<'a>, CommandError> {
    if let Some(head) = plate.filter(|head| head.id == target_id) {
        return Ok(CutHead::Plate(head));
    }
    if let Some(head) = member.filter(|head| head.id == target_id) {
        return Ok(CutHead::Member(head));
    }
    Err(CommandError::new(
        ErrorCode::NotFound,
        "target_id",
        "the element is not in the head revision",
    ))
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

/// Applies `CutByPart`: the tool plate's prism, mapped into the target's frame, becomes a
/// through-cut record. A plate target takes a `part_cut` record whose prism axis is the
/// mapped tool normal — any relative rotation is derived, and the mapped prism must reach
/// past both target faces. A member target takes a member `part_cut` record derived the
/// same way, its prism axis allowed to lean along the member (epic #556 slice 2); a purely
/// axial tool is refused. Everything else is the kernel's replay decision, and a refusal
/// changes nothing.
pub fn cut_by_part(
    command: &CutByPart,
    plate: Option<&AuthoredPlate>,
    member: Option<&AuthoredMember>,
    tool: Option<&AuthoredPlate>,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<PolygonCutOutcome, CommandError> {
    command.validate_arguments()?;
    let tool = tool
        .filter(|tool| tool.id == command.tool_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "tool_id",
                "the tool is not a plate in the head revision",
            )
        })?;
    if tool.id == command.target_id {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "tool_id",
            "a plate cannot cut with itself",
        ));
    }
    if let Some(head) = member.filter(|head| head.id == command.target_id) {
        return cut_member_by_part(command, head, tool, operation_used, next_ordinal);
    }
    let head = found_plate(plate, member, command.target_id)?;
    let (polygon, axis, crosses) = tool_prism(head, tool)?;
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
        HoleKind::PartCut,
        axis,
        Some(command.tool_id),
        operation_used,
        next_ordinal,
    )?;
    Ok(PolygonCutOutcome {
        authored: AuthoredPart::Plate(authored),
        ordinal,
        footprint,
        fingerprint: command.content_fingerprint()?,
    })
}

/// The member path of [`cut_by_part`]: the tool plate's prism mapped into the member frame
/// becomes an owned member `part_cut` record. The stored hole frame generalizes the A04.2
/// convention: its x is the member axis projected onto the plane perpendicular to the cut
/// direction and its y completes the right-handed frame, which reduces to
/// `(Z_m, axis_direction x Z_m)` for a section-plane direction. The derivation resolves
/// once at admission; moving the tool afterwards never updates the cut.
fn cut_member_by_part(
    command: &CutByPart,
    head: &AuthoredMember,
    tool: &AuthoredPlate,
    operation_used: &dyn Fn(&OpId) -> bool,
    next_ordinal: u16,
) -> Result<PolygonCutOutcome, CommandError> {
    if operation_used(&command.operation_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "operation_id",
            "operation id already names a record of the project",
        ));
    }
    if head.hole_records.len() >= MAX_HOLES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "hole",
            "more than 64 holes on one member",
        )
        .with_rule("DV-LIM-001"));
    }
    let ordinal =
        crate::engine::member_hole::admit_ordinal(&head.hole_records, usize::from(next_ordinal))?;
    // The member frame the replay serves: the same construction over the member's axis
    // points, so the stored record means the same geometry on both sides. The section
    // extents only feed validation; the axes come from the axis points and the roll.
    let (Some(start), Some(end)) = (head.axis_points_mm.first(), head.axis_points_mm.get(1)) else {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "axis_points_mm",
            "member frame could not be built",
        ));
    };
    let frame = beamcraft_geom::frame::MemberFrame::new(
        beamcraft_geom::Point3::new(start[0], start[1], start[2]),
        beamcraft_geom::Point3::new(end[0], end[1], end[2]),
        1.0,
        1.0,
        head.roll_rad,
    )
    .map_err(|_| {
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "axis_points_mm",
            "member frame could not be built",
        )
    })?
    .frame();
    let axis = |v: &beamcraft_geom::Vec3| [v.x(), v.y(), v.z()];
    let basis = [axis(&frame.x()), axis(&frame.y()), axis(&frame.z())];
    let origin = start;
    let dot = |u: [f64; 3], v: [f64; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    // The tool normal in element coordinates, then in the member frame.
    let normal = rotate_vector([0.0, 0.0, 1.0], tool.placement.orientation_xyzw);
    let (du, dv, dw) = (
        dot(normal, basis[0]),
        dot(normal, basis[1]),
        dot(normal, basis[2]),
    );
    if (du * du + dv * dv).sqrt() <= 1e-9 {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "tool_id",
            "the tool maps to a purely axial cutter",
        ));
    }
    // The axis point is the tool's own origin, resolved through the frame origin into the
    // member frame exactly as the replay resolves it back.
    let resolve = |p: [f64; 3]| -> [f64; 3] {
        let element = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
        [
            dot(element, basis[0]),
            dot(element, basis[1]),
            dot(element, basis[2]),
        ]
    };
    let axis_point_mm = resolve(tool.placement.translation_mm);
    // The hole frame in element coordinates: the member axis's projection onto the plane
    // perpendicular to the cut direction, and the direction cross it. Degenerate for a
    // purely axial direction, refused above.
    let x_hat = {
        let z = basis[2];
        let along = dot(z, normal);
        let p = [
            z[0] - along * normal[0],
            z[1] - along * normal[1],
            z[2] - along * normal[2],
        ];
        let norm = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        if !(norm.is_finite() && norm > 1e-12) {
            return Err(CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "tool_id",
                "the tool maps to a purely axial cutter",
            ));
        }
        [p[0] / norm, p[1] / norm, p[2] / norm]
    };
    let y_hat = [
        normal[1] * x_hat[2] - normal[2] * x_hat[1],
        normal[2] * x_hat[0] - normal[0] * x_hat[2],
        normal[0] * x_hat[1] - normal[1] * x_hat[0],
    ];
    // The tool's contour mapped into the hole frame, relative to the axis point (the
    // replay resolves the axis point back and adds these very offsets).
    let p0 = tool.placement.translation_mm;
    let mut polygon: Vec<[f64; 2]> = tool
        .contour_points_mm
        .iter()
        .map(|[x, y, _]| {
            let world = local_to_world([*x, *y, 0.0], &tool.placement);
            let rel = [world[0] - p0[0], world[1] - p0[1], world[2] - p0[2]];
            [dot(rel, x_hat), dot(rel, y_hat)]
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
    // The tool contour is counter-clockwise in its own frame; the mapped winding follows
    // the relative orientation, and the record needs clockwise.
    let mut area2 = 0.0f64;
    for (p, q) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        area2 += p[0] * q[1] - q[0] * p[1];
    }
    if !area2.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            "tool_id",
            "value is not finite",
        )
        .with_rule("DV-ATTR-003"));
    }
    if area2 > 0.0 {
        polygon.reverse();
    }
    let mut next = head.clone();
    next.hole_records
        .push(crate::model::command::MemberHoleRecord {
            operation_id: command.operation_id,
            ordinal,
            kind: crate::model::command::MemberHoleKind::PartCut,
            axis_point_mm,
            axis_direction: [du, dv, dw],
            extent: crate::model::command::HoleExtent::Through,
            polygon_mm: polygon,
            owner: None,
        });
    let mut footprint = Footprint::for_update(head.id, false, false, true, false, None)?;
    let read = Footprint::from_entries(
        &[Entry {
            target: Target::Element(tool.id),
            group: Group::ShapeInputs,
        }],
        &[],
    )?;
    footprint = footprint.union(read)?;
    Ok(PolygonCutOutcome {
        authored: AuthoredPart::Member(next),
        ordinal,
        footprint,
        fingerprint: command.content_fingerprint()?,
    })
}

/// The tool plate's prism mapped into the target plate's frame: the clockwise outline
/// projected along the tool normal onto the target's local `z = 0` plane, the prism's unit
/// axis in the target frame (its Z part kept positive) and whether the mapped band reaches
/// past both target faces. Any relative rotation is derived; a prism whose axis lies in
/// the target plane never crosses a face and is refused.
type MappedPrism = (Vec<[f64; 2]>, [f64; 3], bool);

fn tool_prism(target: &AuthoredPlate, tool: &AuthoredPlate) -> Result<MappedPrism, CommandError> {
    let relative = quat_multiply(
        quat_conjugate(target.placement.orientation_xyzw),
        tool.placement.orientation_xyzw,
    );
    let mut axis = rotate_vector([0.0, 0.0, 1.0], relative);
    let norm = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if !(norm.is_finite() && norm > 0.0) {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "tool_id",
            "the tool must be parallel to the target plate",
        ));
    }
    axis = [axis[0] / norm, axis[1] / norm, axis[2] / norm];
    // The prism is a line, so either axis direction maps the same outline; the axis is
    // canonicalized to a positive z part (master's flipped parallel tools keep their cut)
    // and the tool band keeps the flip sign alone.
    let flipped = axis[2] < 0.0;
    if flipped {
        axis = [-axis[0], -axis[1], -axis[2]];
    }
    let dz = axis[2];
    if !dz.is_finite() || dz.abs() <= 1e-6 {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "tool_id",
            "the tool's prism never crosses the target's faces",
        ));
    }
    let mut polygon: Vec<[f64; 2]> = tool
        .contour_points_mm
        .iter()
        .map(|[x, y, _]| {
            let world = local_to_world([*x, *y, 0.0], &tool.placement);
            let local = world_to_local(world, &target.placement);
            // Project along the prism axis onto the z = 0 plane: the opening's shape at
            // the reference height.
            let t = local[2] / dz;
            [local[0] - t * axis[0], local[1] - t * axis[1]]
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
    // The mapped winding follows the relative orientation; the record needs clockwise.
    let mut area2 = 0.0f64;
    for (p, q) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        area2 += p[0] * q[1] - q[0] * p[1];
    }
    if !area2.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            "tool_id",
            "value is not finite",
        )
        .with_rule("DV-ATTR-003"));
    }
    if area2 > 0.0 {
        polygon.reverse();
    }
    // Both material bands follow the authored thickness sides (issue 555 audit item 2):
    // the tool's band, carried along the prism axis into the target frame, must reach past
    // both faces of the target's own band. The record's prism lines hang off the projected
    // outline in the z = 0 plane, so the band is read at the outline's extreme axis
    // heights: the worst line must still clear both faces.
    let offset = world_to_local(tool.placement.translation_mm, &target.placement);
    let dot = |u: [f64; 3], v: [f64; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let centre = dot(offset, axis);
    let sign = if flipped { -1.0 } else { 1.0 };
    let [tool_low, tool_high] = tool.local_z_band_mm();
    let (s_lo, s_hi) = if sign > 0.0 {
        (centre + sign * tool_low, centre + sign * tool_high)
    } else {
        (centre + sign * tool_high, centre + sign * tool_low)
    };
    let mut rd_lo = f64::INFINITY;
    let mut rd_hi = f64::NEG_INFINITY;
    for r in &polygon {
        let height = dot([r[0], r[1], 0.0], axis);
        rd_lo = rd_lo.min(height);
        rd_hi = rd_hi.max(height);
    }
    let [target_low, target_high] = target.local_z_band_mm();
    let crosses = (s_lo - rd_hi) * dz < target_low - TOOL_REACH_MM
        && (s_hi - rd_lo) * dz > target_high + TOOL_REACH_MM;
    Ok((polygon, axis, crosses))
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
    // the mid of its authored thickness band (the phantom slab mid would flip the side
    // decision for a bottom or middle plate — the tool side of the same audit item already
    // reads its band, issue 569 items 6-9), a member's axis midpoint.
    let target = match (plate, member) {
        (Some(authored), _) => {
            let (cx, cy) = bbox_center(authored.contour_points_mm.iter().map(|p| [p[0], p[1]]));
            let [low, high] = authored.local_z_band_mm();
            let reference = [cx, cy, (low + high) / 2.0];
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

/// The two tool face plane points and the face normal in world coordinates. The extrusion
/// axis is the plate's own: the contour frame's ordered area vector for a 3D contour (its
/// body frame, `docs/design/k1-plate-contour-frame.md`), the placement `+Z` for a `z = 0`
/// contour. The faces sit at the material band the authored thickness side lays off (A12.1),
/// not at the phantom `0..t` slab a `top` plate alone has (issue 569 items 6 and 9).
fn tool_faces(tool: &AuthoredPlate) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let [low, high] = tool.local_z_band_mm();
    let framed = if tool.contour_points_mm.iter().any(|p| p[2] != 0.0) {
        crate::kernel::plate_contour::admit(&tool.contour_points_mm, tool.thickness_mm)
            .ok()
            .and_then(|contour| contour.frame)
            .map(|frame| {
                let z = frame.z();
                (
                    [z.x(), z.y(), z.z()],
                    tool.contour_points_mm
                        .first()
                        .copied()
                        .unwrap_or([0.0, 0.0, 0.0]),
                )
            })
    } else {
        None
    };
    match framed {
        // The face planes run through the contour origin laid off along the frame Z.
        Some((axis, origin)) => {
            let at = |offset: f64| {
                local_to_world(
                    [
                        origin[0] + axis[0] * offset,
                        origin[1] + axis[1] * offset,
                        origin[2] + axis[2] * offset,
                    ],
                    &tool.placement,
                )
            };
            (
                at(low),
                at(high),
                rotate_vector(axis, tool.placement.orientation_xyzw),
            )
        }
        // A `z = 0` contour: the contour centre carried along the placement `+Z`.
        None => {
            let (cx, cy) = bbox_center(tool.contour_points_mm.iter().map(|p| [p[0], p[1]]));
            let normal = rotate_vector([0.0, 0.0, 1.0], tool.placement.orientation_xyzw);
            (
                local_to_world([cx, cy, low], &tool.placement),
                local_to_world([cx, cy, high], &tool.placement),
                normal,
            )
        }
    }
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
        crate::engine::cut::Found::Plate(authored) => {
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
            let kept = crate::engine::cut::Found::Plate(authored).appended(kept_record);
            let CutTarget::Plate(kept) = kept else {
                return Err(CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "target_id",
                    "the split target changed kind",
                ));
            };
            (AuthoredPart::Plate(kept), AuthoredPart::Plate(created))
        }
        crate::engine::cut::Found::Member(authored) => {
            let mut other_cuts = copied_records(&authored.cuts, &command.new_id)?;
            other_cuts.extend(copied_records(
                core::slice::from_ref(&other_record),
                &command.new_id,
            )?);
            let mut created = authored.clone();
            created.id = command.new_id;
            created.name = command.new_name.clone();
            created.cuts = other_cuts;
            let kept = crate::engine::cut::Found::Member(authored).appended(kept_record);
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
    use crate::engine::command::evaluate_plate;
    use crate::model::command::{HoleExtent, MemberHoleKind};
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
            bends: Vec::new(),
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
            bends: Vec::new(),
        }
    }

    #[test]
    fn a_part_cut_tool_band_follows_both_thickness_sides() {
        // Issue 555 audit item 2: a 30 mm tool from z = -10 reaches past both faces of the
        // 10 mm target only when its band is read from its side: `top` [-10, 20] crosses the
        // target's own [0, 10], while the same placement read as `bottom` [-40, -10] misses
        // it; a `bottom` target [-10, 0] is crossed by a tool band [-20, 10].
        let at = |z: f64| Placement {
            translation_mm: [100.0, 100.0, z],
            ..Placement::IDENTITY
        };
        let target = plate();
        let cutter = tool(9, 50.0, 50.0, 30.0, at(-10.0));
        assert!(tool_prism(&target, &cutter).unwrap().2);
        let mut below = cutter.clone();
        below.thickness_side = Some(crate::model::command::PlateSide::Bottom);
        assert!(!tool_prism(&target, &below).unwrap().2);
        let mut sunk = plate();
        sunk.thickness_side = Some(crate::model::command::PlateSide::Bottom);
        assert!(!tool_prism(&sunk, &cutter).unwrap().2);
        assert!(
            tool_prism(&sunk, &tool(9, 50.0, 50.0, 30.0, at(-20.0)))
                .unwrap()
                .2
        );
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
            kind: crate::model::command::MemberKind::Beam,
            name: "Member".into(),
            class: 0,
            axis_points_mm: vec![[0.0, 0.0, 0.0], [length, 0.0, 0.0]],
            follow: None,
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
            hole_records: Vec::new(),
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
            axis_point_mm: None,
            axis_direction: None,
        };
        let outcome = cut_by_polygon(&command, Some(&source), None, &none, 0).unwrap();
        assert_eq!(outcome.ordinal, 0);
        // A member target needs the member form: the plate form on a member is an argument
        // error, not the old unsupported-geometry refusal (epic #556 slice 1).
        let beam = member(6, 6000.0, [0.0; 3], digest(b"profile"));
        let on_member = CutByPolygon {
            target_id: id(6),
            ..command.clone()
        };
        assert_eq!(
            cut_by_polygon(&on_member, Some(&source), Some(&beam), &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        let AuthoredPart::Plate(outcome_authored) = &outcome.authored else {
            panic!("a polygon cut on a plate target stays a plate");
        };
        assert_eq!(outcome_authored.hole_records[0].kind, HoleKind::PolygonCut);
        let body = evaluate_plate(outcome_authored).unwrap();
        assert!((body.quantities().volume_mm3 - 4_900_000.0).abs() <= 1e-6);
        // A second record takes the next ordinal of the shared through-cut counter.
        let next = CutByPolygon {
            operation_id: op(202),
            polygon_mm: square(300.0, 100.0, 100.0),
            ..command.clone()
        };
        assert_eq!(
            cut_by_polygon(&next, Some(outcome_authored), None, &none, 1)
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
        let AuthoredPart::Plate(attempted) = attempted.authored else {
            panic!("expected the plate target");
        };
        assert!(evaluate_plate(&attempted).is_err());
        // The plate form on a plate refuses the member frame fields, both together.
        let plate_frame = CutByPolygon {
            axis_point_mm: Some([0.0; 3]),
            axis_direction: Some([0.0, 1.0, 0.0]),
            ..command.clone()
        };
        assert_eq!(
            cut_by_polygon(&plate_frame, Some(&source), None, &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_member_polygon_cut_admits_an_owned_polygon_cut_record() {
        let beam = member(6, 6000.0, [0.0; 3], digest(b"profile"));
        // The member form: a slot through both walls of the section along +Y, through the body.
        let command = CutByPolygon {
            target_id: id(6),
            operation_id: op(205),
            polygon_mm: square(100.0, 100.0, 100.0),
            axis_point_mm: Some([500.0, 0.0, 0.0]),
            axis_direction: Some([0.0, 1.0, 0.0]),
        };
        let outcome = cut_by_polygon(&command, None, Some(&beam), &none, 0).unwrap();
        let AuthoredPart::Member(authored) = &outcome.authored else {
            panic!("expected the member target");
        };
        assert_eq!(outcome.ordinal, 0);
        assert_eq!(authored.hole_records.len(), 1);
        let record = &authored.hole_records[0];
        assert_eq!(record.kind, MemberHoleKind::PolygonCut);
        assert_eq!(record.extent, HoleExtent::Through);
        assert_eq!(record.owner, None);
        assert_eq!(record.polygon_mm, command.polygon_mm);
        // The plate form on the cut member is the argument error, and the duplicate id is the
        // shared record-identity refusal.
        let plate_form = CutByPolygon {
            axis_point_mm: None,
            axis_direction: None,
            ..command.clone()
        };
        assert_eq!(
            cut_by_polygon(&plate_form, None, Some(authored), &none, 1)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            cut_by_polygon(&command, None, Some(&beam), &|o: &OpId| *o == op(205), 0)
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        // An axial cut direction (a component along the member axis) is refused at the model
        // admission; the kernel cutter is perpendicular to the axis.
        let axial = CutByPolygon {
            axis_direction: Some([0.0, 0.0, 1.0]),
            ..command.clone()
        };
        assert_eq!(
            cut_by_polygon(&axial, None, Some(&beam), &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
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
        let AuthoredPart::Plate(outcome_authored) = &outcome.authored else {
            panic!("a plate target keeps a plate state");
        };
        // The vertical part cut is a part_cut record with the unit axis.
        assert_eq!(outcome_authored.hole_records[0].kind, HoleKind::PartCut);
        assert_eq!(
            outcome_authored.hole_records[0].axis_direction_mm,
            [0.0, 0.0, 1.0]
        );
        let body = evaluate_plate(outcome_authored).unwrap();
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
        // A tool lying in the target plane never crosses a face, and a steeply tilted tool
        // whose prism cannot cover the target's band either.
        let sideways = AuthoredPlate {
            placement: Placement {
                translation_mm: [150.0, 150.0, -5.0],
                orientation_xyzw: [
                    0.0,
                    (std::f64::consts::FRAC_PI_2).sin(),
                    0.0,
                    (std::f64::consts::FRAC_PI_2).cos(),
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
        // An oblique tool (45 degrees) whose mapped prism still passes both faces cuts:
        // the band, read at the mapped outline's extreme axis heights, covers the slab
        // (the tool is thick enough for the projected outline's spread).
        let leaning = tool(
            6,
            100.0,
            100.0,
            200.0,
            Placement {
                translation_mm: [500.0, 200.0, -61.6],
                orientation_xyzw: [
                    0.0,
                    (std::f64::consts::FRAC_PI_8).sin(),
                    0.0,
                    (std::f64::consts::FRAC_PI_8).cos(),
                ],
            },
        );
        let lean = cut_by_part(&command, Some(&source), None, Some(&leaning), &none, 1).unwrap();
        let AuthoredPart::Plate(lean_authored) = &lean.authored else {
            panic!("a plate target keeps a plate state");
        };
        let lean_axis = lean_authored.hole_records[0].axis_direction_mm;
        assert!((lean_axis[0] - std::f64::consts::FRAC_1_SQRT_2).abs() <= 1e-12);
        assert!((lean_axis[2] - std::f64::consts::FRAC_1_SQRT_2).abs() <= 1e-12);
        assert!(evaluate_plate(lean_authored).is_ok());
        // A flipped parallel tool (its local +Z pointing against the target normal) cuts
        // exactly the upright tool's cut: the prism is a line, the axis canonicalizes to
        // the positive z and only the band keeps the flip sign.
        let flipped = tool(
            6,
            100.0,
            100.0,
            20.0,
            Placement {
                translation_mm: [150.0, 150.0, 15.0],
                orientation_xyzw: [1.0, 0.0, 0.0, 0.0],
            },
        );
        let flipped_cut =
            cut_by_part(&command, Some(&source), None, Some(&flipped), &none, 2).unwrap();
        let AuthoredPart::Plate(flipped_authored) = &flipped_cut.authored else {
            panic!("a plate target keeps a plate state");
        };
        assert_eq!(
            flipped_authored.hole_records[0].axis_direction_mm,
            [0.0, 0.0, 1.0]
        );
        assert!(evaluate_plate(flipped_authored).is_ok());
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
    fn a_fit_against_a_framed_contour_tool_trims_at_the_contour_plane() {
        // Issue 569 item 6: the tool's contour is a rectangle in the plane x = 2000. Its
        // frame Z is the ordered area vector +X, so the fit plane is the contour plane, not
        // the phantom placement-Z slab that sliced the member lengthwise.
        let beam = member(6, 3000.0, [0.0; 3], digest(b"profile"));
        let end_plate = AuthoredPlate {
            contour_points_mm: vec![
                [2000.0, -300.0, -300.0],
                [2000.0, 300.0, -300.0],
                [2000.0, 300.0, 300.0],
                [2000.0, -300.0, 300.0],
            ],
            thickness_mm: 20.0,
            ..tool(7, 0.0, 0.0, 20.0, Placement::IDENTITY)
        };
        let command = FitEndToPart {
            target_id: id(6),
            operation_id: op(207),
            tool_id: id(7),
        };
        let outcome =
            fit_end_to_part(&command, None, Some(&beam), Some(&end_plate), &none, 0).unwrap();
        let CutTarget::Member(authored) = &outcome.target else {
            panic!("expected the member target");
        };
        let [record] = authored.cuts.as_slice() else {
            panic!("expected one cut record");
        };
        // The plane is x = 2000 and the midpoint side is kept.
        assert!((record.point_mm[0] - 2000.0).abs() <= 1e-9);
        assert_eq!(record.normal, [1.0, 0.0, 0.0]);
        assert_eq!(record.keep, KeepSide::Negative);
        // A cap plate written at z = 3000 trims the column there, not at its thickness.
        let column = AuthoredMember {
            kind: crate::model::command::MemberKind::Column,
            axis_points_mm: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 3500.0]],
            ..member(6, 3500.0, [0.0; 3], digest(b"profile"))
        };
        let cap = AuthoredPlate {
            contour_points_mm: vec![
                [-300.0, -300.0, 3000.0],
                [300.0, -300.0, 3000.0],
                [300.0, 300.0, 3000.0],
                [-300.0, 300.0, 3000.0],
            ],
            thickness_mm: 20.0,
            ..tool(8, 0.0, 0.0, 20.0, Placement::IDENTITY)
        };
        let column_command = FitEndToPart {
            target_id: id(6),
            operation_id: op(208),
            tool_id: id(8),
        };
        let outcome =
            fit_end_to_part(&column_command, None, Some(&column), Some(&cap), &none, 0).unwrap();
        let CutTarget::Member(authored) = &outcome.target else {
            panic!("expected the member target");
        };
        let [record] = authored.cuts.as_slice() else {
            panic!("expected one cut record");
        };
        assert!((record.point_mm[2] - 3000.0).abs() <= 1e-9);
        assert_eq!(record.normal, [0.0, 0.0, 1.0]);
        assert_eq!(record.keep, KeepSide::Negative);
    }

    #[test]
    fn a_fit_reads_the_tool_thickness_side_not_the_phantom_slab() {
        // Issue 569 item 9: a `bottom` tool spans x = 1900..2000, so the near face is
        // x = 2000; reading the phantom 0..t slab would cut one thickness short at 2100. The
        // same body authored as `top` with its plane one thickness back fits identically.
        let beam = AuthoredMember {
            axis_points_mm: vec![[1000.0, 0.0, 0.0], [5000.0, 0.0, 0.0]],
            ..member(6, 4000.0, [0.0; 3], digest(b"profile"))
        };
        // Placement x-axis +Y, y-axis +Z, so the plate's local Z is the world +X.
        let orientation = [0.5, 0.5, 0.5, 0.5];
        let bottom = AuthoredPlate {
            thickness_mm: 100.0,
            thickness_side: Some(crate::model::command::PlateSide::Bottom),
            placement: Placement {
                translation_mm: [2000.0, -300.0, -300.0],
                orientation_xyzw: orientation,
            },
            ..tool(7, 600.0, 600.0, 100.0, Placement::IDENTITY)
        };
        let top = AuthoredPlate {
            thickness_mm: 100.0,
            placement: Placement {
                translation_mm: [1900.0, -300.0, -300.0],
                orientation_xyzw: orientation,
            },
            ..tool(8, 600.0, 600.0, 100.0, Placement::IDENTITY)
        };
        for (tool_id, authored) in [(id(7), &bottom), (id(8), &top)] {
            let command = FitEndToPart {
                target_id: id(6),
                operation_id: op(209),
                tool_id,
            };
            let outcome =
                fit_end_to_part(&command, None, Some(&beam), Some(authored), &none, 0).unwrap();
            let CutTarget::Member(fitted) = &outcome.target else {
                panic!("expected the member target");
            };
            let [record] = fitted.cuts.as_slice() else {
                panic!("expected one cut record");
            };
            // The trim is flush with the shared real face x = 2000.
            assert_eq!(record.point_mm, [2000.0, 0.0, 0.0]);
            assert_eq!(record.normal, [1.0, 0.0, 0.0]);
            assert_eq!(record.keep, KeepSide::Positive);
        }
    }

    #[test]
    fn a_fit_reads_the_target_thickness_side_not_the_phantom_slab() {
        // Issue 569 items 6-9, target half (the review on the 1111 batch): a `bottom`
        // 100 mm target spans world x = 0..100, so the reference midpoint sits at x = 50 and
        // the fit trims flush with the tool's near face at x = 90. Reading the phantom slab
        // mid (local t/2, world x = 150) picks the tool's far face at x = 100 instead and the
        // fit removes nothing. The same body authored as `top` one placement back fits
        // identically, so the decision follows the band, not the absolute position.
        let orientation = [0.5, 0.5, 0.5, 0.5];
        let bottom = AuthoredPlate {
            thickness_mm: 100.0,
            thickness_side: Some(crate::model::command::PlateSide::Bottom),
            placement: Placement {
                translation_mm: [100.0, -300.0, -300.0],
                orientation_xyzw: orientation,
            },
            ..tool(9, 600.0, 600.0, 100.0, Placement::IDENTITY)
        };
        let top = AuthoredPlate {
            thickness_mm: 100.0,
            placement: Placement {
                translation_mm: [0.0, -300.0, -300.0],
                orientation_xyzw: orientation,
            },
            ..tool(10, 600.0, 600.0, 100.0, Placement::IDENTITY)
        };
        let fitting = AuthoredPlate {
            thickness_mm: 10.0,
            placement: Placement {
                translation_mm: [90.0, -300.0, -300.0],
                orientation_xyzw: orientation,
            },
            ..tool(7, 600.0, 600.0, 10.0, Placement::IDENTITY)
        };
        for (target_id, authored) in [(id(9), &bottom), (id(10), &top)] {
            let command = FitEndToPart {
                target_id,
                operation_id: op(210),
                tool_id: id(7),
            };
            let outcome =
                fit_end_to_part(&command, Some(authored), None, Some(&fitting), &none, 0).unwrap();
            let CutTarget::Plate(fitted) = &outcome.target else {
                panic!("expected the plate target");
            };
            let [record] = fitted.cuts.as_slice() else {
                panic!("expected one cut record");
            };
            // The derived plane lives in the target's own frame: the trim is flush with the
            // tool's near real face (target-local z = -10 for the bottom band, z = 90 for the
            // top band), keeping the material side the authored band mid lies on. The phantom
            // slab mid would pick the tool's far face instead and trim nothing.
            let expected_face = if target_id == id(9) { -10.0 } else { 90.0 };
            assert_eq!(record.point_mm, [300.0, 300.0, expected_face]);
            assert_eq!(record.normal, [0.0, 0.0, 1.0]);
            assert_eq!(record.keep, KeepSide::Negative);
        }
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
