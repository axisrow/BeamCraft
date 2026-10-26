// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Display-only foreign geometry; it never becomes a certified body or a quantity.

use super::{SceneElement, SceneMesh};
use crate::ElementKind;
use crate::reference::ReferenceRecord;
use beamcraft_geom::{Point3, Translation3};
use beamcraft_kernel::trimesh::{MAX_TRIANGLES, MAX_VERTICES, TriMesh};
use beamcraft_model::command::{CommandError, Placement};
use beamcraft_model::digest::Sha256;
use beamcraft_model::error::ErrorCode;

/// A decoded foreign polygon mesh, before explicit R01 units interpretation.
/// Triangles and convex planar quads are supported; no shell certification is implied.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceGeometry {
    /// Source-local vertex positions.
    pub points: Vec<[f64; 3]>,
    /// Number of corners of each face.
    pub face_vertex_counts: Vec<u32>,
    /// Vertex indices, in face order.
    pub face_vertex_indices: Vec<u32>,
    /// Accumulated translations of the foreign prim and its foreign ancestors, in source units.
    pub translation: [f64; 3],
    /// USD `orientation = "leftHanded"` reverses the face winding.
    pub left_handed: bool,
}

impl ReferenceGeometry {
    /// The polygonal surface of a USD Cube (centred at the origin, side length `size`).
    pub fn cube(size: f64, translation: [f64; 3]) -> Result<Self, CommandError> {
        if !size.is_finite() {
            return Err(error(ErrorCode::NonFiniteNumber));
        }
        if size <= 0.0 {
            return Err(error(ErrorCode::InvalidArgument));
        }
        let h = size / 2.0;
        Ok(Self {
            points: (0..8)
                .map(|bits| [1, 2, 4].map(|mask| if bits & mask == 0 { -h } else { h }))
                .collect(),
            face_vertex_counts: vec![4; 6],
            face_vertex_indices: vec![
                0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 1, 3, 7, 5, 3, 2, 6, 7, 2, 0, 4, 6,
            ],
            translation,
            left_handed: false,
        })
    }
}

fn error(code: ErrorCode) -> CommandError {
    CommandError::new(
        code,
        "reference.geometry",
        "foreign display geometry is invalid or unsupported",
    )
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross([x, y, z]: [f64; 3], [a, b, c]: [f64; 3]) -> [f64; 3] {
    [y * c - z * b, z * a - x * c, x * b - y * a]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Do not fan-triangulate concave, crossed or nonplanar quads into a different surface.
fn convex_quad([a, b, c, d]: [[f64; 3]; 4]) -> bool {
    let n = cross(sub(b, a), sub(c, b));
    let length = dot(n, n).sqrt();
    length > 0.0
        && dot(n, sub(d, a)).abs() <= length * 1e-6
        && [[b, c, d], [c, d, a], [d, a, b]]
            .into_iter()
            .all(|[p, q, r]| dot(n, cross(sub(q, p), sub(r, q))) > 0.0)
}

/// The world-space vertices and triangle corners of one wrapper's composed foreign surface.
pub(crate) type ForeignSurface = (Vec<[f64; 3]>, Vec<[u32; 3]>);

/// The world-space vertices and triangle corners of the composed foreign surfaces, mapped
/// through `record` (explicit units correction, then placement) with the supported profile's
/// caps, face rules and coordinate bounds. Both the display element and the R02 measurement
/// build on it; a caller adds what it alone needs (the rotation guard and the translation
/// instance for the view, kernel certification for the measurement).
///
/// `record` is the mapping (the display path passes a placement-identity copy so only the
/// correction reaches the vertices); `bound_translation` is the second bound of the coordinate
/// check `v` and `v + t`. A caller whose mapping already includes the placement translation
/// (the measurement) passes `[0.0; 3]`, because its vertices are world coordinates and only
/// `|v|` is real; the view passes its translation so the composed position is bounded before
/// the translation instance is applied.
pub(crate) fn foreign_surface(
    record: &ReferenceRecord,
    bound_translation: [f64; 3],
    geometry: &[ReferenceGeometry],
) -> Result<ForeignSurface, CommandError> {
    record.placement.validate()?;
    record.source.validate()?;
    if let Some(units) = record.units {
        units.validate()?;
    }
    if geometry.len() > MAX_VERTICES {
        return Err(error(ErrorCode::LimitExceeded));
    }
    let (count, faces, corners) = geometry
        .iter()
        .fold((0usize, 0usize, 0usize), |(p, f, i), g| {
            (
                p.saturating_add(g.points.len()),
                f.saturating_add(g.face_vertex_counts.len()),
                i.saturating_add(g.face_vertex_indices.len()),
            )
        });
    if count > MAX_VERTICES || faces > MAX_TRIANGLES || corners > MAX_TRIANGLES * 3 {
        return Err(error(ErrorCode::LimitExceeded));
    }
    let triangles = geometry
        .iter()
        .flat_map(|g| &g.face_vertex_counts)
        .fold(0usize, |n, count| {
            n.saturating_add(usize::try_from(count.saturating_sub(2)).unwrap_or(usize::MAX))
        });
    if triangles > MAX_TRIANGLES {
        return Err(error(ErrorCode::LimitExceeded));
    }
    if geometry.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut points = Vec::with_capacity(count);
    let mut indices = Vec::with_capacity(triangles);
    for g in geometry {
        if g.points.is_empty() || g.face_vertex_counts.is_empty() {
            return Err(error(ErrorCode::InvalidArgument));
        }
        let base = u32::try_from(points.len()).map_err(|_| error(ErrorCode::LimitExceeded))?;
        for p in &g.points {
            let p = record.source_point_to_world([
                p[0] + g.translation[0],
                p[1] + g.translation[1],
                p[2] + g.translation[2],
            ])?;
            if p.iter().zip(bound_translation).any(|(v, t)| {
                v.abs() > beamcraft_kernel::trimesh::MAX_COORDINATE_MM
                    || (v + t).abs() > beamcraft_kernel::trimesh::MAX_COORDINATE_MM
            }) {
                return Err(error(ErrorCode::LimitExceeded));
            }
            points.push(p);
        }
        let mut remaining = g.face_vertex_indices.as_slice();
        for count in &g.face_vertex_counts {
            // shortcut: triangle/convex-quad cages only, add bounded tessellation with a larger USD profile.
            if !matches!(count, 3 | 4) {
                return Err(error(ErrorCode::UnsupportedGeometry));
            }
            let (face, rest) = remaining
                .split_at_checked(*count as usize)
                .ok_or_else(|| error(ErrorCode::InvalidArgument))?;
            remaining = rest;
            let mut corners = Vec::with_capacity(face.len());
            for i in face {
                if usize::try_from(*i).ok().is_none_or(|i| i >= g.points.len()) {
                    return Err(error(ErrorCode::InvalidArgument));
                }
                corners.push(
                    base.checked_add(*i)
                        .ok_or_else(|| error(ErrorCode::LimitExceeded))?,
                );
            }
            if let [a, b, c, d] = corners.as_slice() {
                let mut quad = [[0.0; 3]; 4];
                for (p, i) in quad.iter_mut().zip([a, b, c, d]) {
                    *p = *points
                        .get(*i as usize)
                        .ok_or_else(|| error(ErrorCode::InvalidArgument))?;
                }
                if !convex_quad(quad) {
                    return Err(error(ErrorCode::UnsupportedGeometry));
                }
            }
            if g.left_handed {
                corners.reverse();
            }
            let first = *corners
                .first()
                .ok_or_else(|| error(ErrorCode::InvalidArgument))?;
            for edge in corners.get(1..).unwrap_or_default().windows(2) {
                let [b, c] = edge else {
                    return Err(error(ErrorCode::InvalidArgument));
                };
                indices.push([first, *b, *c]);
            }
        }
        if !remaining.is_empty() {
            return Err(error(ErrorCode::InvalidArgument));
        }
    }
    Ok((points, indices))
}

/// Combines a wrapper's visible foreign surfaces into one reference-kind scene element.
///
/// The wrapper placement remains a translation instance. Explicit R01 units/up-axis correction
/// is baked into its local display vertices, never into the foreign content. Declared source
/// units alone change nothing. A rotated wrapper is `UNSUPPORTED_GEOMETRY`, not drawn unrotated.
/// An empty group has no display representation. Foreign surfaces are not engineering solids;
/// picking/measurement support is not added by this conversion.
pub fn reference_element(
    record: &ReferenceRecord,
    geometry: &[ReferenceGeometry],
) -> Result<Option<SceneElement>, CommandError> {
    record.placement.validate()?;
    record.source.validate()?;
    if let Some(units) = record.units {
        units.validate()?;
    }
    if !record.placement.is_identity_rotation() {
        return Err(error(ErrorCode::UnsupportedGeometry));
    }
    let local = ReferenceRecord {
        placement: Placement::IDENTITY,
        ..record.clone()
    };
    let (points, indices) = foreign_surface(&local, record.placement.translation_mm, geometry)?;
    if points.is_empty() {
        return Ok(None);
    }
    let mesh = TriMesh::from_buffers(&points, &indices).map_err(|e| error(e.code.f04_code()))?;
    let blob = mesh.to_blob();
    let digest = Sha256::digest(&blob).map_err(|_| error(ErrorCode::LimitExceeded))?;
    let mesh = SceneMesh::new(
        blob,
        digest,
        mesh.vertices()
            .iter()
            .map(|[x, y, z]| Point3::new(*x, *y, *z))
            .collect(),
    );
    let [x, y, z] = record.placement.translation_mm;
    let placement = Translation3::new(x, y, z).map_err(|_| error(ErrorCode::NonFiniteNumber))?;
    Ok(Some(SceneElement::new(
        record.id,
        ElementKind::Reference,
        placement,
        mesh,
    )))
}
