// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Pilot plane cut: keep one closed half-space of a certified body and cap the section.
//!
//! Scope (everything else is refused with a typed [`OpFailure`], never partially built):
//! a single-shell certified body, a finite plane that strictly crosses the body (some vertex
//! strictly on each side), no face of the body in the plane, and a section that is one outer
//! loop plus loops inside it. Vertices exactly on the plane are allowed when the plane still
//! crosses the body; they are used as they are and create no new vertex.
//!
//! Local feature ids: input faces keep their ids (a face that is clipped keeps its id and a
//! face whose triangles all disappear leaves the table and is listed in
//! [`OpResult::removed_faces`]); the cap is `cut:f.p<ordinal>`. Edges and vertices are not
//! named by this pilot; the engine's feature query names the cap corners from these face roles
//! (the cut cap corner amendment). The `cut` kind and the `p` role are provisional and need the same
//! contract amendment as the G05 `extrusion` kind.

use super::body::{Body, MAX_ORDINAL};
use super::diagnostic::{OpCode, OpDefect, OpFailure, OpField};
use super::plane::CutPlane;
use super::seal::{Candidate, Expect, OpKind, OpResult, seal};
use super::section::{
    CapLoop, ChainError, canonical_loop, chain_loops, split_loops, triangulate_cap,
};
use super::{ExactBudget, OpLimits, OpStats, poll};
use crate::diagnostic::Stage;
use crate::exact::project;
use core::cmp::Ordering;
use std::collections::BTreeMap;

/// Which closed half-space of the plane survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// The side the plane normal points to (`normal . (p - point) >= 0`).
    Positive,
    /// The opposite side (`normal . (p - point) <= 0`).
    Negative,
}

fn cmp_point(a: [f64; 3], b: [f64; 3]) -> Ordering {
    a[0].total_cmp(&b[0])
        .then(a[1].total_cmp(&b[1]))
        .then(a[2].total_cmp(&b[2]))
}

/// Cuts `body` by `plane`, keeping the `keep` half-space, with the default bounds and no
/// cancellation. `ordinal` (at most 999) names the cap `cut:f.p<ordinal>`.
pub fn plane_cut(
    body: &Body,
    plane: &CutPlane,
    keep: Keep,
    ordinal: u16,
) -> Result<OpResult, OpFailure> {
    plane_cut_with(body, plane, keep, ordinal, &OpLimits::DEFAULT, |_| false)
}

/// Cuts with explicit bounds and the F06 cancellation points: `Validation` at entry, `Build`
/// before the classification, `Checks` before the candidate's certification (whose own G06
/// polls follow), and `Quantities` before sealing. A cancelled call returns no result and
/// nothing polls after sealing.
pub fn plane_cut_with(
    body: &Body,
    plane: &CutPlane,
    keep: Keep,
    ordinal: u16,
    limits: &OpLimits,
    mut cancelled: impl FnMut(Stage) -> bool,
) -> Result<OpResult, OpFailure> {
    poll(&mut cancelled, Stage::Validation)?;
    plane.validate()?;
    if ordinal > MAX_ORDINAL {
        return Err(OpFailure::new(
            OpCode::AboveLimit,
            OpField::Plane,
            OpDefect::Ordinal,
            Stage::Caps,
        ));
    }
    let cap_id = format!("cut:f.p{ordinal}");
    if body.face_table().contains(&cap_id) {
        return Err(OpFailure::new(
            OpCode::BlobInvalid,
            OpField::FaceTable,
            OpDefect::Ordinal,
            Stage::Validation,
        ));
    }
    poll(&mut cancelled, Stage::Build)?;

    let mesh = body.mesh();
    let (vertices, triangles) = (mesh.vertices(), mesh.triangles());
    let mut budget = ExactBudget::new(limits.max_exact_predicates);
    let sense = match keep {
        Keep::Positive => 1i8,
        Keep::Negative => -1i8,
    };
    let sense_value = f64::from(sense);
    let mut sides: Vec<i8> = Vec::with_capacity(vertices.len());
    for p in vertices {
        let (sign, _) = plane.evaluate(*p, &mut budget)?;
        sides.push(sign * sense);
    }
    let count = |wanted: i8| sides.iter().filter(|s| **s == wanted).count();
    let (positive, zero, negative) = (count(1), count(0), count(-1));
    let refuse = |code, defect| Err(OpFailure::new(code, OpField::Plane, defect, Stage::Build));
    if negative == 0 {
        return if zero == 0 {
            refuse(OpCode::OpNoEffect, OpDefect::PlaneOutside)
        } else {
            refuse(OpCode::OpUnsupported, OpDefect::PlaneTangent)
        };
    }
    if positive == 0 {
        return if zero == 0 {
            refuse(OpCode::OpNoEffect, OpDefect::PlaneEmpty)
        } else {
            refuse(OpCode::OpUnsupported, OpDefect::PlaneTangent)
        };
    }
    let side_of = |i: u32| {
        usize::try_from(i)
            .ok()
            .and_then(|i| sides.get(i))
            .copied()
            .unwrap_or(0)
    };
    if triangles.iter().any(|t| t.iter().all(|&i| side_of(i) == 0)) {
        return refuse(OpCode::OpUnsupported, OpDefect::PlaneCoplanarFace);
    }

    // Clip every triangle against the plane; a crossing edge gets exactly one new vertex.
    let original = vertices.len();
    let mut out_vertices: Vec<[f64; 3]> = vertices.to_vec();
    let mut crossing: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut kept: Vec<[u32; 3]> = Vec::new();
    let mut kept_faces: Vec<u16> = Vec::new();
    let internal = || OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Build);
    let mut cross = |a: u32, b: u32, out: &mut Vec<[f64; 3]>| -> Result<u32, OpFailure> {
        let key = (a.min(b), a.max(b));
        if let Some(&v) = crossing.get(&key) {
            return Ok(v);
        }
        let at = |i: u32| {
            usize::try_from(i)
                .ok()
                .and_then(|i| vertices.get(i))
                .copied()
                .ok_or_else(internal)
        };
        let point = plane.intersection(at(key.0)?, at(key.1)?)?;
        let id = u32::try_from(out.len()).map_err(|_| internal())?;
        out.push(point);
        crossing.insert(key, id);
        Ok(id)
    };
    for (tri, face) in triangles.iter().zip(mesh.faces()) {
        let mut polygon: Vec<u32> = Vec::with_capacity(4);
        for k in 0..3 {
            let current = *tri.get(k).ok_or_else(internal)?;
            let next = *tri.get((k + 1) % 3).ok_or_else(internal)?;
            let (sc, sn) = (side_of(current), side_of(next));
            if sc >= 0 {
                polygon.push(current);
            }
            if sc * sn < 0 {
                polygon.push(cross(current, next, &mut out_vertices)?);
            }
        }
        match polygon.as_slice() {
            [a, b, c] => {
                kept.push([*a, *b, *c]);
                kept_faces.push(*face);
            }
            [a, b, c, d] => {
                let quad = [*a, *b, *c, *d];
                let position = |i: u32| {
                    usize::try_from(i)
                        .ok()
                        .and_then(|i| out_vertices.get(i))
                        .copied()
                        .unwrap_or([0.0; 3])
                };
                let first = (0..4)
                    .min_by(|&i, &j| {
                        cmp_point(
                            position(quad.get(i).copied().unwrap_or(0)),
                            position(quad.get(j).copied().unwrap_or(0)),
                        )
                    })
                    .unwrap_or(0);
                let q = |k: usize| quad.get((first + k) % 4).copied().ok_or_else(internal);
                kept.push([q(0)?, q(1)?, q(2)?]);
                kept.push([q(0)?, q(2)?, q(3)?]);
                kept_faces.push(*face);
                kept_faces.push(*face);
            }
            _ => {}
        }
    }

    // The boundary of the clipped surface lies in the plane: it is the section.
    let mut directed: Vec<(u32, u32)> = kept
        .iter()
        .flat_map(|[a, b, c]| [(*a, *b), (*b, *c), (*c, *a)])
        .collect();
    directed.sort_unstable();
    let pinched =
        || OpFailure::unsupported(OpField::Section, OpDefect::SectionPinched, Stage::Build);
    if directed.windows(2).any(|w| w.first() == w.get(1)) {
        return Err(pinched());
    }
    // The cap is triangulated in a frame fixed by the plane alone (the dominant axis of the
    // normal, seen from the side where that component is positive), so the two halves of a
    // plane and the two spellings (n, d) and (-n, -d) of it share one triangulation. The cap
    // faces away from the kept body; `along_m` says whether that is the frame's viewing side.
    let axis = plane.dominant_axis();
    let frame_positive = plane.normal().get(axis).copied().unwrap_or(0.0) > 0.0;
    let along_m = (sense > 0) != frame_positive;
    let boundary: Vec<(u32, u32)> = directed
        .iter()
        .filter(|(a, b)| directed.binary_search(&(*b, *a)).is_err())
        .map(|&(a, b)| if along_m { (b, a) } else { (a, b) })
        .collect();
    let on_plane = |i: u32| {
        usize::try_from(i).is_ok_and(|v| v >= original || sides.get(v).copied() == Some(0))
    };
    if boundary.is_empty() || boundary.iter().any(|(a, b)| !on_plane(*a) || !on_plane(*b)) {
        return Err(OpFailure::unsupported(
            OpField::Section,
            OpDefect::SectionOpen,
            Stage::Build,
        ));
    }
    let rings = chain_loops(&boundary, limits.max_loops).map_err(|e| match e {
        ChainError::Open => {
            OpFailure::unsupported(OpField::Section, OpDefect::SectionOpen, Stage::Build)
        }
        ChainError::Pinched => pinched(),
        ChainError::TooManyLoops => OpFailure::new(
            OpCode::AboveLimit,
            OpField::Limits,
            OpDefect::BudgetLoops,
            Stage::Build,
        ),
    })?;
    let position = |i: u32| {
        usize::try_from(i)
            .ok()
            .and_then(|i| out_vertices.get(i))
            .copied()
    };
    let loops: Vec<CapLoop> = rings
        .iter()
        .map(|r| canonical_loop(r, position))
        .collect::<Option<_>>()
        .ok_or_else(internal)?;
    let frame = move |p: [f64; 3]| project(p, axis);
    let (outer, holes) = split_loops(
        loops,
        frame,
        OpDefect::SectionMultiple,
        OpField::Section,
        &mut budget,
    )?;
    let cap = triangulate_cap(
        &outer,
        &holes,
        frame,
        limits,
        body.tolerance(),
        OpField::Section,
        true,
    )?;

    let mut deviation = 0.0f64;
    for p in out_vertices.iter().skip(original) {
        deviation = deviation.max(plane.deviation_bound_mm(*p)?);
    }

    let cap_face = u16::try_from(body.face_table().len()).map_err(|_| internal())?;
    for [a, b, c] in &cap {
        kept.push(if along_m { [*a, *b, *c] } else { [*a, *c, *b] });
        kept_faces.push(cap_face);
    }
    // Scale by the largest component first so tiny normals neither underflow nor overflow.
    let scale = plane.normal().iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let [nx, ny, nz] = plane.normal().map(|v| v / scale);
    let length = (nx * nx + ny * ny + nz * nz).sqrt();
    let flip = -sense_value;
    let normal = [nx / length * flip, ny / length * flip, nz / length * flip];
    if !normal.iter().all(|v| v.is_finite()) {
        return Err(internal());
    }
    let mut face_ids = body.face_table().to_vec();
    face_ids.push(cap_id);
    let mut normals = body.face_normals().to_vec();
    normals.push(Some(normal.map(|v| if v == 0.0 { 0.0 } else { v })));

    let stats = OpStats {
        triangles_in: triangles.len(),
        triangles_out: 0,
        new_vertices: out_vertices.len() - original,
        loops: 1 + holes.len(),
        cap_points: outer.ids.len() + holes.iter().map(|h| h.ids.len()).sum::<usize>(),
        exact_predicates: budget.used(),
    };
    seal(
        Candidate {
            vertices: out_vertices,
            triangles: kept,
            faces: kept_faces,
            face_ids,
            normals,
        },
        body.tolerance(),
        limits,
        Expect {
            kind: OpKind::PlaneCut,
            euler: None,
            genus: None,
            analytic_volume: None,
            plane_deviation_mm: Some(deviation),
            stats,
        },
        &mut cancelled,
    )
}
