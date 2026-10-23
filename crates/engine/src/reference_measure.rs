// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Experimental, read-only measurements over a pinned foreign triangle mesh.
//!
//! Results are mesh enclosures in world millimetres, never F06 engineering quantities. The
//! caller supplies the foreign mesh extracted from the pinned source; this module applies the
//! reference's explicit units interpretation and rigid placement before certifying it.

use crate::reference::ReferenceRecord;
use beamcraft_kernel::measure::{MeshMeasurement, measure};
use beamcraft_kernel::topology::{Analysis, analyze};
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_kernel::verdict::ShellStatus;
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;

/// Algorithm identity for this experimental result format.
pub const REFERENCE_MEASUREMENT_ALGORITHM: &str = "reference-triangle-shell-enclosure/1";

/// One independent result for one mesh in one pinned reference.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceMeshMeasurement {
    /// Wrapper identity whose units and placement were applied.
    pub reference_id: beamcraft_model::id::ElementId,
    /// Exact foreign source bytes measured.
    pub source_digest: ContentDigest,
    /// One-based explicit source version.
    pub source_version: u32,
    /// Identity over the source pin and transformed triangle buffers.
    pub input_signature: ContentDigest,
    /// Geometry certification result. Only `Certified` has values.
    pub shell_status: ShellStatus,
    /// Conservative enclosure of surface area in square millimetres.
    pub area_mm2: Option<beamcraft_kernel::enclosure::Enclosure>,
    /// Conservative enclosure of enclosed volume in cubic millimetres.
    pub volume_mm3: Option<beamcraft_kernel::enclosure::Enclosure>,
    /// Foreign mesh quantities never become current BeamCraft engineering quantities.
    pub engineering_quantity_status: beamcraft_kernel::measure::QuantityStatus,
}

/// Maps and certifies one foreign source mesh. Open, self-intersecting, inward or numerically
/// uncertain meshes return a result with no area or volume; mass is deliberately not inferred.
pub fn measure_reference_mesh(
    reference: &ReferenceRecord,
    source_mesh: &TriMesh,
) -> Result<ReferenceMeshMeasurement, CommandError> {
    let vertices = source_mesh
        .vertices()
        .iter()
        .map(|point| reference.source_point_to_world(*point))
        .collect::<Result<Vec<_>, _>>()?;
    let world_mesh = TriMesh::from_buffers(&vertices, source_mesh.triangles()).map_err(|_| {
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "mesh",
            "transformed reference mesh is outside supported geometry bounds",
        )
    })?;
    let analysis: Analysis = analyze(&world_mesh).map_err(|_| {
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "mesh",
            "reference mesh analysis could not be completed",
        )
    })?;
    let measured: MeshMeasurement = measure(&world_mesh, &analysis).map_err(|_| {
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "mesh",
            "reference mesh measurement could not be completed",
        )
    })?;
    let mut signature =
        Vec::with_capacity(64 + vertices.len() * 24 + source_mesh.triangles().len() * 12);
    signature.extend_from_slice(b"beamcraft-reference-mesh-measurement/1\0");
    signature.extend_from_slice(REFERENCE_MEASUREMENT_ALGORITHM.as_bytes());
    signature.extend_from_slice(&beamcraft_geom::tolerance::TOLERANCE_POLICY_VERSION.to_be_bytes());
    signature.extend_from_slice(reference.source.digest.as_bytes());
    signature.extend_from_slice(reference.id.to_string().as_bytes());
    signature.extend_from_slice(&reference.source.version.to_be_bytes());
    for point in &vertices {
        for coordinate in point {
            signature.extend_from_slice(&coordinate.to_bits().to_be_bytes());
        }
    }
    for triangle in source_mesh.triangles() {
        for index in triangle {
            signature.extend_from_slice(&index.to_be_bytes());
        }
    }
    let input_signature = Sha256::digest(&signature).map_err(|_| {
        CommandError::new(
            ErrorCode::BackendUnavailable,
            "measurement",
            "measurement signature could not be computed",
        )
    })?;
    Ok(ReferenceMeshMeasurement {
        reference_id: reference.id,
        source_digest: reference.source.digest,
        source_version: reference.source.version,
        input_signature,
        shell_status: measured.shell_status(),
        area_mm2: measured.area_mm2(),
        volume_mm3: measured.volume_mm3(),
        engineering_quantity_status: measured.quantity_status(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::command::Placement;
    use beamcraft_model::id::ElementId;
    use beamcraft_model::reference::{SourceFormat, SourcePin, UnitsInterpretation, UpAxis};

    fn reference(meters_per_unit: f64) -> ReferenceRecord {
        ReferenceRecord {
            id: ElementId::parse("01900000-0000-7000-8000-000000000001").unwrap(),
            name: "synthetic cube".to_owned(),
            placement: Placement {
                translation_mm: [0.0; 3],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
            source: SourcePin {
                digest: Sha256::digest(b"synthetic USD source").unwrap(),
                bytes: 20,
                format: SourceFormat::Usda,
                default_prim: "Cube".to_owned(),
                version: 1,
                label: "cube.usda".to_owned(),
                declared_meters_per_unit: Some(meters_per_unit),
                declared_up_axis: Some(UpAxis::Z),
            },
            units: Some(UnitsInterpretation {
                meters_per_unit,
                up_axis: UpAxis::Z,
            }),
        }
    }

    fn cube() -> TriMesh {
        TriMesh::from_buffers(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
            &[
                [0, 2, 1],
                [0, 3, 2],
                [4, 5, 6],
                [4, 6, 7],
                [0, 1, 5],
                [0, 5, 4],
                [1, 2, 6],
                [1, 6, 5],
                [2, 3, 7],
                [2, 7, 6],
                [3, 0, 4],
                [3, 4, 7],
            ],
        )
        .unwrap()
    }

    #[test]
    fn applies_explicit_units_and_reports_mesh_enclosures_not_engineering_quantities() {
        let result = measure_reference_mesh(&reference(1.0), &cube()).unwrap();
        assert_eq!(result.shell_status, ShellStatus::Certified);
        assert!(result.area_mm2.unwrap().contains(6_000_000.0));
        assert!(result.volume_mm3.unwrap().contains(1_000_000_000.0));
        assert_eq!(
            result.engineering_quantity_status,
            beamcraft_kernel::measure::QuantityStatus::Unsupported
        );
    }

    #[test]
    fn open_foreign_mesh_has_no_area_or_volume_value() {
        let open = TriMesh::from_buffers(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
        )
        .unwrap();
        let result = measure_reference_mesh(&reference(0.001), &open).unwrap();
        assert_ne!(result.shell_status, ShellStatus::Certified);
        assert_eq!(result.area_mm2, None);
        assert_eq!(result.volume_mm3, None);
    }
}
