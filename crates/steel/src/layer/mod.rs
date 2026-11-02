// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The steel half of the authored layer (#388): the discipline to install into the platform
//! registry, the free functions for every steel shape, shape reader and steel change the apps
//! build, and the part-instance links of a stage, so no caller names a `Shape::` or `Change::`
//! steel constructor directly. The functions keep the argument shape of the platform methods
//! they replaced, so a call site changes its path only (`steel::layer::plate(...)` instead of
//! `authored::steel_layer::plate(...)`).
//!
//! Call [`install`] once at start-up to register the steel discipline with
//! `beamcraft_layer_engine::authored::disciplines`; without it every steel part kind of a
//! loaded layer stays inert unknown vocabulary and no steel element can be authored.

mod connection;
pub mod definition;
pub mod discipline;
mod instance_shape;
mod member;
mod plate;
mod plate_change;
mod read;
mod write;

pub use connection::*;
pub use definition::{fastener, rectangle, section};
pub use discipline::install;
pub use instance_shape::*;
pub use member::*;
pub use plate::*;
pub use plate_change::*;

use crate::model::command::{
    BendRecord, CutRecord, HoleRecord, MemberHoleRecord, PlateCorner, PlateSide,
};
use crate::model::instance::PartKind;
use beamcraft_layer_engine::authored::{
    Change, HookedChange, HookedShape, Opinion, Shape, invalid,
};
use beamcraft_layer_engine::fail;
use beamcraft_model::digest::ContentDigest;

//
// Shapes.
//

/// An extruded plate (P01, A02, A04, A10, A12); see [`PlateShape`].
pub fn plate(plate: PlateShape) -> Shape {
    Shape::Discipline(HookedShape::new(plate))
}

/// A lofted plate between two face contours (A11); see [`LoftedPlateShape`].
pub fn lofted_plate(plate: LoftedPlateShape) -> Shape {
    Shape::Discipline(HookedShape::new(plate))
}

/// A beam or a column with a required pinned profile (P03, P12, P13, P14); see
/// [`MemberShape`].
pub fn member(member: MemberShape) -> Shape {
    Shape::Discipline(HookedShape::new(member))
}

/// The shape that creates `part` as it stands: its local inputs, without the label, placement
/// and material the platform writes. Shared by the single copy (P05) and the array copies (T03).
///
/// # Errors
///
/// `DEPENDENCY_UNAVAILABLE` for a drilled member: A04.2 member-hole records have no copy
/// authoring yet (fresh operation ids and a member-holes write), so a copy would report the
/// source's holed mesh and lose the holes on the next evaluation.
pub fn part_shape(
    part: &crate::engine::part::PartRecord,
) -> Result<Shape, beamcraft_model::command::CommandError> {
    use crate::engine::part::PartRecord;
    Ok(match part {
        PartRecord::Plate(r) => {
            let a = &r.authored;
            plate(PlateShape {
                contour_mm: a.contour_points_mm.clone(),
                thickness_mm: a.thickness_mm,
                thickness_side: a.thickness_side,
                corners: a.corners.clone(),
                bends: a.bends.clone(),
                holes: a
                    .holes_mm
                    .iter()
                    .map(|h| h.iter().map(|[x, y]| [*x, *y, 0.0]).collect())
                    .collect(),
                hole_records: a.hole_records.clone(),
                cuts: a.cuts.clone(),
            })
        }
        PartRecord::Member(r) => {
            if !r.authored.hole_records.is_empty() {
                return Err(beamcraft_model::command::CommandError::new(
                    beamcraft_model::error::ErrorCode::DependencyUnavailable,
                    "hole_records",
                    "copying a drilled member is not implemented",
                ));
            }
            let a = &r.authored;
            member(MemberShape {
                kind: match a.kind {
                    crate::model::command::MemberKind::Beam => MemberKind::Beam,
                    crate::model::command::MemberKind::Column => MemberKind::Column,
                },
                curved: a.curved,
                axis_mm: a.axis_points_mm.clone(),
                profile: a.profile,
                roll_rad: a.roll_rad,
                placement: a.section_placement,
                cuts: a.cuts.clone(),
            })
        }
    })
}

/// An instance of a canonical part (P11); see [`InstanceShape`].
pub fn instance(kind: PartKind, definition: String) -> Shape {
    Shape::Discipline(HookedShape::new(InstanceShape { kind, definition }))
}

/// An A06 fillet weld between two parts; see [`Weld`].
pub fn weld(weld: Weld) -> Shape {
    Shape::Discipline(HookedShape::new(weld))
}

/// An A05 bolt group along a world line; see [`Bolt`].
pub fn bolt(bolt: Bolt) -> Shape {
    Shape::Discipline(HookedShape::new(bolt))
}

/// An A14.3 bolt group over a pinned fastener definition; see [`BoltGroup`].
pub fn bolt_group(group: BoltGroup) -> Shape {
    Shape::Discipline(HookedShape::new(group))
}

//
// Shape readers.
//

/// The extruded plate of a shape, when it is one.
pub fn as_plate(shape: &Shape) -> Option<&PlateShape> {
    match shape {
        Shape::Discipline(hook) => hook.0.as_any().downcast_ref::<PlateShape>(),
        _ => None,
    }
}

/// The lofted plate of a shape, when it is one.
pub fn as_lofted_plate(shape: &Shape) -> Option<&LoftedPlateShape> {
    match shape {
        Shape::Discipline(hook) => hook.0.as_any().downcast_ref::<LoftedPlateShape>(),
        _ => None,
    }
}

/// The member of a shape, when it is one.
pub fn as_member(shape: &Shape) -> Option<&MemberShape> {
    match shape {
        Shape::Discipline(hook) => hook.0.as_any().downcast_ref::<MemberShape>(),
        _ => None,
    }
}

/// The P11 instance of a shape, when it is one.
pub fn as_instance(shape: &Shape) -> Option<&InstanceShape> {
    match shape {
        Shape::Discipline(hook) => hook.0.as_any().downcast_ref::<InstanceShape>(),
        _ => None,
    }
}

/// The A06 weld of a shape, when it is one.
pub fn as_weld(shape: &Shape) -> Option<&Weld> {
    match shape {
        Shape::Discipline(hook) => hook.0.as_any().downcast_ref::<Weld>(),
        _ => None,
    }
}

/// The A05 bolt group of a shape, when it is one.
pub fn as_bolt(shape: &Shape) -> Option<&Bolt> {
    match shape {
        Shape::Discipline(hook) => hook.0.as_any().downcast_ref::<Bolt>(),
        _ => None,
    }
}

/// The A14.3 bolt group of a shape, when it is one.
pub fn as_bolt_group(shape: &Shape) -> Option<&BoltGroup> {
    match shape {
        Shape::Discipline(hook) => hook.0.as_any().downcast_ref::<BoltGroup>(),
        _ => None,
    }
}

//
// Changes.
//

/// Replace the complete plate contour; see [`SetContour`].
pub fn contour(id: String, points_mm: Vec<[f64; 3]>) -> Change {
    Change::Discipline(HookedChange::new(SetContour { id, points_mm }))
}

/// Replace a required plate thickness; see [`SetThickness`].
pub fn thickness(id: String, mm: f64) -> Change {
    Change::Discipline(HookedChange::new(SetThickness { id, mm }))
}

/// Replace the plate's thickness side (A12.1); see [`SetThicknessSide`].
pub fn thickness_side(id: String, side: PlateSide) -> Change {
    Change::Discipline(HookedChange::new(SetThicknessSide { id, side }))
}

/// Replace the plate's corner treatments as a whole (A12.2); see [`SetCorners`].
pub fn corners(id: String, corners: Vec<PlateCorner>) -> Change {
    Change::Discipline(HookedChange::new(SetCorners { id, corners }))
}

/// Replace the plate's bend records as a whole (A10); see [`SetBends`].
pub fn bends(id: String, bends: Vec<BendRecord>) -> Change {
    Change::Discipline(HookedChange::new(SetBends { id, bends }))
}

/// Replace the owned cut records of a plate, beam or column (A03); see [`SetCuts`].
pub fn cuts(id: String, cuts: Vec<CutRecord>) -> Change {
    Change::Discipline(HookedChange::new(SetCuts { id, cuts }))
}

/// Replace the plate's hole records (A04); see [`SetHoleRecords`].
pub fn hole_records(id: String, records: Vec<HoleRecord>) -> Change {
    Change::Discipline(HookedChange::new(SetHoleRecords { id, records }))
}

/// Replace the member's hole records (A04.2); see [`SetMemberHoles`].
pub fn member_holes(id: String, records: Vec<MemberHoleRecord>) -> Change {
    Change::Discipline(HookedChange::new(SetMemberHoles { id, records }))
}

/// Replace the member's path points as a whole (P03 two, P12 three or more); see [`SetAxis`].
pub fn axis(id: String, points_mm: Vec<[f64; 3]>) -> Change {
    Change::Discipline(HookedChange::new(SetAxis { id, points_mm }))
}

/// Change the optional member roll opinion; see [`SetRoll`].
pub fn roll(id: String, opinion: Opinion<f64>) -> Change {
    Change::Discipline(HookedChange::new(SetRoll { id, opinion }))
}

/// Replace the required immutable member profile; see [`SetProfile`].
pub fn profile(id: String, digest: ContentDigest) -> Change {
    Change::Discipline(HookedChange::new(SetProfile { id, digest }))
}

/// Replace the complete authored record of an A14.3 bolt group; see [`ReplaceBoltGroup`].
pub fn replace_bolt_group(id: String, group: BoltGroup) -> Change {
    Change::Discipline(HookedChange::new(ReplaceBoltGroup { id, group }))
}

/// Persistent part-instance links of a validated stage (P11): each instance and the canonical
/// part its `bim:instanceOf` record names. Definitions are canonical parts, never chains, and
/// the stage holds at most [`crate::model::instance::MAX_PART_INSTANCES`] instances.
pub fn links(
    prims: &[beamcraft_layer_engine::Prim],
) -> Result<Vec<crate::model::instance::InstanceLink>, beamcraft_layer_engine::Diagnostic> {
    use beamcraft_layer_engine::authored::instance::definition;
    use beamcraft_model::error::ErrorCode;
    use beamcraft_model::id::ElementId;

    let mut out = Vec::new();
    for p in prims {
        if let Some(source) = definition(p, prims)? {
            if out.len() >= crate::model::instance::MAX_PART_INSTANCES {
                return Err(fail(ErrorCode::LimitExceeded, "too many part instances"));
            }
            let id = |p: &beamcraft_layer_engine::Prim| {
                ElementId::parse(p.element_id.as_deref().ok_or_else(invalid)?)
                    .map_err(|_| invalid())
            };
            out.push(crate::model::instance::InstanceLink {
                instance: id(p)?,
                definition: id(source)?,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate_shape() -> PlateShape {
        PlateShape {
            contour_mm: vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 100.0, 0.0]],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            bends: Vec::new(),
            holes: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    /// Whether a change is the discipline change `T`.
    fn hooked<T: 'static>(change: &Change) -> bool {
        match change {
            Change::Discipline(hook) => hook.0.as_any().downcast_ref::<T>().is_some(),
            _ => false,
        }
    }

    #[test]
    fn the_facade_builds_the_shapes_it_reads_back() {
        let plate = plate(plate_shape());
        assert_eq!(as_plate(&plate), Some(&plate_shape()));
        assert!(as_lofted_plate(&plate).is_none());
        assert!(as_member(&plate).is_none());
        let instance = instance(
            PartKind::Plate,
            "0199e7a0-0000-7000-8000-000000000001".into(),
        );
        assert!(as_instance(&instance).is_some());
        assert!(as_instance(&plate).is_none());
    }

    #[test]
    fn the_facade_builds_the_discipline_changes() {
        assert!(hooked::<SetContour>(&contour(
            "0199e7a0-0000-7000-8000-000000000001".into(),
            Vec::new(),
        )));
        assert!(hooked::<SetThickness>(&thickness(
            "0199e7a0-0000-7000-8000-000000000002".into(),
            10.0,
        )));
    }
}
