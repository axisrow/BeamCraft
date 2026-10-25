// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! R02: measurements of a foreign reference's composed surface as separate result objects.
//!
//! The measured thing is never the foreign model itself and never a quantity of ours: the
//! kernel certifies the polygon surface the wrapper composes (the display profile: translated
//! `Cube` and triangle/convex-quad `Mesh` cages), mapped through the wrapper's explicit units
//! correction and its full rigid placement, and the result is a standalone reading of that
//! surface. Nothing is stored and no foreign prim is written; the caller decides where a
//! result lives (persisting measurements is W03, not this).
//!
//! The status policy is the kernel's, whole: volume, area and centroid exist only for a
//! certified closed shell, as outward-rounded enclosures of the exact sums — an open,
//! non-manifold, inward, degenerate or self-intersecting surface is never measured exactly.
//! Mass is never invented: a foreign surface has no density input, and F06 section 8 gives a
//! mesh from another source no engineering quantity, so the mass record is `unsupported` with
//! no value by construction. The measurement is a pure function of the pinned bytes, the units
//! correction and the placement: the same head always answers with the same numbers.

use crate::reference::ReferenceRecord;
use crate::scene::{ReferenceGeometry, foreign_surface};
use beamcraft_kernel::diagnostic::Stage;
use beamcraft_kernel::enclosure::Enclosure;
use beamcraft_kernel::measure::{self, MassRecord};
use beamcraft_kernel::spatial::Aabb;
use beamcraft_kernel::topology;
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_kernel::verdict::{MeshError, ShellStatus};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;

/// The kernel reading of one wrapper's composed surface: present only when the wrapper's
/// sources offer a supported surface at all.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshReading {
    /// The shell verdict the reading is gated by; values below exist only for a certified shell.
    pub status: ShellStatus,
    /// Digest of the world-space triangle mesh the numbers come from: the input signature that
    /// moves when the source, the correction or the placement moves the geometry.
    pub mesh_digest: ContentDigest,
    /// Exact min/max of the world-space vertices, always available.
    pub bounds_mm: Aabb,
    /// Enclosure of the enclosed volume in mm3; `None` unless certified.
    pub volume_mm3: Option<Enclosure>,
    /// Enclosure of the surface area in mm2; `None` unless certified.
    pub area_mm2: Option<Enclosure>,
    /// Enclosure of the volume centroid in mm; `None` unless certified.
    pub centroid_mm: Option<[Enclosure; 3]>,
    /// Mass of the surface: `unsupported` with no value, always. A foreign surface carries no
    /// density and no engineering quantity (F06 section 8); this field exists so the result
    /// states that instead of leaving mass silently absent.
    pub mass: MassRecord,
}

/// The measurement of one reference wrapper: which wrapper, what it pins, and the surface
/// reading, if it has a supported surface.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceMeasurement {
    /// Identity of the measured wrapper.
    pub reference: ElementId,
    /// Digest of the pinned source the surface was composed from.
    pub source_digest: ContentDigest,
    /// Version counter of the pinned source.
    pub source_version: u32,
    /// The surface reading; `None` when the wrapper composes no supported surface (an empty or
    /// invisible group), which is no measurement rather than a zero.
    pub mesh: Option<MeshReading>,
}

fn kernel_error(e: MeshError) -> CommandError {
    CommandError::new(
        e.code.f04_code(),
        "reference.kernel",
        "the kernel could not certify or measure the foreign surface",
    )
}

/// Measures the composed foreign surface of one wrapper.
///
/// Same supported profile as the view (`ReferenceGeometry`), but the full placement is applied:
/// a rotated wrapper is measured where it stands, not refused like the display path. Bounded by
/// the profile caps and the kernel's production limits; `cancelled` is polled by the kernel
/// between its stages and a cancelled call leaves no measurement.
pub fn measure_reference(
    record: &ReferenceRecord,
    geometry: &[ReferenceGeometry],
    cancelled: &mut dyn FnMut(Stage) -> bool,
) -> Result<ReferenceMeasurement, CommandError> {
    // The mapping record already carries the full placement, so the vertices are world
    // coordinates and the |v| half of the core's bound is the real one; a second translation
    // here would refuse legal far placements the kernel itself accepts (TriMesh re-checks
    // MAX_COORDINATE_MM on the world vertices anyway).
    let (points, indices) = foreign_surface(record, [0.0; 3], geometry)?;
    let no_surface = || ReferenceMeasurement {
        reference: record.id,
        source_digest: record.source.digest,
        source_version: record.source.version,
        mesh: None,
    };
    if points.is_empty() {
        return Ok(no_surface());
    }
    let mesh = TriMesh::from_buffers(&points, &indices).map_err(|e| {
        CommandError::new(
            e.code.f04_code(),
            "reference.geometry",
            "the foreign surface is not a bounded triangle mesh",
        )
    })?;
    let analysis = topology::analyze_with(&mesh, topology::Limits::DEFAULT, &mut *cancelled)
        .map_err(kernel_error)?;
    let measurement =
        measure::measure_with_cancel(&mesh, &analysis, &mut *cancelled).map_err(kernel_error)?;
    let blob = mesh.to_blob();
    let digest = Sha256::digest(&blob).map_err(|_| {
        CommandError::new(
            ErrorCode::LimitExceeded,
            "reference.geometry",
            "the measured mesh is out of the digest budget",
        )
    })?;
    Ok(ReferenceMeasurement {
        reference: record.id,
        source_digest: record.source.digest,
        source_version: record.source.version,
        mesh: Some(MeshReading {
            status: analysis.status(),
            mesh_digest: digest,
            bounds_mm: *analysis.bounds(),
            volume_mm3: measurement.volume_mm3(),
            area_mm2: measurement.area_mm2(),
            centroid_mm: measurement.centroid_mm(),
            // F06 section 8: a foreign mesh never carries a density or an engineering quantity.
            mass: measurement.mass(None),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_kernel::measure::QuantityStatus;
    use beamcraft_model::command::Placement;
    use beamcraft_model::digest::Sha256;
    use beamcraft_model::reference::{SourceFormat, SourcePin, UnitsInterpretation, UpAxis};

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn pin() -> SourcePin {
        SourcePin {
            digest: Sha256::digest(b"source").unwrap(),
            bytes: 6,
            format: SourceFormat::Usda,
            default_prim: "Site".into(),
            version: 2,
            label: "site.usda".into(),
            declared_meters_per_unit: Some(0.001),
            declared_up_axis: Some(UpAxis::Z),
        }
    }

    fn record(placement: Placement, units: Option<UnitsInterpretation>) -> ReferenceRecord {
        ReferenceRecord {
            id: id(0xa1),
            name: "Site".into(),
            placement,
            source: pin(),
            units,
        }
    }

    /// An outward tetrahedron with 10 mm legs, optionally shifted.
    fn tetra(shift: [f64; 3]) -> ReferenceGeometry {
        ReferenceGeometry {
            points: [
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [0.0, 10.0, 0.0],
                [0.0, 0.0, 10.0],
            ]
            .iter()
            .map(|p| [p[0] + shift[0], p[1] + shift[1], p[2] + shift[2]])
            .collect(),
            face_vertex_counts: vec![3; 4],
            face_vertex_indices: vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2],
            translation: [0.0; 3],
            left_handed: false,
        }
    }

    fn measured(
        record: &ReferenceRecord,
        geometry: &[ReferenceGeometry],
    ) -> Result<ReferenceMeasurement, CommandError> {
        measure_reference(record, geometry, &mut |_| false)
    }

    #[test]
    fn a_cube_is_certified_with_the_exact_values_inside_the_enclosures() {
        let reading = measured(
            &record(Placement::IDENTITY, None),
            &[ReferenceGeometry::cube(4.0, [0.0; 3]).unwrap()],
        )
        .unwrap()
        .mesh
        .unwrap();
        assert_eq!(reading.status, ShellStatus::Certified);
        let volume = 64.0_f64;
        assert!(
            reading.volume_mm3.unwrap().lo <= volume && volume <= reading.volume_mm3.unwrap().hi
        );
        let area = 96.0_f64;
        assert!(reading.area_mm2.unwrap().lo <= area && area <= reading.area_mm2.unwrap().hi);
        let centroid = reading.centroid_mm.unwrap();
        for axis in centroid {
            assert!(axis.lo <= 0.0 && 0.0 <= axis.hi);
        }
        assert_eq!(reading.bounds_mm.min, [-2.0; 3]);
        assert_eq!(reading.bounds_mm.max, [2.0; 3]);
        // Mass is never invented: unsupported, no value, by construction.
        assert_eq!(reading.mass.status, QuantityStatus::Unsupported);
        assert_eq!(reading.mass.kg, None);
    }

    #[test]
    fn a_far_surface_measures_instead_of_hitting_the_bound_twice() {
        // World vertices just inside the kernel's 4e9 mm cap: only the |v| half of the core's
        // coordinate check may see them, because the mapping already applied the translation;
        // composing the translation a second time would refuse this legal reading.
        let placement = Placement {
            translation_mm: [500_000_000.0, 0.0, 0.0],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        };
        let reading = measured(
            &record(placement, None),
            &[ReferenceGeometry::cube(4.0, [3_499_999_000.0, 0.0, 0.0]).unwrap()],
        )
        .unwrap()
        .mesh
        .unwrap();
        assert_eq!(reading.status, ShellStatus::Certified);
        assert_eq!(reading.bounds_mm.min, [3_999_998_998.0, -2.0, -2.0]);
        assert_eq!(reading.bounds_mm.max, [3_999_999_002.0, 2.0, 2.0]);
    }

    #[test]
    fn units_correction_and_a_rotated_placement_reach_the_values() {
        // A 2 m cube, corrected from metres and Y up, placed a quarter turn about Z over
        // (10, 20, 30): the volume is 2 m3 in project millimetres, the centroid stands at the
        // translation, and the bounds follow the rotated corners.
        let units = Some(UnitsInterpretation {
            meters_per_unit: 1.0,
            up_axis: UpAxis::Y,
        });
        let h = core::f64::consts::FRAC_1_SQRT_2;
        let placement = Placement {
            translation_mm: [10.0, 20.0, 30.0],
            orientation_xyzw: [0.0, 0.0, h, h],
        };
        let reading = measured(
            &record(placement, units),
            &[ReferenceGeometry::cube(2.0, [0.0; 3]).unwrap()],
        )
        .unwrap()
        .mesh
        .unwrap();
        assert_eq!(reading.status, ShellStatus::Certified);
        // A cube of side 2 m: 8 m3 of project millimetres.
        let volume = 8_000_000_000.0_f64;
        assert!(
            reading.volume_mm3.unwrap().lo <= volume && volume <= reading.volume_mm3.unwrap().hi
        );
        let centroid = reading.centroid_mm.unwrap();
        for (axis, want) in centroid.iter().zip([10.0, 20.0, 30.0]) {
            assert!(axis.lo <= want && want <= axis.hi);
        }
        // The quarter turn is axis-preserving for an axis-aligned cube, so the bounds follow
        // the translation to within the quaternion's float noise.
        let near = |got: [f64; 3], want: [f64; 3]| {
            for (a, b) in got.iter().zip(want) {
                assert!((a - b).abs() < 1e-6, "{got:?} != {want:?}");
            }
        };
        near(reading.bounds_mm.min, [-990.0, -980.0, -970.0]);
        near(reading.bounds_mm.max, [1010.0, 1020.0, 1030.0]);
    }

    #[test]
    fn a_shifted_foreign_prim_measures_where_it_stands() {
        let geometry = ReferenceGeometry::cube(4.0, [5.0, 0.0, 0.0]).unwrap();
        let reading = measured(&record(Placement::IDENTITY, None), &[geometry])
            .unwrap()
            .mesh
            .unwrap();
        assert_eq!(reading.status, ShellStatus::Certified);
        assert_eq!(reading.bounds_mm.min, [3.0, -2.0, -2.0]);
        assert_eq!(reading.bounds_mm.max, [7.0, 2.0, 2.0]);
    }

    #[test]
    fn an_open_surface_reports_its_status_and_no_value() {
        let open = ReferenceGeometry {
            points: vec![
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [0.0, 10.0, 0.0],
                [0.0, 0.0, 10.0],
            ],
            face_vertex_counts: vec![3, 3],
            face_vertex_indices: vec![0, 1, 2, 0, 1, 3],
            translation: [0.0; 3],
            left_handed: false,
        };
        let reading = measured(&record(Placement::IDENTITY, None), &[open])
            .unwrap()
            .mesh
            .unwrap();
        assert_eq!(reading.status, ShellStatus::Open);
        assert_eq!(reading.volume_mm3, None);
        assert_eq!(reading.area_mm2, None);
        assert_eq!(reading.centroid_mm, None);
        assert_eq!(reading.mass.status, QuantityStatus::Unsupported);
        assert_eq!(reading.bounds_mm.max, [10.0, 10.0, 10.0]);
    }

    #[test]
    fn self_intersecting_shells_are_never_measured_exactly() {
        let reading = measured(
            &record(Placement::IDENTITY, None),
            &[tetra([0.0; 3]), tetra([2.0, 2.0, 2.0])],
        )
        .unwrap()
        .mesh
        .unwrap();
        assert_eq!(reading.status, ShellStatus::Overlapping);
        assert_eq!(reading.volume_mm3, None);
        assert_eq!(reading.area_mm2, None);
        assert_eq!(reading.centroid_mm, None);
        assert_eq!(reading.mass.kg, None);
    }

    #[test]
    fn disjoint_shells_certify_and_sum() {
        let reading = measured(
            &record(Placement::IDENTITY, None),
            &[tetra([0.0; 3]), tetra([100.0, 0.0, 0.0])],
        )
        .unwrap()
        .mesh
        .unwrap();
        assert_eq!(reading.status, ShellStatus::Certified);
        let volume = 2.0 * (1000.0 / 6.0);
        assert!(
            reading.volume_mm3.unwrap().lo <= volume && volume <= reading.volume_mm3.unwrap().hi
        );
    }

    #[test]
    fn a_wrapper_without_a_supported_surface_is_no_measurement_not_a_zero() {
        let none = measured(&record(Placement::IDENTITY, None), &[]).unwrap();
        assert_eq!(none.reference, id(0xa1));
        assert_eq!(none.source_digest, pin().digest);
        assert_eq!(none.source_version, 2);
        assert_eq!(none.mesh, None);
    }

    #[test]
    fn the_measurement_is_a_pure_function_of_the_inputs() {
        let geometry = [ReferenceGeometry::cube(4.0, [0.0; 3]).unwrap()];
        let first = measured(&record(Placement::IDENTITY, None), &geometry).unwrap();
        let second = measured(&record(Placement::IDENTITY, None), &geometry).unwrap();
        assert_eq!(first, second);
        let mut moved = record(Placement::IDENTITY, None);
        moved.placement.translation_mm = [7.0, 0.0, 0.0];
        let third = measured(&moved, &geometry).unwrap();
        assert_ne!(first, third);
    }

    #[test]
    fn a_cancelled_measurement_leaves_nothing_behind() {
        let mut first_poll = true;
        let mut cancel = |_stage: Stage| !std::mem::take(&mut first_poll);
        let error = measure_reference(
            &record(Placement::IDENTITY, None),
            &[ReferenceGeometry::cube(4.0, [0.0; 3]).unwrap()],
            &mut cancel,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled);
    }
}
