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
//! | `contour_points_mm` (A12.3) | yes | yes | mesh and quantity record (a plate contour) |
//! | `axis_points_mm` | yes | yes | mesh and quantity record (a member path, P12) |
//! | `profile` | yes | yes | mesh and quantity record (a member's pinned section) |
//! | `translation_mm` | no | no | untouched (the body is element-local) |
//! | `orientation_xyzw` | no | no | untouched (the body is element-local) |
//! | `rotate` (T03) | no | no | untouched (a placement-only rigid rotation) |
//! | `mirror` (T03) | yes | yes | mesh and quantity record (the side and the cuts flip) |
//! | `level` | no | no | untouched (a relationship of the element only) |
//!
//! A move keeps the rotation and never touches the shape, the body or the stored quantities, so
//! no geometry is copied or rebuilt. An orientation change replaces the stored quaternion as a
//! whole; local meshes and quantities are unchanged. A `rotate` patch composes a world-axis
//! rotation with the current placement and stays placement-only. A `mirror` patch re-encodes the
//! element through a symmetry of its local body (see `transform`): the world result is the exact
//! mirror image, and the re-encoded fields are a shape change, so the body is rebuilt. A profile
//! change replaces the pinned section of a member (a plate has none). Fields outside this list
//! (contour, roll) are `UNSUPPORTED_PROPERTY`: they belong to later tasks.

use crate::engine::command::AuthoredPlate;
use crate::engine::member::AuthoredMember;
use crate::model::command::ElementPatch;
use beamcraft_model::command::{CommandError, Placement};
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
    /// The element is rotated about a world axis (T03): a placement-only rigid rotation.
    pub rotate: bool,
    /// The element is mirrored about a world plane (T03): the placement and the local shape
    /// records are re-encoded, so the body is rebuilt.
    pub mirror: bool,
    /// The element is translated by a relative world offset (T03): a placement-only move.
    pub move_mm: bool,
    /// The pinned profile of a member changes; the body is rebuilt from the new section.
    pub profile: bool,
}

/// The effect of a patch on the element it targets, ignoring whether a value really differs:
/// an update that sets the current value is still that update.
pub const fn effect(patch: &ElementPatch) -> UpdateEffect {
    UpdateEffect {
        name: patch.name.is_some(),
        class: patch.class.is_some(),
        material: patch.material.is_some(),
        shape: patch.corners.is_some()
            || patch.contour_points_mm.is_some()
            || patch.thickness_mm.is_some()
            || patch.thickness_side.is_some()
            || patch.axis_points_mm.is_some()
            || patch.mirror.is_some()
            || patch.profile.is_some(),
        placement: patch.translation_mm.is_some()
            || patch.rotate.is_some()
            || patch.mirror.is_some()
            || patch.move_mm.is_some(),
        orientation: patch.orientation_xyzw.is_some() || patch.rotate.is_some(),
        level: patch.level.is_some(),
        rotate: patch.rotate.is_some(),
        mirror: patch.mirror.is_some(),
        move_mm: patch.move_mm.is_some(),
        profile: patch.profile.is_some(),
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
    if patch.profile.is_some() {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "profile",
            "a plate has no profile",
        ));
    }
    let mut next = plate.clone();
    if let Some(points) = &patch.contour_points_mm {
        next.contour_points_mm.clone_from(points);
    }
    if let Some(corners) = &patch.corners {
        next.corners.clone_from(corners);
    }
    // One admission covers the pair: a contour patch is validated with the corners it arrives
    // with (each naming a vertex of the new contour), a corner patch with the standing contour.
    if patch.contour_points_mm.is_some() || patch.corners.is_some() {
        next.to_command().validate_arguments()?;
    }
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
    next.placement = if let Some(mirror) = &patch.mirror {
        crate::engine::transform::mirror_plate_local(&mut next)?;
        crate::engine::transform::mirror_placement(
            &plate.placement,
            mirror,
            &crate::engine::transform::PLATE_SYMMETRY,
        )?
    } else {
        placement_after(&plate.placement, patch)?
    };
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
    if patch.contour_points_mm.is_some() {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "contour_points_mm",
            "only plates have a contour",
        ));
    }
    if patch.corners.is_some() {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "corner",
            "only plates have contour corners",
        ));
    }
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
    // The mirror re-encodes through the path symmetry; a path replacement in the same patch
    // would mirror through one path and re-encode through another (the model refuses it too).
    if patch.mirror.is_some() && patch.axis_points_mm.is_some() {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "mirror",
            "a mirror takes no path replacement in the same patch",
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
    if let Some(profile) = patch.profile {
        next.profile = profile;
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
    if let Some(mirror) = &patch.mirror {
        crate::engine::transform::mirror_member_local(&mut next)?;
        // The placement absorbs the world mirror through the same local symmetry the path
        // reflects by, computed from the original (unmirrored) axis.
        let symmetry = crate::engine::transform::member_symmetry(&member.axis_points_mm)
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "mirror",
                    "a polybeam or curved member has no expressible mirrored path",
                )
            })?;
        next.placement =
            crate::engine::transform::mirror_placement(&member.placement, mirror, &symmetry)?;
    } else {
        next.placement = placement_after(&member.placement, patch)?;
    }
    Ok(next)
}

/// The placement after the patch: a `rotate` patch composes a world-axis rotation with the
/// current placement (T03), the translation is replaced when the patch names one, the rotation
/// is replaced when the patch names one, and the result is validated (`LIMIT_EXCEEDED` outside
/// the coordinate range, `NON_FINITE_NUMBER` or `INVALID_ARGUMENT` for a hostile quaternion).
fn placement_after(placement: &Placement, patch: &ElementPatch) -> Result<Placement, CommandError> {
    if let Some(rotate) = &patch.rotate {
        return crate::engine::transform::rotate_placement(placement, rotate);
    }
    if let Some(move_mm) = patch.move_mm {
        let next = Placement {
            translation_mm: [
                placement.translation_mm[0] + move_mm[0],
                placement.translation_mm[1] + move_mm[1],
                placement.translation_mm[2] + move_mm[2],
            ],
            orientation_xyzw: placement.orientation_xyzw,
        };
        next.validate()?;
        return Ok(next);
    }
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
    use crate::model::command::MemberKind;
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
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
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
                level: false,
                rotate: false,
                mirror: false,
                move_mm: false,
                profile: false
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
            curved: false,
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000006").unwrap(),
            kind: MemberKind::Beam,
            name: "Beam".into(),
            class: 0,
            axis_points_mm: vec![[0.0; 3], [1000.0, 0.0, 0.0]],
            profile: Sha256::digest(b"profile").unwrap(),
            roll_rad: None,
            section_placement: None,
            placement: Placement::IDENTITY,
            material: None,
            cuts: Vec::new(),
            hole_records: Vec::new(),
        }
    }

    #[test]
    fn a_thickness_side_patch_is_a_shape_change_and_a_member_refuses_it() {
        use crate::model::command::PlateSide;
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
            curved: false,
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000006").unwrap(),
            kind: MemberKind::Beam,
            name: "Beam".into(),
            class: 0,
            axis_points_mm: vec![[0.0; 3], [1000.0, 0.0, 0.0]],
            profile,
            roll_rad: None,
            section_placement: None,
            placement: Placement::IDENTITY,
            material: None,
            cuts: Vec::new(),
            hole_records: Vec::new(),
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
    fn a_rotate_patch_turns_the_placement_and_keeps_the_shape() {
        use crate::model::command::ElementRotate;
        let quarter = ElementRotate {
            pivot_mm: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            angle_rad: core::f64::consts::FRAC_PI_2,
        };
        let patch = ElementPatch {
            rotate: Some(quarter),
            ..ElementPatch::default()
        };
        let effects = effect(&patch);
        assert!(effects.rotate && effects.placement && effects.orientation);
        assert!(!effects.shape && !effects.mirror);
        let turned = patch_member(&member(), &patch).unwrap();
        // The placement translation sits on the pivot and stays; the world pose carries the
        // quarter turn; the shape stays element-local and untouched.
        assert_eq!(turned.placement.translation_mm, [0.0; 3]);
        let [x, y, z, w] = turned.placement.orientation_xyzw;
        assert!(z.abs() - core::f64::consts::FRAC_1_SQRT_2 < 1e-9);
        assert!(w.abs() - core::f64::consts::FRAC_1_SQRT_2 < 1e-9);
        assert!(x.abs() < 1e-9 && y.abs() < 1e-9);
        assert_eq!(turned.axis_points_mm, member().axis_points_mm);
        // Absolute placement fields are refused beside a rotate (the model validates this, the
        // engine honours the rotate alone).
        let model = crate::model::command::UpdateElement {
            target_id: turned.id,
            patch: ElementPatch {
                rotate: Some(quarter),
                translation_mm: Some([1.0; 3]),
                ..ElementPatch::default()
            },
        };
        assert_eq!(
            model.validate_arguments().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_mirror_patch_reencodes_a_plate_and_rebuilds_the_body() {
        use crate::model::command::ElementMirror;
        use crate::model::command::PlateSide;
        let patch = ElementPatch {
            mirror: Some(ElementMirror {
                point_mm: [0.0; 3],
                normal: [1.0, 0.0, 0.0],
            }),
            ..ElementPatch::default()
        };
        let effects = effect(&patch);
        assert!(effects.mirror && effects.shape && effects.placement);
        assert!(!effects.orientation);
        let mirrored = patch_plate(&plate(), &patch).unwrap();
        // The default (top) side flips to the explicit bottom; the contour stays.
        assert_eq!(mirrored.thickness_side, Some(PlateSide::Bottom));
        assert_eq!(mirrored.contour_points_mm, plate().contour_points_mm);
        // The mirrored translation of the identity placement stays the identity here.
        assert_eq!(mirrored.placement.translation_mm, [0.0; 3]);
        // A member mirror is refused for a polybeam path; a path replacement beside a mirror
        // is refused outright (one symmetry per patch).
        let bent = ElementPatch {
            mirror: patch.mirror,
            ..ElementPatch::default()
        };
        let mut poly = member();
        poly.axis_points_mm = vec![[0.0; 3], [1000.0, 0.0, 0.0], [0.0, 500.0, 0.0]];
        assert_eq!(
            patch_member(&poly, &bent).unwrap_err().code,
            ErrorCode::UnsupportedGeometry
        );
        let with_path = ElementPatch {
            mirror: patch.mirror,
            axis_points_mm: Some(vec![[0.0; 3], [2000.0, 0.0, 0.0]]),
            ..ElementPatch::default()
        };
        assert_eq!(
            patch_member(&member(), &with_path).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
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
    #[test]
    fn a_contour_patch_replaces_the_whole_contour_and_validates_the_pair() {
        use crate::model::command::{CornerTreatment, PlateCorner};
        let before = plate();
        let moved = ElementPatch {
            contour_points_mm: Some(vec![
                [0.0, 0.0, 0.0],
                [2000.0, 0.0, 0.0],
                [2000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ]),
            ..Default::default()
        };
        assert!(effect(&moved).shape);
        let after = patch_plate(&before, &moved).unwrap();
        assert_eq!(after.contour_points_mm[1], [2000.0, 0.0, 0.0]);
        assert_eq!(after.corners, before.corners);
        assert_eq!(after.thickness_mm, 10.0);
        // Corners arriving with the contour are validated against the new contour: the corner
        // names a vertex of it, and a stale index is refused before anything changes.
        let pair = patch_plate(
            &before,
            &ElementPatch {
                contour_points_mm: Some(vec![
                    [0.0, 0.0, 0.0],
                    [500.0, 0.0, 0.0],
                    [0.0, 500.0, 0.0],
                ]),
                corners: Some(vec![PlateCorner {
                    vertex: 0,
                    treatment: CornerTreatment::Round { radius_mm: 10. },
                }]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(pair.contour_points_mm.len(), 3);
        assert_eq!(pair.corners.len(), 1);
        assert!(
            patch_plate(
                &before,
                &ElementPatch {
                    contour_points_mm: Some(vec![
                        [0.0, 0.0, 0.0],
                        [500.0, 0.0, 0.0],
                        [0.0, 500.0, 0.0]
                    ]),
                    corners: Some(vec![PlateCorner {
                        vertex: 3,
                        treatment: CornerTreatment::Round { radius_mm: 10. },
                    }]),
                    ..Default::default()
                }
            )
            .is_err()
        );
        // Too few points and a member target are refused.
        assert_eq!(
            patch_plate(
                &before,
                &ElementPatch {
                    contour_points_mm: Some(vec![[0.0, 0.0, 0.0], [500.0, 0.0, 0.0]]),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            patch_member(
                &member(),
                &ElementPatch {
                    contour_points_mm: Some(before.contour_points_mm.clone()),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
            ErrorCode::UnsupportedProperty
        );
    }

    #[test]
    fn corner_replacement_is_a_shape_patch_and_preserves_the_sharp_contour() {
        use crate::model::command::{CornerTreatment, PlateCorner};
        let before = plate();
        let corners = vec![PlateCorner {
            vertex: 0,
            treatment: CornerTreatment::Round { radius_mm: 10. },
        }];
        let patch = ElementPatch {
            corners: Some(corners.clone()),
            ..Default::default()
        };
        assert!(effect(&patch).shape);
        let after = patch_plate(&before, &patch).unwrap();
        assert_eq!(after.corners, corners);
        assert_eq!(after.contour_points_mm, before.contour_points_mm);
        assert!(before.corners.is_empty());
        assert!(patch_member(&member(), &patch).is_err());
        let cleared = patch_plate(
            &after,
            &ElementPatch {
                corners: Some(vec![]),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(cleared.corners.is_empty());
        assert!(
            patch_plate(
                &before,
                &ElementPatch {
                    corners: Some(vec![PlateCorner {
                        vertex: 999,
                        treatment: CornerTreatment::Round { radius_mm: 1. }
                    }]),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
