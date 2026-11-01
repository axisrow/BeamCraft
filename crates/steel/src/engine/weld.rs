// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A06 welds: the engine's decision on `CreateWeld` (domain vocabulary sections 2.3 and 5.1).
//!
//! A weld is a `connection` element whose one `connects` record names exactly two distinct parts,
//! the main part first. The engine never touches USD: the adapter reads the kinds of the model's
//! elements into a [`Parts`] value, and the decision here is pure, bounded and deterministic. The
//! layer engine re-checks the same rules on the authored candidate, so a defect in either place
//! fails closed.
//!
//! Rules: the identity is free (`DUPLICATE_ID`); both parts exist (`NOT_FOUND`) and are plates,
//! beams or columns (`DV-REL-003`, `INVALID_ARGUMENT`); they are distinct and not the weld itself
//! (`DV-REL-006`, `DV-REL-007`, admitted by the command). The vocabulary also allows an assembly
//! as a `connects` target; a weld joins parts, so this slice refuses it as an unsupported target.

use crate::model::command::{CreateWeld, WeldGeometry};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::solid::SolidMesh;
use beamcraft_geom::{Frame3, Point3, RigidTransform, Vec3};
use beamcraft_kernel::profile::{Point2, extrude};
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::BTreeMap;

/// The kind of an element as far as a weld is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    /// A plate, beam or column: a part a weld can join.
    Part,
    /// An assembly: a valid `connects` target in the vocabulary, not supported for a weld.
    Assembly,
    /// Any other element (a weld, a reference wrapper, unknown content).
    Other,
}

/// The elements of a model and their kinds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parts {
    pub(crate) kinds: BTreeMap<ElementId, PartKind>,
}

fn error(code: ErrorCode, field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(code, field, message)
}

impl Parts {
    /// Records an element. A repeated identity keeps the last kind.
    pub fn insert(&mut self, id: ElementId, kind: PartKind) {
        self.kinds.insert(id, kind);
    }

    /// The kind of an element, if it is in the model.
    pub fn kind(&self, id: &ElementId) -> Option<PartKind> {
        self.kinds.get(id).copied()
    }

    /// Decides a weld creation and returns its footprint. Errors leave nothing applied.
    pub fn create(&self, command: &CreateWeld) -> Result<Footprint, CommandError> {
        command.validate_arguments()?;
        if self.kinds.contains_key(&command.new_id) {
            return Err(error(
                ErrorCode::DuplicateId,
                "new_id",
                "the identity is already in use",
            ));
        }
        for (id, field) in [
            (&command.main_id, "main_id"),
            (&command.secondary_id, "secondary_id"),
        ] {
            match self.kind(id) {
                None => {
                    return Err(error(
                        ErrorCode::NotFound,
                        field,
                        "the part is not in the head revision",
                    ));
                }
                Some(PartKind::Part) => {}
                Some(PartKind::Assembly) => {
                    return Err(error(
                        ErrorCode::UnsupportedGeometry,
                        field,
                        "a weld joins plates, beams or columns, not assemblies",
                    ));
                }
                Some(PartKind::Other) => {
                    return Err(error(
                        ErrorCode::InvalidArgument,
                        field,
                        "a weld joins plates, beams or columns",
                    ));
                }
            }
        }
        Footprint::for_weld(command.new_id, [command.main_id, command.secondary_id])
    }
}

/// The solid of one geometric weld (A15): the sealed mesh of the whole path, its analytic volume
/// and the signature of the inputs the volume was computed from.
#[derive(Debug, Clone, PartialEq)]
pub struct WeldSolid {
    mesh: SolidMesh,
    length_mm: f64,
    volume_mm3: f64,
    shape_signature: ContentDigest,
}

impl WeldSolid {
    /// The sealed mesh of the whole path.
    pub const fn mesh(&self) -> &SolidMesh {
        &self.mesh
    }
    /// Total path length in millimetres.
    pub const fn length_mm(&self) -> f64 {
        self.length_mm
    }
    /// Analytic volume in mm3: the sum of the per-segment certified extrusion volumes, which is
    /// `size^2 / 2 * length` for a fillet weld.
    pub const fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }
    /// Signature of the shape inputs of the volume.
    pub const fn shape_signature(&self) -> ContentDigest {
        self.shape_signature
    }
}

/// The world length of a polyline, summed over its segments.
pub fn path_length(path: &[[f64; 3]]) -> f64 {
    path.windows(2)
        .map(|pair| match pair {
            [from, to] => {
                let [fx, fy, fz] = from;
                let [tx, ty, tz] = to;
                let (dx, dy, dz) = (tx - fx, ty - fy, tz - fz);
                (dx * dx + dy * dy + dz * dz).sqrt()
            }
            _ => 0.0,
        })
        .sum()
}

/// Builds the solid of a fillet weld (A15) along `geometry.path_mm`.
///
/// Every segment carries one right-triangle prism whose legs are the weld size, extruded by the
/// kernel (G05) with its per-segment volume certificate, and placed by a deterministic frame
/// rule: the segment tangent, the reference up axis (Z, then X only when the tangent is nearly
/// vertical) and the perpendicular the up axis projects onto. The volume is analytic by
/// construction: the prism area is `size^2 / 2`, so the whole solid is `size^2 / 2 * length`.
/// Segment treatment at corners is plain butt joins, so the length is exactly the polyline
/// length; mitred corners stay open (they are G12's sweep work, not this slice's).
pub fn solid(size_mm: f64, geometry: &WeldGeometry) -> Result<WeldSolid, CommandError> {
    let unsupported = || {
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "path_mm",
            "the weld solid failed the kernel admission",
        )
    };
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut volume = 0.0;
    for pair in geometry.path_mm.windows(2) {
        let [from, to] = pair else {
            continue;
        };
        let [fx, fy, fz] = from;
        let [tx, ty, tz] = to;
        let (dx, dy, dz) = (tx - fx, ty - fy, tz - fz);
        let length = (dx * dx + dy * dy + dz * dz).sqrt();
        if !length.is_finite() || length <= 0.0 {
            return Err(unsupported());
        }
        let tangent = [dx / length, dy / length, dz / length];
        // The reference up axis, falling back to X only for a near-vertical tangent.
        let up = if tangent[2].abs() > 0.999 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        let projection = up[0] * tangent[0] + up[1] * tangent[1] + up[2] * tangent[2];
        let n = [
            up[0] - projection * tangent[0],
            up[1] - projection * tangent[1],
            up[2] - projection * tangent[2],
        ];
        let n = Vec3::new(n[0], n[1], n[2])
            .map_err(|_| unsupported())?
            .normalized()
            .map_err(|_| unsupported())?;
        let t = Vec3::new(tangent[0], tangent[1], tangent[2]).map_err(|_| unsupported())?;
        let b = n.cross(t).map_err(|_| unsupported())?;
        let frame = Frame3::try_new(b, n, t).map_err(|_| unsupported())?;
        let transform =
            RigidTransform::from_body_frame(frame, Point3::new(from[0], from[1], from[2]))
                .map_err(|_| unsupported())?;
        // The fillet cross-section: a right triangle with both legs equal to the weld size,
        // extruded along the local +Z by the segment length.
        let outer = [
            Point2 { x: 0.0, y: 0.0 },
            Point2 { x: size_mm, y: 0.0 },
            Point2 { x: 0.0, y: size_mm },
        ];
        let extrusion = extrude(&outer, &[], length).map_err(|_| unsupported())?;
        let placed = TriMesh::from_blob(extrusion.mesh().blob())
            .map_err(|_| unsupported())?
            .placed(&transform)
            .map_err(|_| unsupported())?;
        volume += extrusion.volume_mm3();
        let offset = vertices.len() as u32;
        vertices.extend(placed.vertices().iter().copied());
        for triangle in placed.triangles() {
            triangles.push([
                triangle[0] + offset,
                triangle[1] + offset,
                triangle[2] + offset,
            ]);
        }
    }
    let merged = TriMesh::from_buffers(&vertices, &triangles).map_err(|_| unsupported())?;
    let mesh = SolidMesh::composed(&merged, volume).ok_or_else(unsupported)?;
    Ok(WeldSolid {
        mesh,
        length_mm: path_length(&geometry.path_mm),
        volume_mm3: volume,
        shape_signature: signature(size_mm, geometry)?,
    })
}

/// The signature of the inputs a weld solid's volume was computed from.
fn signature(size_mm: f64, geometry: &WeldGeometry) -> Result<ContentDigest, CommandError> {
    let mut bytes = Vec::with_capacity(64 + geometry.path_mm.len() * 24);
    bytes.extend_from_slice(b"BeamCraft/A15/WeldSolid/v1\0");
    bytes.extend_from_slice(&beamcraft_geom::canonical_f64_bytes(size_mm).map_err(|_| {
        CommandError::new(
            ErrorCode::NonFiniteNumber,
            "size_mm",
            "weld size must be finite",
        )
        .with_rule("DV-ATTR-003")
    })?);
    for point in &geometry.path_mm {
        for coordinate in point {
            bytes.extend_from_slice(&beamcraft_geom::canonical_f64_bytes(*coordinate).map_err(
                |_| {
                    CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "path_mm",
                        "a weld path point must be finite",
                    )
                    .with_rule("DV-ATTR-003")
                },
            )?);
        }
    }
    Sha256::digest(&bytes)
        .map_err(|_| CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::command::{WeldLocation, WeldSide, WeldSpec, WeldType};
    use beamcraft_engine::footprint::{Entry, Group, Target};

    fn id(n: u32) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000004{n:05}")).unwrap()
    }

    fn command(main: u32, secondary: u32) -> CreateWeld {
        CreateWeld {
            new_id: id(9),
            name: "W".into(),
            main_id: id(main),
            secondary_id: id(secondary),
            weld: WeldSpec {
                weld_type: WeldType::Fillet,
                size_mm: 5.0,
                location: WeldLocation::Site,
                side: WeldSide::Other,
            },
            geometry: None,
        }
    }

    fn parts() -> Parts {
        let mut parts = Parts::default();
        parts.insert(id(1), PartKind::Part);
        parts.insert(id(2), PartKind::Part);
        parts.insert(id(3), PartKind::Assembly);
        parts.insert(id(4), PartKind::Other);
        parts
    }

    #[test]
    fn a_weld_reads_both_parts_and_writes_only_itself() {
        let footprint = parts().create(&command(1, 2)).unwrap();
        for part in [id(1), id(2)] {
            let lifecycle = Entry {
                target: Target::Element(part),
                group: Group::Lifecycle,
            };
            assert!(footprint.reads().contains(&lifecycle));
            assert!(
                !footprint
                    .writes()
                    .iter()
                    .any(|e| e.target == lifecycle.target)
            );
        }
        assert_eq!(footprint.writes().len(), 7);
        assert!(
            footprint
                .writes()
                .iter()
                .all(|e| e.target == Target::Element(id(9)))
        );
    }

    #[test]
    fn missing_wrong_kind_and_taken_identities_are_refused() {
        let parts = parts();
        let code = |c: CreateWeld| parts.create(&c).unwrap_err().code;
        assert_eq!(code(command(1, 7)), ErrorCode::NotFound);
        assert_eq!(code(command(3, 1)), ErrorCode::UnsupportedGeometry);
        assert_eq!(code(command(1, 4)), ErrorCode::InvalidArgument);
        assert_eq!(code(command(1, 1)), ErrorCode::InvalidArgument);
        let taken = CreateWeld {
            new_id: id(4),
            ..command(1, 2)
        };
        assert_eq!(code(taken), ErrorCode::DuplicateId);
    }

    fn geometry(path: &[[f64; 3]]) -> WeldGeometry {
        WeldGeometry {
            path_mm: path.to_vec(),
            symbol: crate::model::command::WeldSymbol {
                all_around: false,
                contour: crate::model::command::WeldContour::None,
            },
        }
    }

    #[test]
    fn a_fillet_solid_volume_is_half_the_size_squared_times_the_length() {
        // One straight segment: the prism area is size^2/2, so V = s^2/2 * L exactly.
        let straight = solid(6.0, &geometry(&[[0.0; 3], [100.0, 0.0, 0.0]])).unwrap();
        assert_eq!(straight.length_mm(), 100.0);
        assert_eq!(straight.volume_mm3(), 0.5 * 36.0 * 100.0);
        assert!(straight.mesh().vertices().len() >= 6);
        // The same size along a rotated, placed segment keeps the analytic area-length volume
        // (compared within a relative 1e-12: the length is irrational here).
        let lifted = solid(6.0, &geometry(&[[10.0, 20.0, 30.0], [40.0, 75.0, 110.0]])).unwrap();
        let expected = 0.5 * 36.0 * (30.0f64.powi(2) + 55.0f64.powi(2) + 80.0f64.powi(2)).sqrt();
        assert!(
            (lifted.volume_mm3() - expected).abs() <= expected.abs() * 1e-12,
            "{} vs {}",
            lifted.volume_mm3(),
            expected
        );
    }

    #[test]
    fn a_polyline_solid_sums_the_segment_volumes_and_keeps_the_full_length() {
        let path = [
            [0.0; 3],
            [100.0, 0.0, 0.0],
            [100.0, 50.0, 0.0],
            [100.0, 50.0, 25.0],
        ];
        let bent = solid(8.0, &geometry(&path)).unwrap();
        assert_eq!(bent.length_mm(), 175.0);
        assert_eq!(bent.volume_mm3(), 0.5 * 64.0 * 175.0);
        // One 3-vertex cross-section ring per path point.
        assert_eq!(bent.mesh().vertices().len(), 18);
    }

    #[test]
    fn a_degenerate_or_nonfinite_segment_is_refused_by_the_kernel_admission() {
        // The command validation refuses these first; the builder still fails closed.
        assert_eq!(
            solid(6.0, &geometry(&[[0.0; 3], [0.0; 3]]))
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        assert_eq!(
            solid(6.0, &geometry(&[[0.0; 3], [f64::NAN, 0.0, 0.0]]))
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGeometry
        );
        assert!(solid(6.0, &geometry(&[[0.0; 3], [0.0, 10.0, 0.0]])).is_ok());
    }

    #[test]
    fn the_solid_signature_binds_the_size_and_the_whole_path() {
        let base = solid(6.0, &geometry(&[[0.0; 3], [100.0, 0.0, 0.0]])).unwrap();
        let same = solid(6.0, &geometry(&[[0.0; 3], [100.0, 0.0, 0.0]])).unwrap();
        assert_eq!(base.shape_signature(), same.shape_signature());
        let bigger = solid(8.0, &geometry(&[[0.0; 3], [100.0, 0.0, 0.0]])).unwrap();
        let moved = solid(6.0, &geometry(&[[1.0, 0.0, 0.0], [101.0, 0.0, 0.0]])).unwrap();
        assert_ne!(base.shape_signature(), bigger.shape_signature());
        assert_ne!(base.shape_signature(), moved.shape_signature());
        assert_eq!(path_length(&[[0.0; 3], [3.0, 4.0, 0.0]]), 5.0);
    }
}
