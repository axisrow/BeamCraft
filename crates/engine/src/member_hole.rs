// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F04 command application for member holes (A04.2, the F06 member-hole amendment): one owned
//! member-hole record appended to, replaced in or removed from a valid member, and the replay
//! of the member's records over the evaluated body.
//!
//! The engine never touches USD. The commands check the record admission the command layer owes
//! (target lifecycle, record identity across the project, ordinal capacity), mutate the
//! authored state and declare the footprint; the adapter re-evaluates the body through the
//! ordinary [`crate::member::evaluate_member`] replay and commits the result as one
//! transaction. A rejection changes nothing.
//!
//! Replay cuts each record with the kernel's certified through-hole operation. The cutter is
//! mapped into the hole's own frame by an exact axis permutation, so a body whose member frame
//! is axis-aligned (unit components `0.0`/`+-1.0`, which includes every member whose axis is
//! parallel to a coordinate axis and carries no roll) is cut with bit-exact coordinates. Every
//! other case — a section profile, a polybeam or curved path, a roll that moves the frame off
//! the coordinate axes, a ranged extent, a body that is not a prism in the hole frame — is
//! refused with `UNSUPPORTED_GEOMETRY` naming the kernel slice (G14.1) that will lift it. A
//! stored record the replay can no longer serve (the member was edited, the hole no longer
//! fits) refuses the evaluation with the typed hole diagnostic, and the replay runs in ordinal
//! order, so the member keeps its last valid state as F04 section 4 says.

use crate::footprint::Footprint;
use crate::member::AuthoredMember;
use crate::solid::SolidMesh;
use beamcraft_geom::Frame3;
use beamcraft_kernel::ops::{Body, HoleCutter, OpCode, OpFailure, through_hole};
use beamcraft_kernel::profile::{Cavity, Point2};
use beamcraft_model::command::{
    AddHole, CommandError, HoleExtent, HoleSpec, MAX_HOLES, MemberHoleRecord, UpdateHole,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::OpId;

/// Millimetres a through cutter reaches past both faces of the body, as for plate holes.
const CUTTER_OVERSHOOT_MM: f64 = 1.0;

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
fn member_generator(
    command: &AddHole,
) -> Result<&beamcraft_model::command::MemberHole, CommandError> {
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
fn admit_ordinal(records: &[MemberHoleRecord], pinned: usize) -> Result<usize, CommandError> {
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
        axis_point_mm: member.axis_point_mm,
        axis_direction: member.axis_direction,
        extent: member.extent,
        polygon_mm: command.polygon()?,
    });
    Ok(MemberHoleOutcome {
        footprint: Footprint::for_update(command.target_id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
        authored: next,
        ordinal,
    })
}

/// Replaces one member hole's parameters, preserving its identity and ordinal.
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
    command: &beamcraft_model::command::RemoveHole,
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

    /// The map that applies `self` after `outer` (`self(outer(v))`): the rows of the
    /// composition combine the outer rows by the inner rows, exact for the signed unit
    /// components.
    fn after(self, outer: Self) -> Self {
        Self {
            x: combined(self.x, outer),
            y: combined(self.y, outer),
            z: combined(self.z, outer),
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

/// The inner rows combined by one exact signed outer row.
fn combined(row: [f64; 3], inner: Axes) -> [f64; 3] {
    let [a, b, c] = row;
    let part = |s: f64, r: [f64; 3]| {
        let [x, y, z] = r;
        [zeroed(s * x), zeroed(s * y), zeroed(s * z)]
    };
    let [ax, ay, az] = part(a, inner.x);
    let [bx, by, bz] = part(b, inner.y);
    let [cx, cy, cz] = part(c, inner.z);
    [ax + bx + cx, ay + by + cy, az + bz + cz]
}

/// Normalizes negative zero.
fn zeroed(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
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

/// The fixed message for one kernel refusal of a member-hole record.
fn record_refusal_message(failure: &OpFailure) -> &'static str {
    match failure.code {
        OpCode::ContourInvalid | OpCode::UnsupportedContour => {
            "a member hole record does not fit strictly inside a wall face, clear of its edges and the other holes"
        }
        OpCode::OpNoEffect | OpCode::OpUnsupported => {
            "a member hole record no longer fits the member"
        }
        OpCode::AboveLimit => "a member hole record exceeds the kernel capacity",
        _ => "the kernel refused a member hole record",
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
/// ordinal order, with the kernel's certified through-hole operation. The records come from the
/// member frame; `frame` is the member frame's axes in element coordinates. The returned mesh
/// is in element coordinates.
///
/// The first slice serves a straight member whose frame axes are exact coordinate axes and
/// whose records are `through`; everything else is the pending-kernel-slice refusal. A record
/// the kernel refuses (it left the member, reaches a free edge, meets another hole) refuses the
/// evaluation with a diagnostic naming the record, so the member keeps its last valid state.
pub(crate) fn replay_member_holes(
    frame: &Frame3,
    body: &Body,
    records: &[MemberHoleRecord],
) -> Result<SolidMesh, CommandError> {
    let (bx, by, bz) = basis_of(frame);
    let Some(axes_frame) = Axes::of_rows(bx, by, bz) else {
        return Err(pending_member_holes(
            "the member hole needs the kernel slice G14.1: the member frame is not axis-aligned in this slice",
        ));
    };
    // Each record cuts the body in its own hole frame; between records the result comes back
    // to element coordinates, so the next record maps it afresh.
    let mut solid: Option<SolidMesh> = None;
    let mut element_body = body.clone();
    for record in records {
        if !matches!(record.extent, HoleExtent::Through) {
            return Err(pending_member_holes(
                "the member hole needs the kernel slice G14.1: a ranged cutter is not certified in this slice",
            ));
        }
        // The hole frame, in member-frame coordinates: x along +Z_m, y along `axis_direction x
        // +Z_m`, z along the axis direction. The stored polygon is clockwise in this frame (the
        // plate generator's winding) and the kernel expects a clockwise cavity in body XY seen
        // from +Z. The tokens are exact, so the mapping is exact.
        let direction = record.axis_direction;
        let x2d = [0.0, 0.0, 1.0];
        let y2d = cross(&direction, &x2d);
        let Some(to_hole) = Axes::of_rows(x2d, y2d, direction) else {
            return Err(pending_member_holes(
                "the hole frame is not axis-aligned in this slice",
            ));
        };
        // Compose: element -> member frame -> hole frame.
        let full = to_hole.after(axes_frame);
        let hole_body = permuted_body(&element_body, &full)?;
        let z0 = hole_body
            .mesh()
            .vertices()
            .iter()
            .map(|p| p[2])
            .fold(f64::INFINITY, f64::min);
        let z1 = hole_body
            .mesh()
            .vertices()
            .iter()
            .map(|p| p[2])
            .fold(f64::NEG_INFINITY, f64::max);
        // The axis point in hole-frame XY: an exact signed component pick through the hole rows.
        let point = record.axis_point_mm;
        let centre = [weighted(to_hole.x, point), weighted(to_hole.y, point)];
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
        let cutter = HoleCutter::new(
            cavity,
            z0 - CUTTER_OVERSHOOT_MM,
            z1 + CUTTER_OVERSHOOT_MM,
            ordinal,
        );
        let applied =
            through_hole(&hole_body, &cutter).map_err(|failure| record_refusal(&failure))?;
        let back = full.inverse();
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
    use crate::member::{MemberBody, ProfileInfo, evaluate_member};
    use beamcraft_model::command::{MemberKind, Placement, RemoveHole};
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
            hole: HoleSpec::Member(beamcraft_model::command::MemberHole {
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
                .any(|id| id == "cut:f.h0.s0")
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
    fn pending_cases_refuse_with_the_kernel_slice_note() {
        let width = 200.0;
        let height = 300.0;
        // A roll that moves the frame off the coordinate axes.
        let mut rolled = member();
        rolled.roll_rad = Some(0.5);
        rolled.hole_records.push(MemberHoleRecord {
            operation_id: op(2),
            ordinal: 0,
            axis_point_mm: [0.0, 0.0, 500.0],
            axis_direction: [1.0, 0.0, 0.0],
            extent: HoleExtent::Through,
            polygon_mm: member_hole(20.0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0])
                .polygon()
                .unwrap(),
        });
        let error = evaluate_member(&rolled, rectangle(width, height)).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert!(error.message.contains("G14.1"));
        // A ranged record.
        let mut ranged = member();
        ranged.hole_records.push(MemberHoleRecord {
            extent: HoleExtent::Range {
                from_mm: 10.0,
                to_mm: 50.0,
            },
            ..rolled.hole_records[0].clone()
        });
        assert!(
            evaluate_member(&ranged, rectangle(width, height))
                .unwrap_err()
                .message
                .contains("G14.1")
        );
        // A non-axis-aligned member axis.
        let mut skewed = member();
        skewed.axis_points_mm = vec![[0.0, 0.0, 0.0], [1000.0, 300.0, 0.0]];
        skewed.hole_records = rolled.hole_records.clone();
        assert!(evaluate_member(&skewed, rectangle(width, height)).is_err());
    }
}
