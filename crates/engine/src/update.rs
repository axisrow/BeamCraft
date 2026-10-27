// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `UpdateElement` for the supported fields (F04 section 7, P04, P08, D2, P12): a typed patch of
//! the label, the material reference, a plate's thickness, a member's path points, the placement
//! translation (a move), the placement rotation and the level assignment.
//!
//! The engine applies the patch to the authored state, re-evaluates what depends on it and says
//! which groups changed, so the adapter writes only those:
//!
//! | Field | Rebuilds the body | Rebuilds the mass | Derived layer |
//! |-------|-------------------|-------------------|---------------|
//! | `name` | no | no | untouched |
//! | `material` | no | yes | quantity record only (same mesh bytes) |
//! | `thickness_mm` | yes | yes | mesh and quantity record |
//! | `thickness_side` (A12.1) | yes | yes | mesh and quantity record |
//! | `axis_points_mm` | yes | yes | mesh and quantity record (a member path, P12) |
//! | `translation_mm` | no | no | untouched (the body is element-local) |
//! | `orientation_xyzw` | no | no | untouched (the body is element-local) |
//! | `level` | no | no | untouched (a relationship of the element only) |
//!
//! A move keeps the rotation and never touches the shape, the body or the stored quantities, so
//! no geometry is copied or rebuilt. An orientation change replaces the stored quaternion as a
//! whole; local meshes and quantities are unchanged. Fields
//! outside this list (contour, roll, profile) are `UNSUPPORTED_PROPERTY`: they belong to later
//! tasks.

use crate::command::AuthoredPlate;
use crate::member::AuthoredMember;
use beamcraft_model::command::{CommandError, ElementPatch, Placement};
use beamcraft_model::error::ErrorCode;

/// Which groups of an element a patch changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateEffect {
    /// The label changes.
    pub name: bool,
    /// The authored part class changes without changing engineering data.
    pub class: bool,
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
        class: patch.class.is_some(),
        material: patch.material.is_some(),
        shape: patch.thickness_mm.is_some()
            || patch.thickness_side.is_some()
            || patch.axis_points_mm.is_some(),
        placement: patch.translation_mm.is_some(),
        orientation: patch.orientation_xyzw.is_some(),
        level: patch.level.is_some(),
    }
}

/// Applies a patch to an authored plate. A plate has no path points: that field is
/// `UNSUPPORTED_PROPERTY`.
pub fn patch_plate(
    plate: &AuthoredPlate,
    patch: &ElementPatch,
) -> Result<AuthoredPlate, CommandError> {
    if patch.axis_points_mm.is_some() {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "axis_points_mm",
            "a plate has no path points",
        ));
    }
    let mut next = plate.clone();
    if let Some(name) = &patch.name {
        next.name.clone_from(name);
    }
    if let Some(class) = patch.class {
        next.class = class;
    }
    if let Some(material) = patch.material {
        next.material = material;
    }
    if let Some(thickness) = patch.thickness_mm {
        next.thickness_mm = thickness;
    }
    if let Some(side) = patch.thickness_side {
        next.thickness_side = Some(side);
    }
    next.placement = placement_after(&plate.placement, patch)?;
    Ok(next)
}

/// Applies a patch to an authored member. A member has no thickness: that field and the
/// thickness side are `UNSUPPORTED_PROPERTY`. A path patch replaces the whole polyline; fewer
/// than two points is `INVALID_ARGUMENT`, and whether the chords are long enough is the body
/// evaluation's decision.
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
    if patch.thickness_side.is_some() {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "thickness_side",
            "a member has no thickness side",
        ));
    }
    let mut next = member.clone();
    if let Some(name) = &patch.name {
        next.name.clone_from(name);
    }
    if let Some(class) = patch.class {
        next.class = class;
    }
    if let Some(material) = patch.material {
        next.material = material;
    }
    if let Some(points) = &patch.axis_points_mm {
        if points.len() < 2 {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "axis_points_mm",
                "a member path has at least two points",
            ));
        }
        next.axis_points_mm.clone_from(points);
        // A polybeam carries no authored roll (the G12 sweep has no roll input): bending a
        // rolled straight member into a polybeam clears the opinion instead of storing one
        // the body would silently ignore.
        if points.len() > 2 {
            next.roll_rad = None;
        }
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
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
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
                class: false,
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
        let turned_plate = patch_plate(
            &plate(),
            &ElementPatch {
                orientation_xyzw: Some(quarter),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(turned_plate.placement.orientation_xyzw, quarter);
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
        // Plates and members use the same typed rigid-pose validation.
        assert_eq!(
            patch_plate(&plate(), &patch([2.0, 0.0, 0.0, 0.0]))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    fn member() -> AuthoredMember {
        AuthoredMember {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000006").unwrap(),
            kind: MemberKind::Beam,
            name: "Beam".into(),
            class: 0,
            axis_points_mm: vec![[0.0; 3], [1000.0, 0.0, 0.0]],
            profile: Sha256::digest(b"profile").unwrap(),
            roll_rad: None,
            placement: Placement::IDENTITY,
            material: None,
            cuts: Vec::new(),
        }
    }

    #[test]
    fn a_thickness_side_patch_is_a_shape_change_and_a_member_refuses_it() {
        use beamcraft_model::command::PlateSide;
        let side = ElementPatch {
            thickness_side: Some(PlateSide::Bottom),
            ..ElementPatch::default()
        };
        assert!(effect(&side).shape && !effect(&side).placement);
        let next = patch_plate(&plate(), &side).unwrap();
        assert_eq!(next.thickness_side, Some(PlateSide::Bottom));
        assert_eq!(next.thickness_mm, 10.0);
        assert_eq!(
            patch_member(&member(), &side).unwrap_err().code,
            ErrorCode::UnsupportedProperty
        );
    }

    #[test]
    fn a_member_has_no_thickness() {
        let profile = Sha256::digest(b"profile").unwrap();
        let member = AuthoredMember {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000006").unwrap(),
            kind: MemberKind::Beam,
            name: "Beam".into(),
            class: 0,
            axis_points_mm: vec![[0.0; 3], [1000.0, 0.0, 0.0]],
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

    #[test]
    fn a_path_patch_replaces_the_whole_member_path_and_is_a_shape_change() {
        // The effect names the shape group, so the adapter re-evaluates the body.
        let bent = ElementPatch {
            axis_points_mm: Some(vec![[0.0; 3], [3000.0, 0.0, 0.0], [3000.0, 4000.0, 0.0]]),
            ..ElementPatch::default()
        };
        assert!(effect(&bent).shape);
        assert!(!effect(&bent).placement);
        let patched = patch_member(&member(), &bent).unwrap();
        assert_eq!(
            patched.axis_points_mm,
            vec![[0.0; 3], [3000.0, 0.0, 0.0], [3000.0, 4000.0, 0.0]]
        );
        // Fewer than two points is an argument refusal; two points straighten the member.
        assert_eq!(
            patch_member(
                &member(),
                &ElementPatch {
                    axis_points_mm: Some(vec![[0.0; 3]]),
                    ..ElementPatch::default()
                }
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        let straightened = patch_member(
            &patched,
            &ElementPatch {
                axis_points_mm: Some(vec![[0.0; 3], [7000.0, 0.0, 0.0]]),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(straightened.axis_points_mm.len(), 2);
        // The untouched patch keeps the path and everything else.
        let untouched = patch_member(&member(), &ElementPatch::default()).unwrap();
        assert_eq!(untouched, member());
    }

    #[test]
    fn bending_a_rolled_straight_member_clears_the_roll_opinion() {
        let mut rolled = member();
        rolled.roll_rad = Some(0.5);
        let bent = patch_member(
            &rolled,
            &ElementPatch {
                axis_points_mm: Some(vec![[0.0; 3], [3000.0, 0.0, 0.0], [3000.0, 4000.0, 0.0]]),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        // A stored polybeam reads rollRad absent: the sweep has no roll input.
        assert_eq!(bent.roll_rad, None);
        // Straightening keeps the patch's own two points; a polybeam patch on a member
        // without roll stays without one.
        let straight = patch_member(
            &bent,
            &ElementPatch {
                axis_points_mm: Some(vec![[0.0; 3], [7000.0, 0.0, 0.0]]),
                ..ElementPatch::default()
            },
        )
        .unwrap();
        assert_eq!(straight.roll_rad, None);
    }

    #[test]
    fn a_plate_has_no_path_points() {
        assert_eq!(
            patch_plate(
                &plate(),
                &ElementPatch {
                    axis_points_mm: Some(vec![[0.0; 3], [1000.0, 0.0, 0.0]]),
                    ..ElementPatch::default()
                }
            )
            .unwrap_err()
            .code,
            ErrorCode::UnsupportedProperty
        );
    }
}
