// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The view snapshot's fastener meshes of committed bolt groups (issue 512).
//!
//! A committed group places one A14.2 fastener prototype at every bearing point with one rigid
//! transform: the bearing face on the bearing point — floated the
//! [`BEARING_FACE_LIFT_MM`] lift off the plate face so coplanar front faces do not fight in
//! the picture (issue 593) — and the stack along the group's shank axis
//! towards the nut (bolt amendment section 4). This module merges those instances into the one
//! closed scene mesh a snapshot element carries. Every dimension is the pinned definition's or
//! the placement decision's — nothing is invented — and the group's own record is never
//! rewritten: the display reads, like every view.
//!
//! A group is one scene element of kind [`ElementKind::Connection`] with the identity pose: a
//! row of fasteners along a turned frame has no single element-local pose, so the mesh carries
//! its world coordinates directly, in the manner of the reference display. The prototype's own
//! frame maps to the group's, so every fastener of the group shares one orientation.
//!
//! A prototype the kernel refuses, a merged mesh beyond the kernel's caps, or a blob that
//! cannot be hashed refuses the element and so the whole snapshot — like any committed body
//! that no longer evaluates. The volume carried alongside the mesh is the summed component
//! volume of the placed prototypes, not a certified body: the view keeps no quantities.

use crate::engine::bolt::BoltGroupPlaced;
use crate::kernel::fastener::{self, FastenerFailure};
use crate::model::fastener::FastenerSpec;
use beamcraft_engine::element::ElementKind;
use beamcraft_engine::scene::SceneElement;
use beamcraft_engine::solid::SolidMesh;
use beamcraft_geom::Translation3;
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;

/// The display lift that floats the fastener's bearing face — the head washer's joint face,
/// the head's own face where the definition carries no washer — a hair outside the joint
/// (issue 593). The bearing face lies exactly on the bearing plane by construction, the same
/// plane the bearing plate's face occupies with the same outward normal, so in the ring
/// between the hole and the washer rim two front-facing faces competed for one depth value
/// and tore the picture. The lift separates the planes for the depth buffer and stays far
/// below what a view can resolve.
const BEARING_FACE_LIFT_MM: f64 = 0.05;

/// The scene element of one committed bolt group with its solid: the fasteners merged into one
/// mesh in world millimetres at the identity pose. The solid is the same mesh as the element's
/// representation with its volume, kept beside it so a proxy consumer (an IFC export reads the
/// group's bolts, issue 558) needs no blob decode to read the triangles.
///
/// `far_faces` gives, per fastener in bearing order, the joint's far face along the shank axis
/// (millimetres from the bearing plane), when the clamped parts could be measured. The nut and
/// its washer then seat on that face and the length the standard size adds past the joint
/// shows as protrusion beyond the nut; a fastener without a measured face, or one whose face
/// lies past the tip-hung nut, keeps the prototype's stack (issue 562 item 5: a rounded-up
/// length floated the nut washer off the far plate on bare shank).
pub fn scene_element(
    id: ElementId,
    placed: &BoltGroupPlaced,
    spec: &FastenerSpec,
    far_faces: &[Option<f64>],
) -> Result<(SceneElement, SolidMesh), CommandError> {
    let prototype = fastener::build(
        spec,
        placed.length_mm,
        placed.washer_under_head,
        placed.washer_under_nut,
    )
    .map_err(|failure| match failure {
        FastenerFailure::Input(error) => error,
        FastenerFailure::Profile(_) => CommandError::new(
            failure.f04_code(),
            "fastener",
            "the committed group's fastener prototype is refused",
        ),
    })?;
    let mut points = Vec::new();
    let mut triangles = Vec::new();
    let mut volume_mm3 = 0.0_f64;
    // The nut stack's lowest face in the prototype: the nut washer's bearing face, or the
    // nut's own when no washer sits under it.
    let nut_stack = |kind: fastener::ComponentKind| {
        matches!(
            kind,
            fastener::ComponentKind::Nut | fastener::ComponentKind::WasherNut
        )
    };
    let stack_base = prototype
        .components
        .iter()
        .filter(|c| nut_stack(c.component))
        .map(|c| c.z_offset_mm)
        .fold(f64::INFINITY, f64::min);
    for (index, bearing) in placed.bearings_mm.iter().enumerate() {
        // Seat the stack on the far face, never past the tip (a face beyond the hung stack
        // would pull the nut off the thread) and never for a face that is not finite.
        let seat = far_faces
            .get(index)
            .copied()
            .flatten()
            .filter(|face| face.is_finite() && stack_base.is_finite())
            .map(|face| face - stack_base)
            .filter(|delta| *delta < 0.0)
            .unwrap_or(0.0);
        for component in &prototype.components {
            let mesh = component.solid.mesh();
            let lift = if nut_stack(component.component) {
                seat
            } else {
                0.0
            };
            let bearing_lift = match component.component {
                fastener::ComponentKind::WasherHead => Some(-BEARING_FACE_LIFT_MM),
                // The head owns the bearing face only where no washer sits under it; with a
                // washer its underside meets the washer's top face across opposing normals,
                // which the back-face cull keeps clean.
                fastener::ComponentKind::Head if !placed.washer_under_head => {
                    Some(-BEARING_FACE_LIFT_MM)
                }
                _ => None,
            };
            let base = u32::try_from(points.len()).map_err(|_| capped())?;
            for vertex in mesh.vertices() {
                // The prototype frame maps to the group frame: the bearing face on the bearing
                // point, lifted a hair outside the joint (issue 593), the stack along `z`
                // towards the nut (bolt amendment section 4).
                let local = [
                    vertex.x,
                    vertex.y,
                    vertex.z + component.z_offset_mm + lift + bearing_lift.unwrap_or(0.0),
                ];
                points.push([
                    bearing[0]
                        + placed.x[0] * local[0]
                        + placed.y[0] * local[1]
                        + placed.z[0] * local[2],
                    bearing[1]
                        + placed.x[1] * local[0]
                        + placed.y[1] * local[1]
                        + placed.z[1] * local[2],
                    bearing[2]
                        + placed.x[2] * local[0]
                        + placed.y[2] * local[1]
                        + placed.z[2] * local[2],
                ]);
            }
            for triangle in mesh.triangles() {
                triangles.push([triangle[0] + base, triangle[1] + base, triangle[2] + base]);
            }
            volume_mm3 += component.solid.volume_mm3();
        }
    }
    let mesh = TriMesh::from_buffers(&points, &triangles).map_err(|e| {
        CommandError::new(
            e.code.f04_code(),
            "parts",
            "the group's fasteners exceed the mesh caps",
        )
    })?;
    let solid = SolidMesh::composed(&mesh, volume_mm3).ok_or_else(|| {
        CommandError::new(
            ErrorCode::LimitExceeded,
            "parts",
            "the group's merged mesh could not be hashed",
        )
    })?;
    let identity = Translation3::new(0.0, 0.0, 0.0)
        .map_err(|_| CommandError::new(ErrorCode::NonFiniteNumber, "placement", "unreachable"))?;
    let element = SceneElement::new(id, ElementKind::Connection, identity, solid.scene_mesh());
    Ok((element, solid))
}

fn capped() -> CommandError {
    CommandError::new(
        ErrorCode::LimitExceeded,
        "parts",
        "the group's fasteners exceed the mesh caps",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bolt::StoredPlacement;
    use crate::model::fastener::{HeadKind, HoleMode, WasherSpec};

    /// The washered M16 bolt of the kernel fastener suite.
    fn bolt() -> FastenerSpec {
        FastenerSpec {
            designation: Some("M16".into()),
            standard: None,
            grade: None,
            display_name: None,
            kind: crate::model::fastener::FastenerKind::Bolt,
            head_kind: HeadKind::Hex,
            nominal_diameter_mm: 16.0,
            head_across_flats_mm: 24.0,
            head_height_mm: 10.0,
            nut_across_flats_mm: Some(24.0),
            nut_height_mm: Some(14.8),
            washer: Some(WasherSpec {
                outer_diameter_mm: 30.0,
                inner_diameter_mm: 17.0,
                thickness_mm: 3.0,
            }),
            hole_clearance_mm: 2.0,
            lengths_mm: vec![30.0, 35.0, 45.0],
            thread_protrusion_mm: 5.0,
            washer_under_head: true,
            washer_under_nut: true,
            hole_mode: HoleMode::Generate,
            mass_each_kg: None,
        }
    }

    /// Three fasteners along `x` at `x = 50, 130, 220`, the bearing face on `z = 0`.
    fn placed() -> BoltGroupPlaced {
        BoltGroupPlaced::from_stored(&StoredPlacement {
            point1_mm: [0.0; 3],
            point2_mm: [400.0, 0.0, 0.0],
            plane_normal_mm: [0.0, 0.0, 1.0],
            rotation_rad: 0.0,
            on_plane_mm: 0.0,
            depth_mm: 0.0,
            offset_start_mm: 50.0,
            spacings_mm: vec![80.0, 90.0],
            length_mm: 35.0,
            washer_under_head: true,
            washer_under_nut: true,
        })
        .unwrap()
    }

    fn element() -> SceneElement {
        scene_element(
            ElementId::parse("019a7bf0-0000-7000-8000-000000710001").unwrap(),
            &placed(),
            &bolt(),
            &[],
        )
        .unwrap()
        .0
    }

    #[test]
    fn the_group_is_one_connection_element_with_the_world_mesh_at_the_identity_pose() {
        let element = element();
        assert_eq!(element.kind, ElementKind::Connection);
        assert_eq!(element.orientation_xyzw, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(element.placement.offset().components(), [0.0; 3]);
        // The washered M16 stack from the bearing face: washer −3..0 and head −13..−3 outside
        // the joint, shank −13..22 through it with the tip at 22, and the nut stack hanging
        // downward from the tip: nut 2.2..17, washer nut −0.8..2.2. The washer's 30 mm outer
        // diameter is the widest part, so the bounds span the bearings by half of it.
        let placed = placed();
        assert_eq!(
            placed.bearings_mm,
            vec![[50.0, 0.0, 0.0], [130.0, 0.0, 0.0], [220.0, 0.0, 0.0]]
        );
        let vertices = element.full.vertices.as_ref();
        let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
        for v in vertices {
            for (got, axis) in [(v.x, 0), (v.y, 1), (v.z, 2)] {
                min[axis] = min[axis].min(got);
                max[axis] = max[axis].max(got);
            }
        }
        for (got, want) in min.iter().zip([35.0, -15.0, -13.0]) {
            assert!((got - want).abs() <= 1e-9, "min {got} != {want}");
        }
        for (got, want) in max.iter().zip([235.0, 15.0, 22.0]) {
            assert!((got - want).abs() <= 1e-9, "max {got} != {want}");
        }
    }

    #[test]
    fn the_merged_mesh_is_deterministic_and_instanced_per_bearing() {
        let first = element();
        let second = element();
        assert_eq!(first.full.digest, second.full.digest);
        // One prototype per bearing: three times the five-component M16 stack's vertices.
        let one = fastener::build(&bolt(), 35.0, true, true).unwrap();
        let prototype_vertices: usize = one
            .components
            .iter()
            .map(|c| c.solid.mesh().vertices().len())
            .sum();
        let element = element();
        assert_eq!(
            element.full.vertices.len(),
            prototype_vertices * placed().bearings_mm.len()
        );
        // The volume is the summed component volume of every placed prototype.
        let prototype_volume: f64 = one
            .components
            .iter()
            .map(|c| c.solid.volume_mm3())
            .sum::<f64>()
            * placed().bearings_mm.len() as f64;
        let solid = SolidMesh::composed(
            &TriMesh::from_blob(&element.full.blob).unwrap(),
            prototype_volume,
        )
        .unwrap();
        assert_eq!(solid.digest(), element.full.digest);
    }

    #[test]
    fn a_group_the_kernel_cannot_place_or_build_is_refused() {
        let stored = StoredPlacement {
            point1_mm: [0.0; 3],
            point2_mm: [400.0, 0.0, 0.0],
            plane_normal_mm: [0.0, 0.0, 1.0],
            rotation_rad: 0.0,
            on_plane_mm: 0.0,
            depth_mm: 0.0,
            offset_start_mm: 50.0,
            spacings_mm: vec![80.0],
            length_mm: 35.0,
            washer_under_head: true,
            washer_under_nut: true,
        };
        // A committed row that gives no frame is refused, never drawn.
        let degenerate = BoltGroupPlaced::from_stored(&StoredPlacement {
            point2_mm: [0.0; 3],
            ..stored.clone()
        })
        .unwrap_err();
        assert_eq!(degenerate.field, "point2_mm");
        // A stored length the definition cannot build refuses the element.
        let broken = BoltGroupPlaced::from_stored(&StoredPlacement {
            length_mm: 0.0005,
            ..stored
        })
        .unwrap();
        let error = scene_element(
            ElementId::parse("019a7bf0-0000-7000-8000-000000710001").unwrap(),
            &broken,
            &bolt(),
            &[],
        )
        .unwrap_err();
        assert_eq!(
            (error.code, error.field),
            (ErrorCode::InvalidArgument, "length_mm")
        );
    }

    #[test]
    fn a_measured_far_face_seats_the_nut_stack_on_the_joint() {
        // Issue 562 item 5: the 35 mm length leaves the hung nut washer at −0.8, short of a
        // far face at −0.8 nothing changes; a joint whose far face lies at −2.0 (a longer
        // standard size than the grip needs) pulls the washer down onto it, and the shank tip
        // protrudes past the nut instead. A face past the hung stack, or an unknown one,
        // keeps the prototype's stack.
        let id = ElementId::parse("019a7bf0-0000-7000-8000-000000710001").unwrap();
        let low = |faces: &[Option<f64>]| -> Vec<f64> {
            let (element, _) = scene_element(id, &placed(), &bolt(), faces).unwrap();
            // The lowest point of the nut washer of each fastener: vertices above the bearing
            // plane within the washer's radius of each bearing.
            [50.0, 130.0, 220.0]
                .iter()
                .map(|x| {
                    element
                        .full
                        .vertices
                        .iter()
                        .filter(|v| (v.x - x).abs() <= 15.0 + 1e-9 && v.z > -0.85)
                        .map(|v| v.z)
                        .fold(f64::INFINITY, f64::min)
                })
                .collect()
        };
        let hung = low(&[]);
        for z in &hung {
            assert!((z + 0.8).abs() < 1e-9, "{hung:?}");
        }
        let seated = {
            let (element, _) =
                scene_element(id, &placed(), &bolt(), &[Some(-2.0), None, Some(4.0)]).unwrap();
            let washer_low = |x: f64| {
                element
                    .full
                    .vertices
                    .iter()
                    .filter(|v| (v.x - x).abs() <= 15.0 + 1e-9 && v.z > -3.0 + 1e-6)
                    .map(|v| v.z)
                    .fold(f64::INFINITY, f64::min)
            };
            [washer_low(50.0), washer_low(130.0), washer_low(220.0)]
        };
        // The first seats at −2.0; the second has no face; the third's face lies past the
        // hung stack, which stays where the tip puts it.
        assert!((seated[0] + 2.0).abs() < 1e-9, "{seated:?}");
        assert!((seated[1] + 0.8).abs() < 1e-9, "{seated:?}");
        assert!((seated[2] + 0.8).abs() < 1e-9, "{seated:?}");
    }

    #[test]
    fn the_bearing_face_floats_a_hair_outside_the_joint_so_coplanar_faces_do_not_fight() {
        // Issue 593: the washer's joint face sat exactly on the bearing plane — the plane the
        // bearing plate's face occupies with the same outward normal — so the two front-facing
        // faces of the ring between hole and washer rim tore each other apart in the depth
        // buffer. The lift now holds the fastener's bearing face outside the plane, with a
        // washer and with the head's own face where the definition carries none.
        let placed = |washer_under_head: bool| -> BoltGroupPlaced {
            BoltGroupPlaced::from_stored(&StoredPlacement {
                point1_mm: [0.0; 3],
                point2_mm: [400.0, 0.0, 0.0],
                plane_normal_mm: [0.0, 0.0, 1.0],
                rotation_rad: 0.0,
                on_plane_mm: 0.0,
                depth_mm: 0.0,
                offset_start_mm: 50.0,
                spacings_mm: vec![80.0, 90.0],
                length_mm: 35.0,
                washer_under_head,
                washer_under_nut: true,
            })
            .unwrap()
        };
        let id = ElementId::parse("019a7bf0-0000-7000-8000-000000710001").unwrap();
        // The washered stack: the vertices of the washer's outer rim (the Ø30 ring) sit on
        // its two circular faces, and the joint face is the upper one — at the lift, never on
        // the bearing plane. Below zero keeps the nut-side washer, the same Ø30 ring, out.
        let (washer, _) = scene_element(id, &placed(true), &bolt(), &[]).unwrap();
        let rim = [50.0, 130.0, 220.0]
            .iter()
            .map(|x| {
                washer
                    .full
                    .vertices
                    .iter()
                    .filter(|v| v.z < 0.0 && ((v.x - x).hypot(v.y) - 15.0).abs() < 1e-6)
                    .map(|v| v.z)
                    .fold(f64::NEG_INFINITY, f64::max)
            })
            .collect::<Vec<_>>();
        for z in &rim {
            assert!((z + BEARING_FACE_LIFT_MM).abs() < 1e-9, "{rim:?}");
        }
        // Without a washer the head's own face owns the bearing plane: its hexagon corners
        // (across flats 24) below zero touch the plane at the lift, and the head floats a
        // hair higher than the shank start that stays at the bare stack.
        let (bare, _) = scene_element(id, &placed(false), &bolt(), &[]).unwrap();
        let corner = [50.0, 130.0, 220.0]
            .iter()
            .map(|x| {
                bare.full
                    .vertices
                    .iter()
                    .filter(|v| v.z < 0.0 && (v.x - x).hypot(v.y) >= 13.5)
                    .map(|v| v.z)
                    .fold(f64::INFINITY, f64::min)
            })
            .collect::<Vec<_>>();
        for z in &corner {
            assert!(
                (z - (-10.0 - BEARING_FACE_LIFT_MM)).abs() < 1e-9,
                "{corner:?}"
            );
        }
    }

    #[test]
    fn the_solid_beside_the_element_is_the_same_mesh_with_its_volume() {
        let (element, solid) = scene_element(
            ElementId::parse("019a7bf0-0000-7000-8000-000000710001").unwrap(),
            &placed(),
            &bolt(),
            &[],
        )
        .unwrap();
        // The solid is the element's own representation: same vertices, same triangles, same
        // blob digest, so a proxy consumer reads exactly what the view draws.
        assert_eq!(solid.vertices().len(), element.full.vertices.len());
        assert_eq!(solid.digest().to_string(), element.full.digest.to_string());
        let prototype = fastener::build(&bolt(), 35.0, true, true).unwrap();
        let volume: f64 = prototype
            .components
            .iter()
            .map(|c| c.solid.volume_mm3())
            .sum::<f64>()
            * placed().bearings_mm.len() as f64;
        assert!((volume - solid.volume_mm3()).abs() < 1e-6);
    }
}
