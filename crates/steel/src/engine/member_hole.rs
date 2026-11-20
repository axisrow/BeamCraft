// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F04 command application for member holes (A04.2, the F06 member-hole amendment): one owned
//! member-hole record appended to, replaced in or removed from a valid member, and the replay
//! of the member's records over the evaluated body.
//!
//! The engine never touches USD. The commands check the record admission the command layer owes
//! (target lifecycle, record identity across the project, ordinal capacity), mutate the
//! authored state and declare the footprint; the adapter re-evaluates the body through the
//! ordinary [`crate::engine::member::evaluate_member`] replay and commits the result as one
//! transaction. A rejection changes nothing.
//!
//! Replay cuts each record with the kernel's certified section-aware member-hole operation
//! (G14.1 with the G14.2 directed cutters): the body is mapped into the member's own frame by
//! an exact axis permutation, so a member whose frame axes are the coordinate axes (unit
//! components `0.0`/`+-1.0`, which includes every member whose axis is parallel to a
//! coordinate axis and carries no roll) is cut with bit-exact coordinates. The cutter is
//! directed: any section-plane hole direction and a `through` or a `range` extent are
//! certified, on flat walls and on faceted, sloped or multi-facet chains alike. Still refused
//! with `UNSUPPORTED_GEOMETRY`: a roll or a non-coordinate member axis (the permutation must
//! stay exact), a curved member or polybeam, and a hole direction with an axial component
//! (the kernel cutter is perpendicular to the member axis). A stored record the replay can no
//! longer serve (the member was edited, the hole no longer fits) refuses the evaluation with
//! the typed hole diagnostic, and the replay runs in ordinal order, so the member keeps its
//! last valid state as F04 section 4 says.

use crate::engine::member::AuthoredMember;
use crate::kernel::member_hole::{MemberHoleCutter, member_through_hole};
use crate::model::command::{
    AddHole, HoleExtent, HoleSpec, MAX_HOLES, MemberHoleKind, MemberHoleRecord, UpdateHole,
};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::solid::SolidMesh;
use beamcraft_geom::Frame3;
use beamcraft_kernel::ops::{Body, OpCode, OpDefect, OpFailure};
use beamcraft_kernel::profile::{Cavity, Point2};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use beamcraft_model::id::OpId;

/// Millimetres a through cutter reaches past both faces of the body, as for plate holes.
pub(crate) const CUTTER_OVERSHOOT_MM: f64 = 1.0;

/// One replay record's body-height closure over the cutter axis.
type MemberRecordHeight = Box<dyn Fn(&[f64; 3]) -> f64>;

/// Largest operation ordinal the kernel names a hole wall with (`cut:f.h<ordinal>` inside the
/// 64-byte FeatureId bound); one more record on one member is an implementation limit.
const MAX_ORDINAL: usize = 999;

/// Everything one accepted member-hole command produces for the adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberHoleOutcome {
    /// The member's authored state with the record appended, replaced or removed.
    pub authored: AuthoredMember,
    /// The ordinal the record took, or the retired record's ordinal for a removal.
    pub ordinal: usize,
    /// Declared read and write sets.
    pub footprint: Footprint,
    /// Digest of the typed arguments, for replay and reuse decisions.
    pub fingerprint: ContentDigest,
}

/// The member generator of a hole command; any other generator is `UNSUPPORTED_GEOMETRY` (a
/// polygon hole in a member is not admitted in this slice).
fn member_generator(command: &AddHole) -> Result<&crate::model::command::MemberHole, CommandError> {
    match &command.hole {
        HoleSpec::Member(member) => Ok(member),
        _ => Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "hole",
            "a polygon hole in a member is not supported",
        )),
    }
}

fn target_of(head: &AuthoredMember, command: &AddHole) -> Result<(), CommandError> {
    if head.id != command.target_id {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "hole target not found",
        ));
    }
    Ok(())
}

/// The next per-member ordinal in creation order: one past the last live record, with the
/// counter's capacity check (the same rule as a plate's hole counter).
pub(crate) fn admit_ordinal(
    records: &[MemberHoleRecord],
    pinned: usize,
) -> Result<usize, CommandError> {
    let minimum = records
        .last()
        .map_or(Some(0), |r| r.ordinal.checked_add(1))
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "hole.ordinal",
                "hole ordinal exhausted",
            )
        })?;
    if pinned > MAX_ORDINAL {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "hole.ordinal",
            "hole ordinal capacity exhausted",
        ));
    }
    if pinned < minimum {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "hole.ordinal",
            "hole counter regressed",
        ));
    }
    Ok(pinned)
}

/// Appends one member-hole record to the head authored state of a member: argument admission,
/// an unused record id, the next per-member ordinal and the record itself (F06 member-hole
/// amendment sections 1 and 3). Nothing is evaluated here; the adapter re-evaluates the body
/// and refuses the whole command if the replay does.
///
/// `operation_id_used` reports whether the project already holds a record with the id on any
/// element: a record id is unused in the project (`DUPLICATE_ID`), not only on the target.
/// `next_ordinal` comes from the adapter's persistent retirement counter, so a removed record's
/// ordinal is never reused.
pub fn add_member_hole_after(
    command: &AddHole,
    head: &AuthoredMember,
    operation_id_used: &dyn Fn(OpId) -> bool,
    next_ordinal: usize,
) -> Result<MemberHoleOutcome, CommandError> {
    command.validate_arguments()?;
    let member = member_generator(command)?;
    target_of(head, command)?;
    if operation_id_used(command.operation_id) {
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
    let ordinal = admit_ordinal(&head.hole_records, next_ordinal)?;
    let mut next = head.clone();
    next.hole_records.push(MemberHoleRecord {
        operation_id: command.operation_id,
        ordinal,
        kind: crate::model::command::MemberHoleKind::Hole,
        axis_point_mm: member.axis_point_mm,
        axis_direction: member.axis_direction,
        extent: member.extent,
        polygon_mm: command.polygon()?,
        // A hand-placed hole belongs to nobody (the F06 member-hole amendment); only a
        // bolt group's own generation writes an owner (bolt amendment section 5).
        owner: None,
    });
    Ok(MemberHoleOutcome {
        footprint: Footprint::for_update(command.target_id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
        authored: next,
        ordinal,
    })
}

/// Admits one A13 member polygon cut (epic #556 slice 1) as an owned record of kind
/// [`MemberHoleKind::PolygonCut`]: the shared record identity, the `MAX_HOLES` capacity of the
/// member, the next persistent ordinal and the record itself — a through cut along a
/// section-plane direction with an authoritative clockwise polygon in the hole frame, replayed
/// by the same certified member prism subtraction as the hole records. Nothing is evaluated
/// here; the adapter re-evaluates the body and refuses the whole command if the replay does.
///
/// `operation_id_used` and `next_ordinal` have the same contracts as
/// [`add_member_hole_after`].
#[allow(clippy::too_many_arguments)]
pub fn admit_member_polygon_cut(
    target_id: ElementId,
    operation_id: OpId,
    head: &AuthoredMember,
    axis_point_mm: [f64; 3],
    axis_direction: [f64; 3],
    polygon_mm: Vec<[f64; 2]>,
    fingerprint: ContentDigest,
    operation_id_used: &dyn Fn(OpId) -> bool,
    next_ordinal: usize,
) -> Result<MemberHoleOutcome, CommandError> {
    if target_id != head.id {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "the element is not in the head revision",
        ));
    }
    if operation_id_used(operation_id) {
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
            "more than 64 through cuts on one member",
        )
        .with_rule("DV-LIM-001"));
    }
    let ordinal = admit_ordinal(&head.hole_records, next_ordinal)?;
    let mut next = head.clone();
    next.hole_records.push(MemberHoleRecord {
        operation_id,
        ordinal,
        axis_point_mm,
        axis_direction,
        extent: HoleExtent::Through,
        polygon_mm,
        // A polygon cut is authored, never generated; a bolt group owns only its holes.
        owner: None,
        kind: MemberHoleKind::PolygonCut,
    });
    Ok(MemberHoleOutcome {
        footprint: Footprint::for_update(target_id, false, false, true, false, None)?,
        fingerprint,
        authored: next,
        ordinal,
    })
}

/// Replaces one member hole's parameters, preserving its identity and ordinal. A record a
/// bolt group generated belongs to the group (the F06 member-hole amendment section 3): it
/// is edited only through the group's own update, never separately (DV-REL-009, as for the
/// plate records of A14.4).
pub fn update_member_hole(
    command: &UpdateHole,
    head: &AuthoredMember,
) -> Result<MemberHoleOutcome, CommandError> {
    command.validate_arguments()?;
    target_of(head, &command.hole)?;
    let member = member_generator(&command.hole)?;
    let mut next = head.clone();
    let record = next
        .hole_records
        .iter_mut()
        .find(|r| r.operation_id == command.hole.operation_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "operation_id",
                "hole record not found on target",
            )
        })?;
    // A generated hole is owned by its bolt group (bolt amendment section 5): it is edited
    // and removed only through `UpdateBoltGroup`/`DeleteElement` of the group, never
    // separately. The adapter names the group from the same record; this typed refusal
    // fails closed.
    if record.owner.is_some() {
        return Err(CommandError::new(
            ErrorCode::DependencyConflict,
            "operation_id",
            "the hole was generated by a bolt group; edit or delete the group",
        )
        .with_rule("DV-REL-009"));
    }
    // An A13 polygon cut shares the record list but is not an A04.2 hole: the adapter refuses
    // it with a typed result instead of editing one (the plate guard mirrored).
    if record.kind != MemberHoleKind::Hole {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "operation_id",
            "the record is a polygon cut, not a hole",
        ));
    }
    let ordinal = record.ordinal;
    record.axis_point_mm = member.axis_point_mm;
    record.axis_direction = member.axis_direction;
    record.extent = member.extent;
    record.polygon_mm = command.hole.polygon()?;
    Ok(MemberHoleOutcome {
        footprint: Footprint::for_update(command.hole.target_id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
        authored: next,
        ordinal,
    })
}

/// Removes one member hole while preserving every surviving record's identity and ordinal.
pub fn remove_member_hole(
    command: &crate::model::command::RemoveHole,
    head: &AuthoredMember,
) -> Result<MemberHoleOutcome, CommandError> {
    if head.id != command.target_id {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "hole target not found",
        ));
    }
    let ordinal = head
        .hole_records
        .iter()
        .find(|r| r.operation_id == command.operation_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "operation_id",
                "hole record not found on target",
            )
        })?
        .ordinal;
    // A generated hole belongs to its bolt group (bolt amendment section 5); it never
    // leaves on its own (DV-REL-009, as for the plate records of A14.4).
    if head
        .hole_records
        .iter()
        .any(|r| r.operation_id == command.operation_id && r.owner.is_some())
    {
        return Err(CommandError::new(
            ErrorCode::DependencyConflict,
            "operation_id",
            "the hole was generated by a bolt group; edit or delete the group",
        )
        .with_rule("DV-REL-009"));
    }
    // The same kind guard as `update_member_hole`: retiring a polygon cut is not an A04.2
    // removal (the plate guard mirrored).
    if head
        .hole_records
        .iter()
        .any(|r| r.operation_id == command.operation_id && r.kind != MemberHoleKind::Hole)
    {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "operation_id",
            "the record is a polygon cut, not a hole",
        ));
    }
    let mut next = head.clone();
    next.hole_records
        .retain(|r| r.operation_id != command.operation_id);
    Ok(MemberHoleOutcome {
        footprint: Footprint::for_update(command.target_id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
        authored: next,
        ordinal,
    })
}

/// An exact signed axis permutation: three signed coordinate unit rows mapping a vector to
/// `out[k] = row[k] . v`. Every component of every row is exactly `0.0`, `1.0` or `-1.0`, so
/// every product and sum below is computed exactly and replay is bit-stable.
#[derive(Debug, Clone, Copy)]
struct Axes {
    x: [f64; 3],
    y: [f64; 3],
    z: [f64; 3],
}

impl Axes {
    /// Admits three signed unit rows that form a proper rotation (`x` cross `y` is `z`), so
    /// winding is preserved; `None` for anything else.
    fn of_rows(x: [f64; 3], y: [f64; 3], z: [f64; 3]) -> Option<Self> {
        let unit = |row: [f64; 3]| {
            row.iter().filter(|v| **v != 0.0).count() == 1
                && row.iter().all(|v| *v == 0.0 || *v == 1.0 || *v == -1.0)
        };
        if !unit(x) || !unit(y) || !unit(z) || cross(&x, &y) != z {
            return None;
        }
        Some(Self { x, y, z })
    }

    /// Applies the permutation to one vector, normalizing negative zero (a blob coordinate
    /// must never be `-0.0`).
    fn apply(&self, v: &[f64; 3]) -> [f64; 3] {
        [self.x, self.y, self.z].map(|row| zeroed(weighted(row, *v)))
    }

    /// The inverse of a proper signed permutation is its transpose.
    fn inverse(&self) -> Self {
        let [x0, x1, x2] = self.x;
        let [y0, y1, y2] = self.y;
        let [z0, z1, z2] = self.z;
        Self {
            x: [x0, y0, z0],
            y: [x1, y1, z1],
            z: [x2, y2, z2],
        }
    }
}

/// The exact dot of a signed unit row and a vector; the zero components contribute nothing.
fn weighted(row: [f64; 3], v: [f64; 3]) -> f64 {
    let [a, b, c] = row;
    let [x, y, z] = v;
    let partial = if a == 0.0 { 0.0 } else { a * x };
    let partial = partial + if b == 0.0 { 0.0 } else { b * y };
    partial + if c == 0.0 { 0.0 } else { c * z }
}

/// Normalizes negative zero.
fn zeroed(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

/// The member frame the replay serves: three rows that are a proper signed coordinate
/// permutation, so every resolved coordinate is exact and replay is bit-stable. A roll or a
/// non-coordinate member axis admits no generated record (`UNSUPPORTED_GEOMETRY`, the same
/// refusal a stored record meets on replay).
pub(crate) fn is_coordinate_frame(rows: &[[f64; 3]; 3]) -> bool {
    Axes::of_rows(rows[0], rows[1], rows[2]).is_some()
}

/// The exact cross product of two signed coordinate unit vectors.
fn cross(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    let [ax, ay, az] = *a;
    let [bx, by, bz] = *b;
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

/// The member frame axes as component arrays.
fn basis_of(frame: &Frame3) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let axis = |v: &beamcraft_geom::Vec3| [v.x(), v.y(), v.z()];
    (axis(&frame.x()), axis(&frame.y()), axis(&frame.z()))
}

/// The pending-kernel-slice refusal: the case needs the section-aware member-hole operation of
/// G14.1, which is not delivered yet. Every message says so, because the typed error carries a
/// fixed safe message and nothing else.
pub(crate) fn pending_member_holes(reason: &'static str) -> CommandError {
    CommandError::new(ErrorCode::UnsupportedGeometry, "hole", reason)
}

/// The refusal for a stored record the replay can no longer serve: the hole left the member,
/// reaches a free edge or meets another hole, so the member keeps its last valid state (F04
/// section 4). The replay runs in ordinal order, so the first unfit record refuses.
fn record_refusal(failure: &OpFailure) -> CommandError {
    CommandError::new(
        failure.code.f04_code(),
        "hole",
        record_refusal_message(failure),
    )
}

/// The fixed message for one kernel refusal of a member-hole record. The defect decides
/// where it can: a blind, tangent or sub-floor range is the cutter's own refusal, a body that
/// stopped being a straight prism says so.
fn record_refusal_message(failure: &OpFailure) -> &'static str {
    match failure.defect {
        OpDefect::CutterNotThrough | OpDefect::CutterTangent | OpDefect::CutterMargin => {
            "a member hole record's range ends inside the member or touches a wall"
        }
        OpDefect::BodyNotPrism => {
            "a member hole record no longer fits: the body is not a straight prism for the kernel"
        }
        _ => match failure.code {
            OpCode::ContourInvalid | OpCode::UnsupportedContour => {
                "a member hole record does not fit strictly inside a wall face, clear of its edges and the other holes"
            }
            OpCode::OpNoEffect | OpCode::OpUnsupported => {
                "a member hole record no longer fits the member"
            }
            OpCode::AboveLimit => "a member hole record exceeds the kernel capacity",
            _ => "the kernel refused a member hole record",
        },
    }
}

/// Rewrites the vertex records of a BCMESH01 blob through `axes`. The layout is the certified
/// one (`from_blob`): a 32-byte header, then `24 * vertices` bytes of little-endian `f64`
/// triples; everything after the vertices is copied unchanged.
fn permuted_blob(blob: &[u8], axes: &Axes) -> Result<Vec<u8>, CommandError> {
    let word = |at: usize| -> Option<u32> {
        let bytes = blob.get(at..at + 4)?;
        Some(u32::from_le_bytes(bytes.try_into().ok()?))
    };
    let vertices = word(12)
        .ok_or_else(|| pending_member_holes("the body mesh blob is not readable"))?
        as usize;
    let vertex_bytes = vertices
        .checked_mul(24)
        .ok_or_else(|| pending_member_holes("the body mesh blob is not readable"))?;
    let end = usize::try_from(32u64.saturating_add(vertex_bytes as u64))
        .ok()
        .filter(|end| *end <= blob.len())
        .ok_or_else(|| pending_member_holes("the body mesh blob is not readable"))?;
    let mut out = blob.to_vec();
    for at in (32..end).step_by(24) {
        let mut point = [0.0; 3];
        for (k, slot) in point.iter_mut().enumerate() {
            let bytes = blob
                .get(at + k * 8..at + k * 8 + 8)
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| pending_member_holes("the body mesh blob is not readable"))?;
            *slot = f64::from_le_bytes(bytes);
        }
        for (k, value) in axes.apply(&point).into_iter().enumerate() {
            let bytes = value.to_le_bytes();
            let at = at + k * 8;
            out.get_mut(at..at + 8)
                .ok_or_else(|| pending_member_holes("the body mesh blob is not readable"))?
                .copy_from_slice(&bytes);
        }
    }
    Ok(out)
}

/// The body's mesh in the frame `axes` maps to: a certified body again, with the same face
/// table and the permuted face normals.
fn permuted_body(body: &Body, axes: &Axes) -> Result<Body, CommandError> {
    let unreadable = || pending_member_holes("the body mesh blob is not readable");
    let blob = permuted_blob(body.mesh().to_blob().as_slice(), axes)?;
    let mesh = beamcraft_kernel::trimesh::TriMesh::from_blob(&blob).map_err(|_| unreadable())?;
    let normals = body
        .face_normals()
        .iter()
        .map(|n| n.map(|n| axes.apply(&n)))
        .collect();
    Body::from_mesh(
        mesh,
        body.face_table().to_vec(),
        normals,
        Some(body.tolerance()),
    )
    .map_err(|_| pending_member_holes("the permuted body could not be admitted"))
}

/// Replays the member's hole records over `body` (the base shape with its plane cuts), in
/// ordinal order, with the kernel's certified section-aware member-hole operation. The records
/// come from the member frame; `frame` is the member frame's axes in element coordinates and
/// `origin_mm` is the frame origin in element coordinates (the member's first axis point). The
/// returned mesh is in element coordinates.
///
/// A straight member whose frame axes are exact coordinate axes is served with any
/// section-plane hole direction and a `through` or a `range` extent; a roll or a
/// non-coordinate axis, a curved member or polybeam, and an axial hole direction are refused.
/// A record the kernel refuses (it left the member, ends inside a wall, meets another hole)
/// refuses the evaluation with a diagnostic naming the record, so the member keeps its last
/// valid state.
pub(crate) fn replay_member_holes(
    frame: &Frame3,
    origin_mm: [f64; 3],
    body: &Body,
    records: &[MemberHoleRecord],
) -> Result<SolidMesh, CommandError> {
    let (bx, by, bz) = basis_of(frame);
    let Some(axes_frame) = Axes::of_rows(bx, by, bz) else {
        return Err(pending_member_holes(
            "a member hole needs a member frame whose axes are the coordinate axes, without roll",
        ));
    };
    // The kernel frame is the exact cyclic rotation of the member frame: X = the member axis
    // `Z_m`, Y = `X_r`, Z = `Y_r`. A cyclic rotation of a proper rotation stays proper, so the
    // inverse below is its transpose and the body becomes the X-extrusion the kernel op serves.
    let axes_kernel = Axes {
        x: axes_frame.z,
        y: axes_frame.x,
        z: axes_frame.y,
    };
    let back = axes_kernel.inverse();
    // Each record cuts the body in the kernel frame; between records the result comes back
    // to element coordinates, so the next record maps it afresh.
    let mut solid: Option<SolidMesh> = None;
    let mut element_body = body.clone();
    for record in records {
        // The hole axis. A hole or a polygon cut keeps the certified section-plane form
        // and its exact arithmetic; a part cut may lean along the member (its replay
        // normalizes the general direction), a purely axial one staying refused.
        let [du, dv, dw] = record.axis_direction;
        let part_cut = record.kind == crate::model::command::MemberHoleKind::PartCut;
        if !part_cut && dw != 0.0 {
            return Err(pending_member_holes(
                "a member hole axis must stay in the member's section plane",
            ));
        }
        let hole_body = permuted_body(&element_body, &axes_kernel)?;
        // The cutter's axis, range and footprint frame, per kind. The kernel frame is the
        // exact cyclic rotation of the member frame (X = the member axis `Z_m`), so a
        // member-frame direction `(du, dv, dw)` reads `(dw, du, dv)` there.
        let cutter = if part_cut {
            let norm = (du * du + dv * dv + dw * dw).sqrt();
            if !(norm.is_finite() && norm > 0.0) {
                return Err(pending_member_holes(
                    "a member hole axis direction needs a nonzero transverse part",
                ));
            }
            let (dx, dy, dz) = (dw / norm, du / norm, dv / norm);
            let lt = (dy * dy + dz * dz).sqrt();
            if !(lt.is_finite() && lt > 1e-12) {
                return Err(pending_member_holes(
                    "a member hole axis must cross the member's walls, not run along them",
                ));
            }
            let height: MemberRecordHeight =
                Box::new(move |p: &[f64; 3]| p[0].mul_add(dx, p[1].mul_add(dy, p[2] * dz)));
            let (t_lo, t_hi) = hole_body
                .mesh()
                .vertices()
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                    (lo.min(height(p)), hi.max(height(p)))
                });
            let point = record.axis_point_mm;
            let element = [
                origin_mm[0] + weighted([bx[0], by[0], bz[0]], point),
                origin_mm[1] + weighted([bx[1], by[1], bz[1]], point),
                origin_mm[2] + weighted([bx[2], by[2], bz[2]], point),
            ];
            let (a, y_k, z_k) = (
                weighted(bz, element),
                weighted(bx, element),
                weighted(by, element),
            );
            let t0 = dx.mul_add(a, y_k.mul_add(dy, z_k * dz));
            let (from, to) = match record.extent {
                HoleExtent::Through => (t_lo - CUTTER_OVERSHOOT_MM, t_hi + CUTTER_OVERSHOOT_MM),
                HoleExtent::Range { from_mm, to_mm } => (t0 + from_mm, t0 + to_mm),
            };
            // The axis point in the footprint plane: `a` along the member axis, `b` along
            // the oblique frame's `e2 = (0, dz, -dy)/Lt`. The stored polygon is clockwise
            // in exactly this frame — its hole-frame XY is the member axis projected onto
            // the plane perpendicular to the axis and `axis_direction x` that — so the
            // coordinates carry unchanged.
            let centre = [a, (dz * y_k - dy * z_k) / lt];
            let cavity = Cavity::polygon(
                record
                    .polygon_mm
                    .iter()
                    .map(|[x, y]| Point2::new(centre[0] + x, centre[1] + y))
                    .collect(),
            );
            let ordinal = u16::try_from(record.ordinal)
                .ok()
                .filter(|ordinal| *ordinal <= 999)
                .ok_or_else(|| {
                    pending_member_holes("the record ordinal exceeds the kernel capacity")
                })?;
            MemberHoleCutter::oblique(cavity, [dx, dy, dz], from, to, ordinal)
        } else {
            let norm = (du * du + dv * dv).sqrt();
            if !(norm.is_finite() && norm > 0.0) {
                return Err(pending_member_holes(
                    "a member hole axis direction needs a nonzero transverse part",
                ));
            }
            let (dy, dz) = (du / norm, dv / norm);
            let height: MemberRecordHeight =
                Box::new(move |p: &[f64; 3]| p[1].mul_add(dy, p[2] * dz));
            let (t_lo, t_hi) = hole_body
                .mesh()
                .vertices()
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                    (lo.min(height(p)), hi.max(height(p)))
                });
            let point = record.axis_point_mm;
            // The stored member-frame point `(u, v, w)` resolves through the frame origin into
            // element coordinates, then into the kernel frame's `a`, transverse line and height
            // coordinates. For a frame origin at the coordinate origin each resolved component is
            // the stored one (the cross terms vanish exactly on the signed unit rows), so the
            // origin-based members keep their byte-exact results.
            let element = [
                origin_mm[0] + weighted([bx[0], by[0], bz[0]], point),
                origin_mm[1] + weighted([bx[1], by[1], bz[1]], point),
                origin_mm[2] + weighted([bx[2], by[2], bz[2]], point),
            ];
            let (a, y_k, z_k) = (
                weighted(bz, element),
                weighted(bx, element),
                weighted(by, element),
            );
            let t0 = y_k.mul_add(dy, z_k * dz);
            let (from, to) = match record.extent {
                HoleExtent::Through => (t_lo - CUTTER_OVERSHOOT_MM, t_hi + CUTTER_OVERSHOOT_MM),
                HoleExtent::Range { from_mm, to_mm } => (t0 + from_mm, t0 + to_mm),
            };
            // The axis point in the footprint plane: `a` along the member axis, `b` along the
            // axis' `e2 = d x e1`. The stored polygon is clockwise in exactly this frame — its
            // hole-frame XY is `(Z_m, axis_direction x Z_m)` — so the coordinates carry unchanged.
            let centre = [a, dz * y_k - dy * z_k];
            let cavity = Cavity::polygon(
                record
                    .polygon_mm
                    .iter()
                    .map(|[x, y]| Point2::new(centre[0] + x, centre[1] + y))
                    .collect(),
            );
            let ordinal = u16::try_from(record.ordinal)
                .ok()
                .filter(|ordinal| *ordinal <= 999)
                .ok_or_else(|| {
                    pending_member_holes("the record ordinal exceeds the kernel capacity")
                })?;
            MemberHoleCutter::directed(cavity, dy, dz, from, to, ordinal)
        };
        let applied =
            member_through_hole(&hole_body, &cutter).map_err(|failure| record_refusal(&failure))?;
        let result = permuted_body(applied.body(), &back)?;
        let volume = applied.volume_mm3().midpoint();
        solid = Some(
            SolidMesh::from_admitted(result.mesh(), result.face_table().to_vec(), volume)
                .ok_or_else(|| pending_member_holes("the holed mesh could not be admitted"))?,
        );
        element_body = result;
    }
    let Some(solid) = solid else {
        return Err(pending_member_holes("the member has no hole records"));
    };
    Ok(solid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::member::{MemberBody, ProfileInfo, evaluate_member};
    use crate::model::command::{MemberHoleKind, MemberKind, RemoveHole};
    use crate::model::section::{ParametricSection, SectionFamily, SectionShape};
    use beamcraft_model::command::Placement;
    use beamcraft_model::digest::ContentDigest;
    use beamcraft_model::id::{ElementId, OpId};

    fn id(byte: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{byte:02x}")).unwrap()
    }

    fn op(byte: u8) -> OpId {
        OpId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{byte:02x}")).unwrap()
    }

    fn digest(text: &[u8]) -> ContentDigest {
        use beamcraft_model::digest::Sha256;
        Sha256::digest(text).unwrap()
    }

    /// A straight member along the element X axis, section `width` (element Y) by `height`
    /// (element Z). Its frame axes are exact coordinate axes.
    fn member() -> AuthoredMember {
        AuthoredMember {
            id: id(9),
            kind: MemberKind::Beam,
            name: "Member".into(),
            class: 0,
            curved: false,
            axis_points_mm: vec![[0.0, 0.0, 0.0], [1000.0, 0.0, 0.0]],
            follow: None,
            profile: digest(b"profile"),
            roll_rad: None,
            section_placement: None,
            placement: Placement::IDENTITY,
            material: None,
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    fn rectangle(width: f64, height: f64) -> ProfileInfo {
        ProfileInfo::Rectangle {
            width_mm: width,
            height_mm: height,
            declared_area_mm2: None,
        }
    }

    fn member_hole(diameter: f64, axis_point: [f64; 3], direction: [f64; 3]) -> AddHole {
        AddHole {
            target_id: id(9),
            operation_id: op(1),
            hole: HoleSpec::Member(crate::model::command::MemberHole {
                axis_point_mm: axis_point,
                axis_direction: direction,
                extent: HoleExtent::Through,
                diameter_mm: diameter,
            }),
        }
    }

    fn evaluate(authored: &AuthoredMember, width: f64, height: f64) -> MemberBody {
        evaluate_member(authored, rectangle(width, height)).unwrap()
    }

    #[test]
    fn an_admitted_record_takes_the_next_ordinal_and_a_used_id_is_refused() {
        let head = member();
        let none = |_: OpId| false;
        let outcome = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &head,
            &none,
            0,
        )
        .unwrap();
        assert_eq!(outcome.ordinal, 0);
        assert_eq!(outcome.authored.hole_records.len(), 1);
        assert_eq!(outcome.authored.hole_records[0].polygon_mm.len() % 4, 0);
        let used = |id: OpId| id == op(1);
        assert_eq!(
            add_member_hole_after(
                &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
                &outcome.authored,
                &used,
                1
            )
            .unwrap_err()
            .code,
            ErrorCode::DuplicateId
        );
        // A polygon generator is refused on a member target.
        let polygon = AddHole {
            hole: HoleSpec::Round {
                centre_mm: [0.0, 0.0],
                diameter_mm: 20.0,
            },
            ..member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0])
        };
        assert_eq!(
            add_member_hole_after(&polygon, &head, &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        assert_eq!(
            add_member_hole_after(
                &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
                &member(),
                &none,
                1000
            )
            .unwrap_err()
            .code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn update_keeps_identity_and_ordinal_and_remove_restores_the_state() {
        let head = member();
        let none = |_: OpId| false;
        let added = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &head,
            &none,
            0,
        )
        .unwrap();
        let edit = UpdateHole {
            hole: member_hole(40.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
        };
        let updated = update_member_hole(&edit, &added.authored).unwrap();
        assert_eq!(updated.ordinal, added.ordinal);
        assert_eq!(updated.authored.hole_records[0].operation_id, op(1));
        let removal = RemoveHole {
            target_id: id(9),
            operation_id: op(1),
        };
        let removed = remove_member_hole(&removal, &updated.authored).unwrap();
        assert_eq!(removed.ordinal, added.ordinal);
        assert!(removed.authored.hole_records.is_empty());
        assert!(remove_member_hole(&removal, &removed.authored).is_err());
    }

    #[test]
    fn a_group_owned_record_refuses_update_and_removal() {
        // A generated hole belongs to its bolt group (the F06 member-hole amendment, bolt
        // amendment section 5): both member-hole verbs refuse it with DEPENDENCY_CONFLICT
        // under DV-REL-009, the guard the plate records of A14.4 already carry.
        let head = member();
        let none = |_: OpId| false;
        let mut owned = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &head,
            &none,
            0,
        )
        .unwrap()
        .authored;
        owned.hole_records[0].owner = Some(id(5));
        let edit = UpdateHole {
            hole: member_hole(40.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
        };
        let refused = update_member_hole(&edit, &owned).unwrap_err();
        assert_eq!(refused.code, ErrorCode::DependencyConflict);
        assert_eq!(refused.rule_id, Some("DV-REL-009"));
        let removal = RemoveHole {
            target_id: id(9),
            operation_id: op(1),
        };
        let refused = remove_member_hole(&removal, &owned).unwrap_err();
        assert_eq!(refused.code, ErrorCode::DependencyConflict);
        assert_eq!(refused.rule_id, Some("DV-REL-009"));
    }

    #[test]
    fn a_through_hole_cuts_the_certified_body_and_volume_and_undo_agree() {
        let (width, height, length) = (200.0, 300.0, 1000.0);
        let head = member();
        let gross = evaluate(&head, width, height).volume_mm3;
        // The member frame of an X-axis member: X_r = element +Y, Y_r = element +Z, Z_m = +X.
        // A hole along X_r through the section centre removes area x width.
        let authored = add_member_hole_after(
            &member_hole(100.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &head,
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let body = evaluate(&authored, width, height);
        let polygon = authored.hole_records[0].polygon_mm.clone();
        let area: f64 = {
            let mut twice = 0.0;
            for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
                twice += a[0] * b[1] - b[0] * a[1];
            }
            (twice / 2.0).abs()
        };
        let expected = width * height * length - area * width;
        assert!((body.volume_mm3 - expected).abs() <= 1.0);
        assert!(
            body.solid
                .scene_mesh()
                .face_table
                .iter()
                .any(|id| id == "cut:f.h0.w0.s0")
        );
        // A hole along Y_r removes area x height.
        let authored_y = add_member_hole_after(
            &member_hole(100.0, [0.0, 0.0, 500.0], [0.0, 1.0, 0.0]),
            &head,
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let body_y = evaluate(&authored_y, width, height);
        let expected_y = width * height * length - area * height;
        assert!((body_y.volume_mm3 - expected_y).abs() <= 1.0);
        // Undo: the removal restores the gross volume through the ordinary replay.
        let removal = RemoveHole {
            target_id: id(9),
            operation_id: op(1),
        };
        let restored = remove_member_hole(&removal, &authored).unwrap().authored;
        assert!((evaluate(&restored, width, height).volume_mm3 - gross).abs() <= 1e-9);
    }

    #[test]
    fn a_member_off_the_origin_replays_its_hole_records() {
        let (width, height) = (200.0, 300.0);
        // The same beam as `member()`, based at a non-zero offset: the frame axes stay the
        // coordinate axes and the frame origin (the first axis point) is (500, 200, 100).
        let mut placed = member();
        placed.axis_points_mm = vec![[500.0, 200.0, 100.0], [1500.0, 200.0, 100.0]];
        let gross = evaluate(&placed, width, height).volume_mm3;
        // A through hole at the section centre, mid-length: the stored record is the
        // member-frame point (u, v, w) = (0, 0, 500) and must resolve through the frame
        // origin, not read as an element coordinate (which would land the cutter near the
        // world origin, far off this member, and refuse the evaluation).
        let authored = add_member_hole_after(
            &member_hole(100.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &placed,
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let body = evaluate(&authored, width, height);
        let area = polygon_area(&authored.hole_records[0].polygon_mm);
        assert!(
            (body.volume_mm3 - (gross - area * width)).abs() <= 1.0,
            "through removed {}",
            gross - body.volume_mm3
        );
        // The same cut on the origin-based member certifies the same removed volume: the
        // record means the same local geometry, wherever the member is placed.
        let origin = add_member_hole_after(
            &member_hole(100.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &member(),
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let origin_volume = evaluate(&origin, width, height).volume_mm3;
        assert!((body.volume_mm3 - origin_volume).abs() <= 1.0);
        // A range extent is measured along the axis direction from the resolved axis point
        // too: 10 mm past both u-walls removes exactly `area x width`.
        let ranged = AddHole {
            hole: HoleSpec::Member(crate::model::command::MemberHole {
                axis_point_mm: [0.0, 0.0, 500.0],
                axis_direction: [1.0, 0.0, 0.0],
                extent: HoleExtent::Range {
                    from_mm: -110.0,
                    to_mm: 110.0,
                },
                diameter_mm: 50.0,
            }),
            ..member_hole(50.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0])
        };
        let served = add_member_hole_after(&ranged, &placed, &|_| false, 0)
            .unwrap()
            .authored;
        let body_r = evaluate(&served, width, height);
        let area_r = polygon_area(&served.hole_records[0].polygon_mm);
        assert!(
            (body_r.volume_mm3 - (gross - area_r * width)).abs() <= 1.0,
            "range removed {}",
            gross - body_r.volume_mm3
        );
    }

    #[test]
    fn a_hole_that_no_longer_fits_refuses_the_evaluation() {
        let head = member();
        // The axis point's transverse coordinate outside the section (v along Y_r is beyond
        // the 300 mm height): admission passes, the replay refuses.
        let outcome = add_member_hole_after(
            &member_hole(20.0, [0.0, 500.0, 500.0], [1.0, 0.0, 0.0]),
            &head,
            &|_| false,
            0,
        )
        .unwrap();
        let error = evaluate_member(&outcome.authored, rectangle(200.0, 300.0)).unwrap_err();
        assert!(error.message.contains("a member hole record"));
    }

    #[test]
    fn a_filleted_hollow_section_refuses_the_member_cut_with_the_standing_wall_mix_limit() {
        // The G14.2 routing is conservative: a section whose walls mix axis-aligned flats with
        // fillet arcs (a hot-rolled RHS) refuses every member record with the standing
        // `BodyWallUnsupported` refusal; lifting it is unassigned (the G14.2 note). A sharp
        // corner (a zero radius) is all-flat walls and is served; the e2e regression on the
        // sharp SHS carries the served side.
        let rhs = || ProfileInfo::Section {
            shape: SectionShape::Parametric(
                ParametricSection::new(SectionFamily::Rhs, vec![100.0, 60.0, 4.0, 8.0]).unwrap(),
            ),
            declared_area_mm2: None,
        };
        let head = member();
        let authored = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &head,
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let error = evaluate_member(&authored, rhs()).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert!(error.message.contains("no longer fits the member"));
        // The member keeps its last valid state: the uncut evaluation is unchanged.
        assert!(evaluate_member(&head, rhs()).is_ok());
    }

    #[test]
    fn member_records_compose_along_the_kernel_z_direction_only() {
        // The X-extrusion contract admits walls parallel to the member axis and walls of
        // earlier holes drilled along the kernel's Z (the section direction `v`); a record
        // along the kernel's Y (direction `u`) leaves band walls that are neither, so the
        // next record's body fails the contract (`BodyNotPrism`, the standing G14.2
        // limitation). A cut and a hole beside each other compose when both run along `v`.
        let long = AuthoredMember {
            axis_points_mm: vec![[0.0, 0.0, 0.0], [6000.0, 0.0, 0.0]],
            ..member()
        };
        let with_hole = add_member_hole_after(
            &member_hole(40.0, [0.0, 0.0, 4500.0], [0.0, 1.0, 0.0]),
            &long,
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let holed = evaluate_member(&with_hole, rectangle(200.0, 300.0)).unwrap();
        let cut = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 3000.0], [0.0, 1.0, 0.0]),
            &with_hole,
            &|_| false,
            1,
        )
        .unwrap()
        .authored;
        let both = evaluate_member(&cut, rectangle(200.0, 300.0)).unwrap();
        // The removed volumes add: the N-gon hole through 200 mm plus the 20 x 20 slot
        // through the same 200 mm.
        assert!(both.volume_mm3 < holed.volume_mm3 - 70_000.0);
        // The same pair along `u` composes no second record.
        let along_u = add_member_hole_after(
            &member_hole(40.0, [0.0, 0.0, 4500.0], [1.0, 0.0, 0.0]),
            &long,
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let cut_u = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 3000.0], [1.0, 0.0, 0.0]),
            &along_u,
            &|_| false,
            1,
        )
        .unwrap()
        .authored;
        let error = evaluate_member(&cut_u, rectangle(200.0, 300.0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert!(error.message.contains("not a straight prism"));
    }

    #[test]
    fn the_unserved_member_frames_refuse_with_the_typed_error() {
        let width = 200.0;
        let height = 300.0;
        // A roll that moves the frame off the coordinate axes.
        let mut rolled = member();
        rolled.roll_rad = Some(0.5);
        rolled.hole_records.push(MemberHoleRecord {
            operation_id: op(2),
            ordinal: 0,
            kind: MemberHoleKind::Hole,
            axis_point_mm: [0.0, 0.0, 500.0],
            axis_direction: [1.0, 0.0, 0.0],
            extent: HoleExtent::Through,
            polygon_mm: member_hole(20.0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0])
                .polygon()
                .unwrap(),
            owner: None,
        });
        let error = evaluate_member(&rolled, rectangle(width, height)).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert!(error.message.contains("without roll"));
        assert!(!error.message.contains("G14.1"));
        // A non-axis-aligned member axis.
        let mut skewed = member();
        skewed.axis_points_mm = vec![[0.0, 0.0, 0.0], [1000.0, 300.0, 0.0]];
        skewed.hole_records = rolled.hole_records.clone();
        assert!(evaluate_member(&skewed, rectangle(width, height)).is_err());
    }

    /// The shoelace area of one authored hole polygon (clockwise, so the sum is negative).
    fn polygon_area(polygon: &[[f64; 2]]) -> f64 {
        let mut twice = 0.0;
        for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
            twice += a[0] * b[1] - b[0] * a[1];
        }
        (twice / 2.0).abs()
    }

    #[test]
    fn an_oblique_hole_direction_cuts_with_the_certified_volume() {
        let (width, height) = (200.0, 300.0);
        let head = member();
        let gross = evaluate(&head, width, height).volume_mm3;
        // Direction (3, 4) across the section centre: the cutter crosses both u-walls
        // obliquely and removes `area x width / |n . d|` with |n . d| = 3/5. The diameter
        // stays at 16 because the two opposite u-faces overlap in the footprint coordinate
        // only `2 * (3/5 * 150 - 4/5 * 100)` = 20 mm for this direction, so a wider hole
        // would breach the faces' hard edges and the replay would refuse it.
        let authored = add_member_hole_after(
            &member_hole(16.0, [0.0, 0.0, 500.0], [3.0, 4.0, 0.0]),
            &head,
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let body = evaluate(&authored, width, height);
        let area = polygon_area(&authored.hole_records[0].polygon_mm);
        let expected = gross - area * width * 5.0 / 3.0;
        assert!((body.volume_mm3 - expected).abs() <= 1.0);
        assert!(
            body.solid
                .scene_mesh()
                .face_table
                .iter()
                .any(|id| id == "cut:f.h0.w0.s0")
        );
    }

    #[test]
    fn a_non_unit_direction_normalizes_to_the_same_cut() {
        // (6, 8) normalizes to the same line as (3, 4): both quotients are the same doubles
        // (6/10 and 3/5 round to one 0.6), so the magnitude names nothing and the evaluated
        // bodies must be equal.
        let unit = add_member_hole_after(
            &member_hole(16.0, [0.0, 0.0, 500.0], [3.0, 4.0, 0.0]),
            &member(),
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let scaled = add_member_hole_after(
            &member_hole(16.0, [0.0, 0.0, 500.0], [6.0, 8.0, 0.0]),
            &member(),
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        assert_eq!(
            evaluate(&unit, 200.0, 300.0),
            evaluate(&scaled, 200.0, 300.0)
        );
    }

    #[test]
    fn a_ranged_extent_cuts_through_and_a_blind_one_refuses() {
        let (width, height) = (200.0, 300.0);
        let head = member();
        let gross = evaluate(&head, width, height).volume_mm3;
        let ranged = |from_mm: f64, to_mm: f64| AddHole {
            hole: HoleSpec::Member(crate::model::command::MemberHole {
                axis_point_mm: [0.0, 0.0, 500.0],
                axis_direction: [1.0, 0.0, 0.0],
                extent: HoleExtent::Range { from_mm, to_mm },
                diameter_mm: 50.0,
            }),
            ..member_hole(50.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0])
        };
        // The range reaches 10 mm past both u-walls, so it cuts exactly `area x width`.
        let served = add_member_hole_after(&ranged(-110.0, 110.0), &head, &|_| false, 0)
            .unwrap()
            .authored;
        let body = evaluate(&served, width, height);
        let area = polygon_area(&served.hole_records[0].polygon_mm);
        assert!((body.volume_mm3 - (gross - area * width)).abs() <= 1.0);
        // A range that ends 50 mm inside the material is the kernel's blind refusal.
        let blind = add_member_hole_after(&ranged(-110.0, -50.0), &head, &|_| false, 0)
            .unwrap()
            .authored;
        let error = evaluate_member(&blind, rectangle(width, height)).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert!(error.message.contains("range ends inside"));
    }

    #[test]
    fn the_admission_matrix_for_the_hole_direction() {
        let hole = |direction: [f64; 3]| crate::model::command::MemberHole {
            axis_point_mm: [0.0, 0.0, 500.0],
            axis_direction: direction,
            extent: HoleExtent::Through,
            diameter_mm: 20.0,
        };
        for direction in [
            [3.0, 4.0, 0.0],
            [6.0, 8.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
        ] {
            assert!(
                hole(direction).validate().is_ok(),
                "{direction:?} names a served section direction"
            );
        }
        assert_eq!(
            hole([0.0, 0.0, 1.0]).validate().unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
        assert_eq!(
            hole([0.0, 0.0, 0.0]).validate().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            hole([f64::NAN, 1.0, 0.0]).validate().unwrap_err().code,
            ErrorCode::NonFiniteNumber
        );
        assert_eq!(
            hole([1e300, 0.0, 0.0]).validate().unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn a_chs_member_hole_certifies_the_removed_volume_within_the_engine_bound() {
        let chs = || ProfileInfo::Section {
            shape: SectionShape::Parametric(
                ParametricSection::new(SectionFamily::Chs, vec![114.0, 5.0]).unwrap(),
            ),
            declared_area_mm2: None,
        };
        let head = member();
        // The gross stays the uncut P-Mass volume across every hole of the triplet.
        let gross_mm3 = evaluate_member(&head, chs()).unwrap().volume_mm3;
        for diameter in [6.0, 10.0, 20.0] {
            let authored = add_member_hole_after(
                &member_hole(diameter, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
                &head,
                &|_| false,
                0,
            )
            .unwrap()
            .authored;
            let body = evaluate_member(&authored, chs()).unwrap();
            // The measured rule: the drop is the mesh before the hole minus the mesh after,
            // both belonging to the faceted outline, so it certifies against the faceted
            // value (`from_section` stores that polygon's analytic volume, the F06 regular
            // 32-gon hole area times both wall crossings).
            let drop = gross_mm3 - body.volume_mm3;
            let area = polygon_area(&authored.hole_records[0].polygon_mm);
            let expected = 2.0 * area * 5.0;
            assert!(
                (drop - expected).abs() <= 0.12 * expected,
                "d={diameter}: {drop} vs {expected}"
            );
            // The analytic bound of the smooth tube: the hole circle area through both walls
            // of the 5 mm ring. The drop must never leave it (the old constant did).
            let bound = 2.0 * std::f64::consts::PI * (diameter / 2.0) * (diameter / 2.0) * 5.0;
            assert!(drop <= bound, "d={diameter}: {drop} past {bound}");
            // The reported triple agrees with the drop, and the gross stays the nominal one.
            assert!((body.removed_volume_mm3 - drop).abs() <= 1e-6 * drop);
            let quantities = crate::engine::member::member_quantities(&body, None).unwrap();
            assert!(
                (quantities.volume_gross_mm3 - gross_mm3).abs() <= 1e-9 * gross_mm3,
                "d={diameter}: gross {} vs {gross_mm3}",
                quantities.volume_gross_mm3
            );
            assert!((quantities.volume_mm3 - body.volume_mm3).abs() <= 1e-9 * gross_mm3);
            // The replay is byte-stable: a second evaluation returns the same body.
            let again = evaluate_member(&authored, chs()).unwrap();
            assert_eq!(body, again);
            assert!(
                body.solid
                    .scene_mesh()
                    .face_table
                    .iter()
                    .any(|id| id == "cut:f.h0.w0.s0")
            );
        }
    }

    #[test]
    fn a_degenerate_direction_record_refuses_the_replay() {
        for direction in [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]] {
            let mut authored = member();
            authored.hole_records.push(MemberHoleRecord {
                operation_id: op(2),
                ordinal: 0,
                kind: MemberHoleKind::Hole,
                axis_point_mm: [0.0, 0.0, 500.0],
                axis_direction: direction,
                extent: HoleExtent::Through,
                polygon_mm: member_hole(20.0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0])
                    .polygon()
                    .unwrap(),
                owner: None,
            });
            let error = evaluate_member(&authored, rectangle(200.0, 300.0)).unwrap_err();
            assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        }
    }

    #[test]
    fn a_holed_member_keeps_its_named_features() {
        // The G14.2 cutter's band ids carry the amendment's `.w<k>` crossing (and `.p<run>`
        // suffixes on the merge walk). The feature-id grammar must admit them, or the named
        // feature layer of the holed member dies whole and every pick, dimension anchor and
        // drawing reference to the surviving faces loses its address.
        let authored = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &member(),
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let body = evaluate(&authored, 100.0, 50.0);
        let snapshot = beamcraft_engine::query::snapshot::ViewSnapshot::build(
            beamcraft_engine::query::snapshot::ViewPin::new(
                beamcraft_model::digest::Sha256::digest(b"member-hole-features").unwrap(),
                1,
            ),
            vec![beamcraft_engine::scene::SceneElement::new(
                id(9),
                beamcraft_engine::element::ElementKind::Column,
                beamcraft_geom::Translation3::new(0.0, 0.0, 0.0).unwrap(),
                body.solid.scene_mesh(),
            )],
            &beamcraft_engine::limits::Limits::default(),
        )
        .unwrap();
        let features = beamcraft_engine::query::feature::list_features(&snapshot, &id(9)).unwrap();
        assert!(
            features
                .iter()
                .any(|f| f.to_string() == format!("fid1:{}:member:f.x0", id(9)))
        );
        let corner = beamcraft_engine::query::feature::lookup_feature(
            &snapshot,
            &format!("fid1:{}:member:v.x0.y0.z0", id(9)),
        )
        .unwrap();
        assert!(matches!(
            corner.world,
            beamcraft_engine::query::feature::FeatureGeometry::Vertex { .. }
        ));
        let band = beamcraft_engine::query::feature::lookup_feature(
            &snapshot,
            &format!("fid1:{}:cut:f.h0.w0.s0", id(9)),
        )
        .unwrap();
        assert!(matches!(
            band.world,
            beamcraft_engine::query::feature::FeatureGeometry::Surface { .. }
        ));
    }

    /// The view snapshot of one evaluated member body at the identity placement.
    fn member_snapshot(body: &MemberBody) -> beamcraft_engine::query::snapshot::ViewSnapshot {
        beamcraft_engine::query::snapshot::ViewSnapshot::build(
            beamcraft_engine::query::snapshot::ViewPin::new(
                beamcraft_model::digest::Sha256::digest(b"member-cut-features").unwrap(),
                1,
            ),
            vec![beamcraft_engine::scene::SceneElement::new(
                id(9),
                beamcraft_engine::element::ElementKind::Column,
                beamcraft_geom::Translation3::new(0.0, 0.0, 0.0).unwrap(),
                body.solid.scene_mesh(),
            )],
            &beamcraft_engine::limits::Limits::default(),
        )
        .unwrap()
    }

    /// The world position of a looked-up vertex feature.
    fn vertex_of(
        snapshot: &beamcraft_engine::query::snapshot::ViewSnapshot,
        text: &str,
    ) -> [f64; 3] {
        let found = beamcraft_engine::query::feature::lookup_feature(snapshot, text).unwrap();
        match found.world {
            beamcraft_engine::query::feature::FeatureGeometry::Vertex { position } => position,
            _ => panic!("vertex geometry expected"),
        }
    }

    fn near(got: [f64; 3], want: [f64; 3]) {
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-6, "{got:?} != {want:?}");
        }
    }

    #[test]
    fn a_holed_member_names_its_rim_vertices_and_edges_at_known_coordinates() {
        // A d=20 through hole along X_r (element +Y) at the member's mid-length: one
        // crossing, the entry ring on `member:f.x0` (element y = -100) and the exit ring on
        // `member:f.x1` (+100). The cavity's hole frame puts x along the member axis
        // (element X) and y along `direction x Z_m` (element -Z), and ring point 0 sits at
        // angle 0, so every expected position below follows from the F06 frame rules and
        // the generator's canonical 32-gon ring (r = 10: the 0.05 mm chord bound gives 32).
        let authored = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &member(),
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let body = evaluate(&authored, 200.0, 300.0);
        let snapshot = member_snapshot(&body);
        use beamcraft_engine::query::feature::{FeatureId, list_features};
        let names: Vec<String> = list_features(&snapshot, &id(9))
            .unwrap()
            .iter()
            .map(|f| f.local_id())
            .collect();
        for local in [
            "cut:v.h0.w0.x0.0",
            "cut:v.h0.w0.x0.1",
            "cut:v.h0.w0.x1.0",
            "cut:e.h0.w0.s0.x0",
            "cut:e.h0.w0.s0.x1",
            "cut:e.h0.w0.v0",
        ] {
            assert!(
                names.iter().any(|n| n == local),
                "{local} missing: {names:?}"
            );
        }
        // Ring point 0 at angle 0: the +x side of the hole frame, 10 mm from the axis.
        let entry = vertex_of(&snapshot, &format!("fid1:{}:cut:v.h0.w0.x0.0", id(9)));
        near(entry, [510.0, -100.0, 0.0]);
        let exit = vertex_of(&snapshot, &format!("fid1:{}:cut:v.h0.w0.x1.0", id(9)));
        near(exit, [510.0, 100.0, 0.0]);
        // Ring point 1 walks clockwise: hole-frame (10cos, -10sin) is element +z.
        let angle = core::f64::consts::TAU / 32.0;
        let (c, s) = (angle.cos(), angle.sin());
        near(
            vertex_of(&snapshot, &format!("fid1:{}:cut:v.h0.w0.x0.1", id(9))),
            [500.0 + 10.0 * c, -100.0, 10.0 * s],
        );
        // The corner edge runs along the hole axis between the two rings of point 0.
        let corner = beamcraft_engine::query::feature::lookup_feature(
            &snapshot,
            &format!("fid1:{}:cut:e.h0.w0.v0", id(9)),
        )
        .unwrap();
        let beamcraft_engine::query::feature::FeatureGeometry::Edge { ends, .. } = corner.world
        else {
            panic!("edge geometry expected");
        };
        // The two rings of point 0: entry and exit, in either end order.
        let on_ring = |end: [f64; 3], y: f64| {
            (end[0] - 510.0).abs() < 1e-6 && (end[1] - y).abs() < 1e-6 && end[2].abs() < 1e-6
        };
        assert!(
            (on_ring(ends[0], -100.0) && on_ring(ends[1], 100.0))
                || (on_ring(ends[0], 100.0) && on_ring(ends[1], -100.0)),
            "{ends:?}"
        );
        // Snap to the rim vertex, to the nearest point of the corner edge and to its
        // midpoint: the same vertex/edge/midpoint surfaces the box world uses.
        use beamcraft_engine::query::pick::{SnapKind, SnapQuery, snap};
        let snapped = snap(
            &snapshot,
            &SnapQuery {
                point_mm: [510.0, -100.0, 0.0],
                radius_mm: 0.01,
                kinds: vec![SnapKind::Vertex],
                max_hits: 16,
            },
        )
        .unwrap();
        assert_eq!(snapped.candidates.len(), 1);
        assert_eq!(
            snapped.candidates[0].feature_id,
            Some(FeatureId::parse(&format!("fid1:{}:cut:v.h0.w0.x0.0", id(9))).unwrap())
        );
        // The nearest point and the midpoint of the corner edge: the query sits on the v0
        // edge's side of the opening, clear of the neighbouring corners (points 1 and 31).
        let snapped = snap(
            &snapshot,
            &SnapQuery {
                point_mm: [510.2, 0.0, 0.0],
                radius_mm: 1.0,
                kinds: vec![SnapKind::Edge],
                max_hits: 16,
            },
        )
        .unwrap();
        assert_eq!(snapped.candidates.len(), 1);
        assert_eq!(
            snapped.candidates[0]
                .feature_id
                .as_ref()
                .map(FeatureId::local_id)
                .as_deref(),
            Some("cut:e.h0.w0.v0")
        );
        near(snapped.candidates[0].point_world_mm, [510.0, 0.0, 0.0]);
        let snapped = snap(
            &snapshot,
            &SnapQuery {
                point_mm: [510.2, 0.0, 0.0],
                radius_mm: 1.0,
                kinds: vec![SnapKind::Midpoint],
                max_hits: 16,
            },
        )
        .unwrap();
        assert_eq!(snapped.candidates.len(), 1);
        assert_eq!(
            snapped.candidates[0]
                .feature_id
                .as_ref()
                .map(FeatureId::local_id)
                .as_deref(),
            Some("cut:e.h0.w0.v0")
        );
        near(snapped.candidates[0].point_world_mm, [510.0, 0.0, 0.0]);

        // A point pick near the entry ring names the rim vertex.
        use beamcraft_engine::query::feature::FeatureKind;
        use beamcraft_engine::query::pick::{PickQuery, PointPick, pick};
        let mut query = PointPick::new([510.0, -101.0, 0.0], vec![FeatureKind::Vertex]);
        query.radius_mm = 2.0;
        query.max_hits = 16;
        let picked = pick(&snapshot, &PickQuery::Point(query)).unwrap();
        assert_eq!(picked.hits.len(), 1);
        assert_eq!(
            picked.hits[0]
                .feature_id
                .as_ref()
                .map(FeatureId::local_id)
                .as_deref(),
            Some("cut:v.h0.w0.x0.0")
        );
    }

    #[test]
    fn a_plane_cut_member_names_no_cut_edges_and_a_cut_edge_id_refuses_explicitly() {
        // The plane cut names its cap corners (the cap-corner amendment) but no cut edges:
        // this slice names only the walls of prism cuts. A well-formed cut edge id over
        // such a member resolves as explicitly not found, never as another feature.
        let mut authored = member();
        authored.cuts.push(crate::model::command::CutRecord {
            operation_id: op(2),
            ordinal: 0,
            point_mm: [500.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            keep: crate::model::command::KeepSide::Negative,
        });
        let body = evaluate(&authored, 200.0, 300.0);
        let snapshot = member_snapshot(&body);
        use beamcraft_engine::error::ViewCode;
        use beamcraft_engine::query::feature::{
            BodyKind, FeatureKind, list_features, lookup_feature,
        };
        let features = list_features(&snapshot, &id(9)).unwrap();
        assert!(
            features
                .iter()
                .any(|f| f.local_id().starts_with("cut:v.p0."))
        );
        assert!(
            !features
                .iter()
                .any(|f| f.kind() == FeatureKind::Edge && f.body() == BodyKind::Cut)
        );
        let error =
            lookup_feature(&snapshot, &format!("fid1:{}:cut:e.h0.w0.v0", id(9))).unwrap_err();
        assert_eq!(error.code(), ViewCode::FeatureNotFound);
    }

    #[test]
    fn two_records_cut_in_ordinal_order_and_the_replay_repeats_byte_for_byte() {
        let (width, height) = (200.0, 300.0);
        let head = member();
        let gross = evaluate(&head, width, height).volume_mm3;
        // Two separated through holes on one member, both along Y_r: the second record
        // replays over the already-holed body in ordinal order, so the expected net is
        // the exact closed form `(area0 + area1) x height` removed from the gross. The
        // positions keep the two footprints clear of each other on every wall.
        let first = add_member_hole_after(
            &member_hole(40.0, [0.0, 0.0, 500.0], [0.0, 1.0, 0.0]),
            &head,
            &|_| false,
            0,
        )
        .unwrap();
        let second_command = member_hole(30.0, [60.0, 0.0, 300.0], [0.0, 1.0, 0.0]);
        let with_second_id = AddHole {
            operation_id: op(2),
            ..second_command
        };
        let both = add_member_hole_after(&with_second_id, &first.authored, &|_| false, 1)
            .unwrap()
            .authored;
        assert_eq!(
            both.hole_records
                .iter()
                .map(|r| r.ordinal)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        let body = evaluate(&both, width, height);
        let area_first = polygon_area(&both.hole_records[0].polygon_mm);
        let area_second = polygon_area(&both.hole_records[1].polygon_mm);
        let expected = gross - (area_first + area_second) * height;
        assert!(
            (body.volume_mm3 - expected).abs() <= 1.0,
            "two holes removed {}",
            gross - body.volume_mm3
        );
        // Every record names its own wall bands in the feature table, in its own ordinal.
        let faces = body.solid.scene_mesh().face_table;
        assert!(faces.iter().any(|f| f == "cut:f.h0.w0.s0"));
        assert!(faces.iter().any(|f| f == "cut:f.h1.w0.s0"));
        // The replay is byte-stable: a second evaluation returns the same body.
        assert_eq!(body, evaluate(&both, width, height));
    }

    #[test]
    fn a_record_meeting_another_hole_refuses_the_evaluation() {
        let head = member();
        // Two concentric through holes along the same direction: the second footprint
        // contains the first's opening, and a fit condition is "clear of its edges and
        // the other holes". Admission passes (distinct ids, fresh ordinals); the replay
        // must refuse the second record instead of serving a merged cut, and the
        // evaluation fails whole, so the member keeps its last valid state.
        let first = add_member_hole_after(
            &member_hole(40.0, [0.0, 0.0, 500.0], [0.0, 1.0, 0.0]),
            &head,
            &|_| false,
            0,
        )
        .unwrap();
        let overlapping = AddHole {
            operation_id: op(2),
            ..member_hole(100.0, [0.0, 0.0, 500.0], [0.0, 1.0, 0.0])
        };
        let authored = add_member_hole_after(&overlapping, &first.authored, &|_| false, 1)
            .unwrap()
            .authored;
        let error = evaluate_member(&authored, rectangle(200.0, 300.0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert!(
            error
                .message
                .contains("clear of its edges and the other holes"),
            "the overlap refused as: {}",
            error.message
        );
    }

    #[test]
    fn mixed_record_directions_refuse_the_replay_after_the_first_cut() {
        let head = member();
        // One hole along X_r and one along Y_r: the first cut's walls are not parallel
        // to the second cutter's axis, so the kernel cannot serve the second record off
        // the already-holed body and the replay refuses the evaluation whole. The
        // authored records stay valid; only the mixed set has no servable replay.
        let first = add_member_hole_after(
            &member_hole(40.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &head,
            &|_| false,
            0,
        )
        .unwrap();
        let transverse = AddHole {
            operation_id: op(2),
            ..member_hole(30.0, [0.0, 0.0, 300.0], [0.0, 1.0, 0.0])
        };
        let authored = add_member_hole_after(&transverse, &first.authored, &|_| false, 1)
            .unwrap()
            .authored;
        let error = evaluate_member(&authored, rectangle(200.0, 300.0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert!(
            error
                .message
                .contains("the body is not a straight prism for the kernel"),
            "the mixed set refused as: {}",
            error.message
        );
    }

    #[test]
    fn the_member_admission_refusals_pin_their_typed_errors() {
        let head = member();
        let none = |_: OpId| false;
        // A hole command naming another element is a NotFound on the target, on every
        // member-hole verb.
        let elsewhere = AddHole {
            target_id: id(8),
            ..member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0])
        };
        assert_eq!(
            add_member_hole_after(&elsewhere, &head, &none, 0)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        let removal = RemoveHole {
            target_id: id(8),
            operation_id: op(1),
        };
        assert_eq!(
            remove_member_hole(&removal, &head).unwrap_err().code,
            ErrorCode::NotFound
        );
        // An update naming a record the target does not hold is a NotFound on the id.
        let unknown = UpdateHole {
            hole: member_hole(40.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
        };
        assert_eq!(
            update_member_hole(&unknown, &head).unwrap_err().code,
            ErrorCode::NotFound
        );
        // The adapter's persistent counter never regresses: a pinned ordinal below the
        // last live record's is an InvalidArgument.
        let mut advanced = member();
        advanced.hole_records.push(MemberHoleRecord {
            operation_id: op(3),
            ordinal: 5,
            kind: MemberHoleKind::Hole,
            axis_point_mm: [0.0, 0.0, 500.0],
            axis_direction: [1.0, 0.0, 0.0],
            extent: HoleExtent::Through,
            polygon_mm: member_hole(20.0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0])
                .polygon()
                .unwrap(),
            owner: None,
        });
        assert_eq!(
            add_member_hole_after(
                &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
                &advanced,
                &none,
                3
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        // The per-member capacity is 64 records: the 65th is a LimitExceeded under the
        // shared DV-LIM-001 rule, before any replay runs.
        let mut full = member();
        let base = MemberHoleRecord {
            operation_id: op(4),
            ordinal: 0,
            kind: MemberHoleKind::Hole,
            axis_point_mm: [0.0, 0.0, 500.0],
            axis_direction: [1.0, 0.0, 0.0],
            extent: HoleExtent::Through,
            polygon_mm: member_hole(20.0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0])
                .polygon()
                .unwrap(),
            owner: None,
        };
        full.hole_records = (0..MAX_HOLES)
            .map(|i| MemberHoleRecord {
                operation_id: op(i as u8 + 10),
                ordinal: i,
                ..base.clone()
            })
            .collect();
        let overflow = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &full,
            &none,
            MAX_HOLES,
        )
        .unwrap_err();
        assert_eq!(overflow.code, ErrorCode::LimitExceeded);
        assert_eq!(overflow.rule_id, Some("DV-LIM-001"));
        // A curved member or a polybeam has no straight body to replay the records on:
        // the refusal names the straight-member requirement (the admission passed).
        let mut curved = advanced.clone();
        curved.curved = true;
        let error = evaluate_member(&curved, rectangle(200.0, 300.0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert!(error.message.contains("straight member"));
        let mut polybeam = advanced;
        polybeam.axis_points_mm = vec![[0.0, 0.0, 0.0], [500.0, 0.0, 0.0], [1000.0, 0.0, 0.0]];
        let error = evaluate_member(&polybeam, rectangle(200.0, 300.0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert!(error.message.contains("straight member"));
    }
}
