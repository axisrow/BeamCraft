// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `UpdateElement` for the supported fields (F04 section 7, P04, P08, D2): a typed patch of the
//! label, the material reference, a plate's thickness, the placement translation (a move), the
//! placement rotation and the level assignment.
//!
//! The engine applies the patch to the authored state, re-evaluates what depends on it and says
//! which groups changed, so the adapter writes only those:
//!
//! | Field | Rebuilds the body | Rebuilds the mass | Derived layer |
//! |-------|-------------------|-------------------|---------------|
//! | `name` | no | no | untouched |
//! | `material` | no | yes | quantity record only (same mesh bytes) |
//! | `thickness_mm` | yes | yes | mesh and quantity record |
//! | `translation_mm` | no | no | untouched (the body is element-local) |
//! | `orientation_xyzw` | a member: yes (the rotation is in the mesh), a plate: no | no | the placement attribute; for a member also the mesh |
//! | `level` | no | no | untouched (a relationship of the element only) |
//!
//! A move keeps the rotation and never touches the shape, the body or the stored quantities, so
//! no geometry is copied or rebuilt. An orientation change replaces the stored quaternion as a
//! whole (a member body is rebuilt with the new rotation, the quantities are unchanged). Fields
//! outside this list (contour, axis, roll, profile) are `UNSUPPORTED_PROPERTY`: they belong to
//! later tasks.

use crate::command::AuthoredPlate;
use crate::member::AuthoredMember;
use beamcraft_model::command::{CommandError, ElementPatch, Placement};
use beamcraft_model::error::ErrorCode;

/// Which groups of an element a patch changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateEffect {
    /// The label changes.
    pub name: bool,
    /// The material reference changes (including its removal).
    pub material: bool,
    /// A shape input changes, so the body is rebuilt.
    pub shape: bool,
    /// The placement translation changes (a move); the body is untouched.
    pub placement: bool,
    /// The placement rotation changes; the translation is kept.
    pub orientation: bool,
    /// The level assignment changes (D2), including its removal; no geometry is touched.
    pub level: bool,
}

/// The effect of a patch on the element it targets, ignoring whether a value really differs:
/// an update that sets the current value is still that update.
pub const fn effect(patch: &ElementPatch) -> UpdateEffect {
    UpdateEffect {
        name: patch.name.is_some(),
        material: patch.material.is_some(),
        shape: patch.thickness_mm.is_some(),
        placement: patch.translation_mm.is_some(),
        orientation: patch.orientation_xyzw.is_some(),
        level: patch.level.is_some(),
    }
}

/// Applies a patch to an authored plate.
pub fn patch_plate(
    plate: &AuthoredPlate,
    patch: &ElementPatch,
) -> Result<AuthoredPlate, CommandError> {
    let mut next = plate.clone();
    if let Some(name) = &patch.name {
        next.name.clone_from(name);
    }
    if let Some(material) = patch.material {
        next.material = material;
    }
    if let Some(thickness) = patch.thickness_mm {
        next.thickness_mm = thickness;
    }
    // A plate keeps the create-time refusal for a non-identity rotation: committed rotated
    // plates are not evaluated, so accepting the patch here would refuse every later read of
    // the store. The identity stays the reset.
    if let Some(xyzw) = patch.orientation_xyzw
        && xyzw != Placement::IDENTITY.orientation_xyzw
    {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "placement.orientation_xyzw",
            "rotated plate placement is not evaluated in this slice",
        ));
    }
    next.placement = placement_after(&plate.placement, patch)?;
    Ok(next)
}

/// Applies a patch to an authored member. A member has no thickness: that field is
/// `UNSUPPORTED_PROPERTY`.
pub fn patch_member(
    member: &AuthoredMember,
    patch: &ElementPatch,
) -> Result<AuthoredMember, CommandError> {
    if patch.thickness_mm.is_some() {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "thickness_mm",
            "a member has no thickness",
        ));
    }
    let mut next = member.clone();
    if let Some(name) = &patch.name {
        next.name.clone_from(name);
    }
    if let Some(material) = patch.material {
        next.material = material;
    }
    next.placement = placement_after(&member.placement, patch)?;
    Ok(next)
}

/// The placement after the patch: the translation is replaced when the patch names one, the
/// rotation is replaced when the patch names one, and the result is validated
/// (`LIMIT_EXCEEDED` outside the coordinate range, `NON_FINITE_NUMBER` or `INVALID_ARGUMENT`
/// for a hostile quaternion).
fn placement_after(placement: &Placement, patch: &ElementPatch) -> Result<Placement, CommandError> {
    if patch.translation_mm.is_none() && patch.orientation_xyzw.is_none() {
        return Ok(*placement);
    }
    let next = Placement {
        translation_mm: patch.translation_mm.unwrap_or(placement.translation_mm),
        orientation_xyzw: patch.orientation_xyzw.unwrap_or(placement.orientation_xyzw),
    };
    next.validate()?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::command::MemberKind;
    use beamcraft_model::digest::Sha256;
    use beamcraft_model::id::ElementId;

    fn plate() -> AuthoredPlate {
        AuthoredPlate {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
            name: "Plate".into(),
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            placement: Placement::IDENTITY,
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    #[test]
    fn the_effect_names_the_groups_a_patch_touches() {
        let rename = ElementPatch {
            name: Some("B".into()),
            ..ElementPatch::default()
        };
        assert_eq!(
            effect(&rename),
            UpdateEffect {
                name: true,
                material: false,
                shape: false,
                placement: false,
                orientation: false,
                level: false
            }
        );
        let clear = ElementPatch {
            material: Some(None),
            ..ElementPatch::default()
        };
        assert!(effect(&clear).material && !effect(&clear).shape);
        let to_level = ElementPatch {
            level: Some(None),
            ..ElementPatch::default()
        };
        assert!(
            effect(&to_level).level
                && !effect(&to_level).shape
                && !effect(&to_level).material
                && !effect(&to_level).placement
        );
        let shift = ElementPatch {
            translation_mm: Some([1.0, 2.0, 3.0]),
            ..ElementPatch::default()
        };
        assert!(effect(&shift).placement && !effect(&shift).shape && !effect(&shift).material);
    }

    #[test]
    fn a_move_replaces_the_translation_keeps_the_rotation_and_the_shape() {
        let mut turned = plate();
        turned.placement.orientation_xyzw = [0.0, 0.0, 1.0, 0.0];
        let next = patch_plate(
            &turned,
            &ElementPatch {
                translation_mm: Some([5.0, 6.0, 7.0]),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(next.placement.translation_mm, [5.0, 6.0, 7.0]);
        assert_eq!(next.placement.orientation_xyzw, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(next.contour_points_mm, turned.contour_points_mm);
        assert_eq!(next.thickness_mm, turned.thickness_mm);
        let far = patch_plate(
            &plate(),
            &ElementPatch {
                translation_mm: Some([2e9, 0.0, 0.0]),
                ..ElementPatch::default()
            },
        )
        .unwrap_err();
        assert_eq!(far.code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn a_patch_changes_only_its_fields_and_distinguishes_clearing_the_material() {
        let steel = Sha256::digest(b"steel").unwrap();
        let set = patch_plate(
            &plate(),
            &ElementPatch {
                material: Some(Some(steel)),
                thickness_mm: Some(12.0),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(set.material, Some(steel));
        assert_eq!(set.thickness_mm, 12.0);
        assert_eq!(set.name, "Plate");
        let cleared = patch_plate(
            &set,
            &ElementPatch {
                material: Some(None),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(cleared.material, None);
        assert_eq!(cleared.thickness_mm, 12.0);
        // Absence leaves the material alone.
        let same = patch_plate(
            &set,
            &ElementPatch {
                name: Some("N".into()),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(same.material, Some(steel));
    }

    #[test]
    fn an_orientation_patch_replaces_the_rotation_and_keeps_everything_else() {
        let quarter = [0.0, 0.0, 0.5_f64.sqrt(), 0.5_f64.sqrt()];
        let turning = |xyzw: [f64; 4]| ElementPatch {
            orientation_xyzw: Some(xyzw),
            ..ElementPatch::default()
        };
        let turned = patch_member(&member(), &turning(quarter)).unwrap();
        assert_eq!(turned.placement.orientation_xyzw, quarter);
        assert_eq!(turned.placement.translation_mm, [0.0; 3]);
        assert_eq!(turned.axis_points_mm, member().axis_points_mm);
        assert_eq!(turned.roll_rad, None);
        let effect = effect(&turning(quarter));
        assert!(effect.orientation && !effect.placement && !effect.shape);
        // A move after the turn keeps the new rotation; an orientation patch keeps the move.
        let moved = patch_member(
            &turned,
            &ElementPatch {
                translation_mm: Some([5.0, 6.0, 7.0]),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(moved.placement.orientation_xyzw, quarter);
        assert_eq!(moved.placement.translation_mm, [5.0, 6.0, 7.0]);
        // A plate keeps the create-time refusal for a non-identity orientation patch (a rotated
        // plate would refuse every later read of the store), and the identity still resets it.
        let refused = patch_plate(
            &plate(),
            &ElementPatch {
                orientation_xyzw: Some(quarter),
                translation_mm: Some([1.0, 0.0, 0.0]),
                ..ElementPatch::default()
            },
        )
        .unwrap_err();
        assert_eq!(refused.code, ErrorCode::UnsupportedGeometry);
        assert_eq!(refused.field, "placement.orientation_xyzw");
        let reset = patch_plate(
            &plate(),
            &ElementPatch {
                orientation_xyzw: Some(Placement::IDENTITY.orientation_xyzw),
                translation_mm: Some([1.0, 0.0, 0.0]),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(
            reset.placement.orientation_xyzw,
            Placement::IDENTITY.orientation_xyzw
        );
        assert_eq!(reset.placement.translation_mm, [1.0, 0.0, 0.0]);
        assert_eq!(reset.contour_points_mm, plate().contour_points_mm);
    }

    #[test]
    fn a_hostile_orientation_patch_is_refused_by_code() {
        let patch = |xyzw: [f64; 4]| ElementPatch {
            orientation_xyzw: Some(xyzw),
            ..ElementPatch::default()
        };
        // Non-normalized (squared norm 0.5) and the zero quaternion.
        assert_eq!(
            patch_member(&member(), &patch([0.0, 0.0, 0.5, 0.5]))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            patch_member(&member(), &patch([0.0; 4])).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        // NaN and an out-of-range component.
        assert_eq!(
            patch_member(&member(), &patch([f64::NAN, 0.0, 0.0, 1.0]))
                .unwrap_err()
                .code,
            ErrorCode::NonFiniteNumber
        );
        // A plate refuses any non-identity orientation patch as unsupported geometry before
        // validation details, mirroring create_plate; the model layer owns the code-level
        // validation refusals.
        assert_eq!(
            patch_plate(&plate(), &patch([2.0, 0.0, 0.0, 0.0]))
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
    }

    fn member() -> AuthoredMember {
        AuthoredMember {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000006").unwrap(),
            kind: MemberKind::Beam,
            name: "Beam".into(),
            axis_points_mm: [[0.0; 3], [1000.0, 0.0, 0.0]],
            profile: Sha256::digest(b"profile").unwrap(),
            roll_rad: None,
            placement: Placement::IDENTITY,
            material: None,
            cuts: Vec::new(),
        }
    }

    #[test]
    fn a_member_has_no_thickness() {
        let profile = Sha256::digest(b"profile").unwrap();
        let member = AuthoredMember {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000006").unwrap(),
            kind: MemberKind::Beam,
            name: "Beam".into(),
            axis_points_mm: [[0.0; 3], [1000.0, 0.0, 0.0]],
            profile,
            roll_rad: None,
            placement: Placement::IDENTITY,
            material: None,
            cuts: Vec::new(),
        };
        assert_eq!(
            patch_member(
                &member,
                &ElementPatch {
                    thickness_mm: Some(3.0),
                    ..ElementPatch::default()
                }
            )
            .unwrap_err()
            .code,
            ErrorCode::UnsupportedProperty
        );
        let renamed = patch_member(
            &member,
            &ElementPatch {
                name: Some("Beam 2".into()),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(renamed.name, "Beam 2");
        assert_eq!(renamed.profile, profile);
    }
}
