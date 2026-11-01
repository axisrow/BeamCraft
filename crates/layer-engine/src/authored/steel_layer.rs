// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The steel authoring facade (#388): free functions for every steel shape, shape reader and
//! steel change the apps build, so no caller names a `Shape::` or `Change::` steel constructor
//! directly. This module moves to `crates/steel` unchanged when the dependency flips; call
//! sites then change their path only (`steel::plate(...)` instead of
//! `authored::steel_layer::plate(...)`), which is why every constructor here keeps the argument
//! shape of the inherent method it delegates to.

use super::*;
use steel::model::instance::PartKind;

//
// Shapes.
//

/// An extruded plate (P01, A02, A04, A10, A12); see [`PlateShape`].
pub fn plate(plate: PlateShape) -> Shape {
    Shape::plate(plate)
}

/// A lofted plate between two face contours (A11); see [`LoftedPlateShape`].
pub fn lofted_plate(plate: LoftedPlateShape) -> Shape {
    Shape::lofted_plate(plate)
}

/// A beam or a column with a required pinned profile (P03, P12, P13, P14); see
/// [`MemberShape`].
pub fn member(member: MemberShape) -> Shape {
    Shape::member(member)
}

/// An instance of a canonical part (P11); see [`InstanceShape`].
pub fn instance(kind: PartKind, definition: String) -> Shape {
    Shape::instance(kind, definition)
}

/// An A06 fillet weld between two parts; see [`Weld`].
pub fn weld(weld: Weld) -> Shape {
    Shape::weld(weld)
}

/// An A05 bolt group along a world line; see [`Bolt`].
pub fn bolt(bolt: Bolt) -> Shape {
    Shape::bolt(bolt)
}

/// An A14.3 bolt group over a pinned fastener definition; see [`BoltGroup`].
pub fn bolt_group(group: BoltGroup) -> Shape {
    Shape::bolt_group(group)
}

//
// Shape readers.
//

/// The extruded plate of a shape, when it is one.
pub fn as_plate(shape: &Shape) -> Option<&PlateShape> {
    shape.as_plate()
}

/// The lofted plate of a shape, when it is one.
pub fn as_lofted_plate(shape: &Shape) -> Option<&LoftedPlateShape> {
    shape.as_lofted_plate()
}

/// The member of a shape, when it is one.
pub fn as_member(shape: &Shape) -> Option<&MemberShape> {
    shape.as_member()
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
    shape.as_weld()
}

/// The A05 bolt group of a shape, when it is one.
pub fn as_bolt(shape: &Shape) -> Option<&Bolt> {
    shape.as_bolt()
}

/// The A14.3 bolt group of a shape, when it is one.
pub fn as_bolt_group(shape: &Shape) -> Option<&BoltGroup> {
    shape.as_bolt_group()
}

//
// Changes.
//

/// Replace the complete plate contour; see [`SetContour`].
pub fn contour(id: String, points_mm: Vec<[f64; 3]>) -> Change {
    Change::contour(id, points_mm)
}

/// Replace a required plate thickness; see [`SetThickness`].
pub fn thickness(id: String, mm: f64) -> Change {
    Change::thickness(id, mm)
}

/// Replace the plate's thickness side (A12.1); see [`SetThicknessSide`].
pub fn thickness_side(id: String, side: PlateSide) -> Change {
    Change::thickness_side(id, side)
}

/// Replace the plate's corner treatments as a whole (A12.2); see [`SetCorners`].
pub fn corners(id: String, corners: Vec<PlateCorner>) -> Change {
    Change::corners(id, corners)
}

/// Replace the plate's bend records as a whole (A10); see [`SetBends`].
pub fn bends(id: String, bends: Vec<BendRecord>) -> Change {
    Change::bends(id, bends)
}

/// Replace the owned cut records of a plate, beam or column (A03); see [`SetCuts`].
pub fn cuts(id: String, cuts: Vec<CutRecord>) -> Change {
    Change::cuts(id, cuts)
}

/// Replace the plate's hole records (A04); see [`SetHoleRecords`].
pub fn hole_records(id: String, records: Vec<HoleRecord>) -> Change {
    Change::hole_records(id, records)
}

/// Replace the member's hole records (A04.2); see [`SetMemberHoles`].
pub fn member_holes(id: String, records: Vec<MemberHoleRecord>) -> Change {
    Change::member_holes(id, records)
}

/// Replace the member's path points as a whole (P03 two, P12 three or more); see [`SetAxis`].
pub fn axis(id: String, points_mm: Vec<[f64; 3]>) -> Change {
    Change::axis(id, points_mm)
}

/// Change the optional member roll opinion; see [`SetRoll`].
pub fn roll(id: String, opinion: Opinion<f64>) -> Change {
    Change::roll(id, opinion)
}

/// Replace the required immutable member profile; see [`SetProfile`].
pub fn profile(id: String, digest: ContentDigest) -> Change {
    Change::profile(id, digest)
}

/// Replace the complete authored record of an A14.3 bolt group; see [`ReplaceBoltGroup`].
pub fn replace_bolt_group(id: String, group: BoltGroup) -> Change {
    Change::bolt_group(id, group)
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
