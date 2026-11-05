// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The view snapshot's fastener meshes of committed bolt groups (issue 512).
//!
//! A committed group places one A14.2 fastener prototype at every bearing point with one rigid
//! transform: the bearing face on the bearing point and the stack along the group's shank axis
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

/// The scene element of one committed bolt group: its fasteners merged into one mesh in world
/// millimetres at the identity pose.
pub fn scene_element(
    id: ElementId,
    placed: &BoltGroupPlaced,
    spec: &FastenerSpec,
) -> Result<SceneElement, CommandError> {
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
    for bearing in &placed.bearings_mm {
        for component in &prototype.components {
            let mesh = component.solid.mesh();
            let base = u32::try_from(points.len()).map_err(|_| capped())?;
            for vertex in mesh.vertices() {
                // The prototype frame maps to the group frame: the bearing face at the bearing
                // point, the stack along `z` towards the nut (bolt amendment section 4).
                let local = [vertex.x, vertex.y, vertex.z + component.z_offset_mm];
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
    Ok(SceneElement::new(
        id,
        ElementKind::Connection,
        identity,
        solid.scene_mesh(),
    ))
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
        )
        .unwrap()
    }

    #[test]
    fn the_group_is_one_connection_element_with_the_world_mesh_at_the_identity_pose() {
        let element = element();
        assert_eq!(element.kind, ElementKind::Connection);
        assert_eq!(element.orientation_xyzw, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(element.placement.offset().components(), [0.0; 3]);
        // The washered M16 stack from the bearing face: washer 0..3, head 3..13, shank 3..38,
        // nut 38..52.8, washer nut 52.8..55.8. The washer's 30 mm outer diameter is the widest
        // part, so the bounds span the bearings by half of it.
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
        for (got, want) in min.iter().zip([35.0, -15.0, 0.0]) {
            assert!((got - want).abs() <= 1e-9, "min {got} != {want}");
        }
        for (got, want) in max.iter().zip([235.0, 15.0, 55.8]) {
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
        )
        .unwrap_err();
        assert_eq!(
            (error.code, error.field),
            (ErrorCode::InvalidArgument, "length_mm")
        );
    }
}
