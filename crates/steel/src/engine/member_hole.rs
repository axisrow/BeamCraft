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
    AddHole, HoleExtent, HoleSpec, MAX_HOLES, MemberHoleRecord, UpdateHole,
};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::solid::SolidMesh;
use beamcraft_geom::Frame3;
use beamcraft_kernel::ops::{Body, OpCode, OpDefect, OpFailure};
use beamcraft_kernel::profile::{Cavity, Point2};
use beamcraft_model::command::CommandError;
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
        // The hole axis: normalized exactly as the kernel normalizes its `(axis_y, axis_z)`,
        // so the footprint frame here and the kernel's are the same line bit for bit.
        let [du, dv, dw] = record.axis_direction;
        if dw != 0.0 {
            return Err(pending_member_holes(
                "a member hole axis must stay in the member's section plane",
            ));
        }
        let norm = (du * du + dv * dv).sqrt();
        if !(norm.is_finite() && norm > 0.0) {
            return Err(pending_member_holes(
                "a member hole axis direction needs a nonzero transverse part",
            ));
        }
        let (dy, dz) = (du / norm, dv / norm);
        let hole_body = permuted_body(&element_body, &axes_kernel)?;
        // The cutter range along the axis: `through` spans the body's full height past both
        // walls; a range runs from the axis point along the axis (the model refuses from >= to,
        // and a range ending inside a wall is the kernel's blind refusal).
        let height = |p: &[f64; 3]| p[1].mul_add(dy, p[2] * dz);
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
        let cutter = MemberHoleCutter::directed(cavity, dy, dz, from, to, ordinal);
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
    use crate::model::command::{MemberKind, RemoveHole};
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
    fn the_unserved_member_frames_refuse_with_the_typed_error() {
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
    fn a_chs_member_hole_cuts_the_faceted_tube_walls() {
        let chs = || ProfileInfo::Section {
            shape: SectionShape::Parametric(
                ParametricSection::new(SectionFamily::Chs, vec![114.0, 5.0]).unwrap(),
            ),
            declared_area_mm2: None,
        };
        let head = member();
        let gross = evaluate_member(&head, chs()).unwrap();
        let authored = add_member_hole_after(
            &member_hole(20.0, [0.0, 0.0, 500.0], [1.0, 0.0, 0.0]),
            &head,
            &|_| false,
            0,
        )
        .unwrap()
        .authored;
        let body = evaluate_member(&authored, chs()).unwrap();
        // The certified replay volume belongs to the faceted outline, so the drop is
        // measured against the same baseline (`from_section` stores the polygon's analytic
        // volume), not against the closed-form ring area of the uncut evaluation.
        let drop = gross.solid.volume_mm3() - body.volume_mm3;
        let area = polygon_area(&authored.hole_records[0].polygon_mm);
        let expected = 2.0 * area * 5.0;
        assert!(
            (drop - expected).abs() <= 0.12 * expected,
            "{drop} vs {expected}"
        );
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

    #[test]
    fn a_degenerate_direction_record_refuses_the_replay() {
        for direction in [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]] {
            let mut authored = member();
            authored.hole_records.push(MemberHoleRecord {
                operation_id: op(2),
                ordinal: 0,
                axis_point_mm: [0.0, 0.0, 500.0],
                axis_direction: direction,
                extent: HoleExtent::Through,
                polygon_mm: member_hole(20.0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0])
                    .polygon()
                    .unwrap(),
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
}
