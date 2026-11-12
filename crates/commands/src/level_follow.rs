// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D2 follow: the straight beams and columns that lie on a level when its elevation changes.
//!
//! The elements are matched by where they are, not by a stored binding (that is H01.2, #820):
//! every axis end of a straight member whose world Z equals the old elevation moves to the new
//! one, and nothing else changes. A column whose top was placed at a level stretches with it; a
//! beam on the level moves with it. Each element is patched as `update-element --axis-point`
//! patches it (its body is re-evaluated from the standing profile), and every copy that shares
//! its shape must move the same way, or the edit is refused.

use crate::definition::{material_lookup, profile_lookup};
use crate::plate::{decode, evaluate_decoded, part_mesh_change};
use beamcraft_engine::footprint::Footprint;
use beamcraft_layer_engine::authored::Change;
use beamcraft_layer_engine::{Control, OpenRequest, Reader};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use session::Refusal;
use std::collections::BTreeMap;
use steel::engine::part::PartRecord;
use steel::layer as steel_layer;
use steel::model::command::{ElementPatch, MemberPatch, UpdateElement};

/// How far from the old elevation an axis end may lie and still count as on the level, in
/// millimetres. Elevations are copied exactly when a member is placed on a level; the slack only
/// absorbs the rounding of a world transform.
const ON_LEVEL_MM: f64 = 0.01;

/// The writes of the members that follow a level.
#[derive(Debug, Default)]
pub(crate) struct Follow {
    /// The authored shape and derived mesh changes.
    pub(crate) changes: Vec<Change>,
    /// Their footprint.
    pub(crate) footprint: Footprint,
    /// The moved members, in identity order.
    pub(crate) moved: Vec<ElementId>,
}

/// `v` turned by the unit quaternion `[x, y, z, w]`.
fn rotate([x, y, z, w]: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let q = [x, y, z];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        let [a0, a1, a2] = a;
        let [b0, b1, b2] = b;
        [a1 * b2 - a2 * b1, a2 * b0 - a0 * b2, a0 * b1 - a1 * b0]
    };
    let t = cross(q, v).map(|c| 2.0 * c);
    let u = cross(q, t);
    let [v0, v1, v2] = v;
    let [t0, t1, t2] = t;
    let [u0, u1, u2] = u;
    [v0 + w * t0 + u0, v1 + w * t1 + u1, v2 + w * t2 + u2]
}

/// The member's axis with the ends at world Z `from_mm` moved to `to_mm`, in its local frame;
/// `None` when no end lies there.
fn moved_axis(record: &PartRecord, from_mm: f64, to_mm: f64) -> Option<Vec<[f64; 3]>> {
    let PartRecord::Member(member) = record else {
        return None;
    };
    let authored = &member.authored;
    if authored.curved || authored.axis_points_mm.len() != 2 {
        return None;
    }
    let placement = &authored.placement;
    let q = placement.orientation_xyzw;
    let [qx, qy, qz, qw] = q;
    let [.., tz] = placement.translation_mm;
    // The world Z step expressed in the element's own frame.
    let step = rotate([-qx, -qy, -qz, qw], [0.0, 0.0, to_mm - from_mm]);
    let mut any = false;
    let axis = authored
        .axis_points_mm
        .iter()
        .map(|point| {
            let [.., z] = rotate(q, *point);
            if (z + tz - from_mm).abs() <= ON_LEVEL_MM {
                any = true;
                let [px, py, pz] = *point;
                let [sx, sy, sz] = step;
                [px + sx, py + sy, pz + sz]
            } else {
                *point
            }
        })
        .collect();
    any.then_some(axis)
}

fn refuse(message: &'static str) -> Refusal {
    Refusal::from(CommandError::new(
        ErrorCode::UnsupportedProperty,
        "move_elements",
        message,
    ))
}

/// The changes that carry every straight beam and column on the level from `from_mm` to
/// `to_mm`, read from the pinned base. Fails closed: an element the engine cannot re-evaluate,
/// or a shared shape whose copies would move differently, refuses the whole edit.
pub(crate) fn follow_level(
    base: &OpenRequest,
    control: &Control,
    from_mm: f64,
    to_mm: f64,
) -> Result<Follow, Refusal> {
    let mut reader = Reader::default();
    let opened = reader.open(base.clone(), control)?;
    let decoded = decode(&reader, &opened, control)?;
    let (plates, members) = evaluate_decoded(&decoded)?;
    let materials = material_lookup(&decoded.definitions);
    let instances = &decoded.instances;
    // The new local axis of every member on the level, grouped by the shape it shares.
    let mut groups: BTreeMap<ElementId, BTreeMap<ElementId, Vec<[f64; 3]>>> = BTreeMap::new();
    for member in &members {
        let id = member.authored.id;
        let Some(record) = PartRecord::find(&plates, &members, id) else {
            continue;
        };
        if let Some(axis) = moved_axis(&record, from_mm, to_mm) {
            groups
                .entry(instances.definition(id)?)
                .or_default()
                .insert(id, axis);
        }
    }
    let mut follow = Follow::default();
    for (definition, moving) in groups {
        let group = instances.group(definition)?;
        let mut axes = moving.values();
        let Some(axis) = axes.next().cloned() else {
            continue;
        };
        if group.len() != moving.len() || axes.any(|other| *other != axis) {
            return Err(refuse(
                "copies sharing a member's shape do not all lie on the level; edit them apart first",
            ));
        }
        let patch = ElementPatch {
            member: MemberPatch {
                axis_points_mm: Some(axis),
                ..MemberPatch::default()
            },
            ..ElementPatch::default()
        };
        UpdateElement {
            target_id: definition,
            patch: patch.clone(),
        }
        .validate_arguments()?;
        let head = PartRecord::find(&plates, &members, definition)
            .ok_or_else(|| refuse("a member on the level is not evaluated"))?;
        let next = head.patched(
            &patch,
            true,
            &materials,
            profile_lookup(&decoded.definitions),
        )?;
        follow.changes.extend(steel_layer::patch_changes(
            &patch,
            &next,
            &definition.to_string(),
        ));
        follow.changes.push(part_mesh_change(&next)?);
        follow.footprint = follow
            .footprint
            .union(Footprint::for_update(
                definition, false, false, true, false, None,
            )?)?
            .union(instances.dependencies(definition)?)?;
        follow.moved.push(definition);
        for id in group {
            if id == definition {
                continue;
            }
            let other = PartRecord::find(&plates, &members, id)
                .ok_or_else(|| refuse("a member on the level is not evaluated"))?;
            let record = next
                .shared_with(&other, &patch)?
                .ok_or_else(|| refuse("a member on the level is not evaluated"))?;
            follow.changes.push(part_mesh_change(&record)?);
            follow.footprint = follow
                .footprint
                .union(Footprint::for_update(id, false, false, true, false, None)?)?;
            follow.moved.push(id);
        }
    }
    follow.moved.sort();
    Ok(follow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quaternion_and_its_conjugate_undo_each_other() {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        // A quarter turn about X takes local Y to world Z.
        let q = [half, 0.0, 0.0, half];
        let [x, y, z] = rotate(q, [0.0, 1.0, 0.0]);
        assert!(x.abs() < 1e-12 && y.abs() < 1e-12 && (z - 1.0).abs() < 1e-12);
        let [x, y, z] = rotate([-half, 0.0, 0.0, half], [0.0, 0.0, 1.0]);
        assert!(x.abs() < 1e-12 && (y - 1.0).abs() < 1e-12 && z.abs() < 1e-12);
    }
}
