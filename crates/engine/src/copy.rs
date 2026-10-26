// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `CopyElement` for plates and members (F04 section 7, P05).
//!
//! A copy has a fresh identity and the source's authored inputs, with an optional label and a
//! translation added to the placement. The shape inputs, the pinned material and the pinned
//! profile are shared by value: definitions are immutable, so the copy follows the same digests.
//! Because the body is element-local, the copy's body and mesh are the source's, byte for byte;
//! only a later edit of the copy that changes its shape produces different geometry.
//!
//! Relationship authoring belongs to the adapter and layer engine: a duplicate keeps its
//! level and immutable definitions, but joins no assembly and takes no connections (DV-REL-011).

use crate::command::AuthoredPlate;
use crate::member::AuthoredMember;
use beamcraft_model::command::{CommandError, CopyElement, Placement};

fn copied_placement(source: &Placement, command: &CopyElement) -> Result<Placement, CommandError> {
    let [ox, oy, oz] = command.offset_mm;
    let [sx, sy, sz] = source.translation_mm;
    let moved = Placement {
        translation_mm: [sx + ox, sy + oy, sz + oz],
        orientation_xyzw: source.orientation_xyzw,
    };
    moved.validate()?;
    Ok(moved)
}

/// The authored plate of the copy. Fails with `LIMIT_EXCEEDED` when the moved placement leaves
/// the coordinate range.
pub fn copy_plate(
    source: &AuthoredPlate,
    command: &CopyElement,
) -> Result<AuthoredPlate, CommandError> {
    command.validate_arguments()?;
    let mut next = source.clone();
    next.id = command.new_id;
    if let Some(name) = &command.name {
        next.name.clone_from(name);
    }
    next.placement = copied_placement(&source.placement, command)?;
    Ok(next)
}

/// The authored member of the copy; see [`copy_plate`].
pub fn copy_member(
    source: &AuthoredMember,
    command: &CopyElement,
) -> Result<AuthoredMember, CommandError> {
    command.validate_arguments()?;
    let mut next = source.clone();
    next.id = command.new_id;
    if let Some(name) = &command.name {
        next.name.clone_from(name);
    }
    next.placement = copied_placement(&source.placement, command)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::error::ErrorCode;
    use beamcraft_model::id::ElementId;

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

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
            placement: Placement {
                translation_mm: [10.0, 20.0, 30.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
            material: None,
            holes_mm: Vec::new(),
            cut: None,
        }
    }

    fn command() -> CopyElement {
        CopyElement {
            source_id: id(5),
            new_id: id(6),
            name: Some("Copy".into()),
            offset_mm: [100.0, 0.0, -5.0],
        }
    }

    #[test]
    fn a_copy_keeps_the_shape_and_moves_the_placement() {
        let copy = copy_plate(&plate(), &command()).unwrap();
        assert_eq!(copy.id, id(6));
        assert_eq!(copy.name, "Copy");
        assert_eq!(copy.placement.translation_mm, [110.0, 20.0, 25.0]);
        assert_eq!(copy.contour_points_mm, plate().contour_points_mm);
        assert_eq!(copy.thickness_mm, 10.0);
    }

    #[test]
    fn the_label_defaults_to_the_source() {
        let mut c = command();
        c.name = None;
        assert_eq!(copy_plate(&plate(), &c).unwrap().name, "Plate");
    }

    #[test]
    fn a_placement_out_of_range_is_refused() {
        let mut source = plate();
        source.placement.translation_mm[0] = 999_999_999.0;
        let mut c = command();
        c.offset_mm = [10.0, 0.0, 0.0];
        assert_eq!(
            copy_plate(&source, &c).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn an_equal_identity_is_refused() {
        let mut c = command();
        c.new_id = c.source_id;
        assert_eq!(
            copy_plate(&plate(), &c).unwrap_err().code,
            ErrorCode::DuplicateId
        );
    }
}
