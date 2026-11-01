// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G14.1 first slice: a transverse hole through the flat walls of a member section.
//!
//! The body is a straight extrusion along X (the member axis): its lowest and highest
//! vertices define two end planes `x0 < x1`, every triangle at `x0` or at `x1` is an end
//! cap (one face each, facing outward), and every other triangle projects to an exact
//! zero-area region in the YZ plane, which allows extra vertices at intermediate positions
//! along the axis on the walls. The cutter is a G05 cavity (round, oval or polygonal;
//! clockwise in XY) swept over a `z` range: the hole axis is the section direction `Z` of
//! the hole frame, perpendicular to the member axis.
//!
//! Rules (the F06 member-hole amendment, first slice):
//!
//! * Both ends of the cutter lie in free space: every wall it enters it also leaves, so a
//!   range ending inside a wall is refused.
//! * At every wall the footprint lies strictly inside one flat wall face, clear of that
//!   face's edges, of other holes in the same face and of the face's own holes by at least
//!   the geometric floor; contact is refused. A flat face the cutter passes over free
//!   space (the footprint lies entirely beyond its region) is left untouched.
//! * A wall whose two faces are flat and parallel (a constant `z` each) removes the polygon
//!   area times the wall thickness, exact; the volume check keeps that honest.
//! * A curved wall (a fillet, a rounded corner, a tube) or a sloped wall inside the cutter
//!   range is refused in this slice.
//! * The footprint clears the end caps and every wall parallel to the cutter axis by at
//!   least the floor.
//!
//! Construction: each crossed wall face is re-triangulated with the footprint added as one
//! more hole (the face's own loops are kept vertex for vertex), the cutter's own wall is
//! added between the two openings of every crossed wall, and the candidate is certified.
//! Genus grows by one per crossed wall and the volume must equal the body volume minus the
//! sum of the exact wall prisms `area * thickness`.
//!
//! Local feature ids: the wall of the hole at crossing `k` and polygon side `j` is
//! `cut:f.h<ordinal>.w<k>.s<j>` (provisional, like the plane-cut cap id); the crossed wall
//! faces keep their own ids.

use beamcraft_kernel::bigint::Big;
use beamcraft_kernel::diagnostic::Stage;
use beamcraft_kernel::enclosure::Enclosure;
use beamcraft_kernel::exact::{min_exponent, scaled};
use beamcraft_kernel::ops::body::{Body, MAX_ORDINAL};
use beamcraft_kernel::ops::diagnostic::{OpCode, OpDefect, OpFailure, OpField};
use beamcraft_kernel::ops::hole::cutter_field;
use beamcraft_kernel::ops::seal::{Candidate, Expect, OpKind, OpResult, seal};
use beamcraft_kernel::ops::section::{
    CapLoop, ChainError, area_sign, canonical_loop, chain_loops, split_loops, triangulate_cap,
};
use beamcraft_kernel::ops::{ExactBudget, OpLimits, OpStats, poll};
use beamcraft_kernel::profile::predicates::{
    SegRel, boxes_apart, segment_distance, segment_relation, winding_contains,
};
use beamcraft_kernel::profile::{Cavity, MAX_CONTOUR_POINTS, Point2};
use beamcraft_kernel::trimesh::MAX_COORDINATE_MM;

/// A straight cutter across a member: a clockwise cavity polygon in the body's XY plane
/// (the hole-frame section plane) swept over `[z_from_mm, z_to_mm]` along Z.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberHoleCutter {
    cavity: Cavity,
    z_from_mm: f64,
    z_to_mm: f64,
    ordinal: u16,
}

impl MemberHoleCutter {
    /// Creates a cutter without validating it; [`member_through_hole`] validates everything.
    ///
    /// `ordinal` (at most 999) names the hole walls; the caller (the A04 command) owns the
    /// mapping from its authored hole to the ordinal and must keep it stable.
    pub const fn new(cavity: Cavity, z_from_mm: f64, z_to_mm: f64, ordinal: u16) -> Self {
        Self {
            cavity,
            z_from_mm,
            z_to_mm,
            ordinal,
        }
    }

    /// The cavity polygon.
    pub const fn cavity(&self) -> &Cavity {
        &self.cavity
    }

    /// Lower end of the swept range in millimetres.
    pub const fn z_from_mm(&self) -> f64 {
        self.z_from_mm
    }

    /// Upper end of the swept range in millimetres.
    pub const fn z_to_mm(&self) -> f64 {
        self.z_to_mm
    }

    /// The ordinal naming the hole walls.
    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }
}

/// Subtracts `cutter` from `body` with the default bounds and no cancellation.
pub fn member_through_hole(body: &Body, cutter: &MemberHoleCutter) -> Result<OpResult, OpFailure> {
    member_through_hole_with(body, cutter, &OpLimits::DEFAULT, |_| false)
}

/// One flat wall face the cutter crosses: its height, its entry direction and its boundary
/// loops in the projected XY plane (outer counter-clockwise, holes clockwise).
struct CrossedFace {
    face: u16,
    height: f64,
    entry: bool,
    outer: CapLoop,
    holes: Vec<CapLoop>,
}

/// Subtracts with explicit bounds and the F06 cancellation points: `Validation` at entry,
/// `Build` before the body analysis, `Checks` before the candidate's certification (whose
/// own G06 polls follow) and `Quantities` before sealing.
pub fn member_through_hole_with(
    body: &Body,
    cutter: &MemberHoleCutter,
    limits: &OpLimits,
    mut cancelled: impl FnMut(Stage) -> bool,
) -> Result<OpResult, OpFailure> {
    poll(&mut cancelled, Stage::Validation)?;
    let refuse = |code, defect, stage| OpFailure::new(code, OpField::Cutter, defect, stage);
    let internal = || OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Build);
    let points = cutter.cavity.points();
    let m = points.len();
    if m > MAX_CONTOUR_POINTS {
        return Err(refuse(OpCode::AboveLimit, OpDefect::None, Stage::Caps));
    }
    if cutter.ordinal > MAX_ORDINAL {
        return Err(refuse(OpCode::AboveLimit, OpDefect::Ordinal, Stage::Caps));
    }
    if !(cutter.z_from_mm.is_finite() && cutter.z_to_mm.is_finite())
        || points.iter().any(|p| !(p.x.is_finite() && p.y.is_finite()))
    {
        return Err(refuse(OpCode::NonFinite, OpDefect::None, Stage::Finiteness));
    }
    if [cutter.z_from_mm, cutter.z_to_mm]
        .iter()
        .chain(points.iter().flat_map(|p| [&p.x, &p.y]))
        .any(|v| v.abs() > MAX_COORDINATE_MM)
    {
        return Err(refuse(OpCode::AboveLimit, OpDefect::None, Stage::Caps));
    }
    if m < 3 {
        return Err(refuse(OpCode::ContourInvalid, OpDefect::None, Stage::Caps));
    }
    if cutter.z_from_mm >= cutter.z_to_mm {
        return Err(refuse(
            OpCode::NotPositive,
            OpDefect::CutterRange,
            Stage::Ranges,
        ));
    }
    let stem = format!("cut:f.h{}", cutter.ordinal);
    if body
        .face_table()
        .iter()
        .any(|id| id == &stem || id.starts_with(&format!("{stem}.")))
    {
        return Err(OpFailure::new(
            OpCode::BlobInvalid,
            OpField::FaceTable,
            OpDefect::Ordinal,
            Stage::Validation,
        ));
    }
    poll(&mut cancelled, Stage::Build)?;

    let mut budget = ExactBudget::new(limits.max_exact_predicates);
    let solid = analyze_extrusion(body, &mut budget)?;

    let flat = |p: [f64; 3]| [p[0], p[1]];
    let eps = body.tolerance().eps_geom_mm();
    let (zf, zt) = (cutter.z_from_mm, cutter.z_to_mm);
    let (p_min_x, p_max_x) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.x), hi.max(p.x))
        });
    // The footprint clears both end faces of the member by at least the floor.
    let (near, far) = (p_min_x - solid.x0, solid.x1 - p_max_x);
    let end_refusal = if near < 0.0 || far < 0.0 {
        Some(OpDefect::CutterNotThrough)
    } else if near == 0.0 || far == 0.0 {
        Some(OpDefect::CutterTangent)
    } else if near < eps || far < eps {
        Some(OpDefect::CutterMargin)
    } else {
        None
    };
    if let Some(defect) = end_refusal {
        return Err(refuse(OpCode::OpUnsupported, defect, Stage::Ranges));
    }

    // Classify every wall face against the cutter range. A flat constant-z face inside the
    // range is crossed only where the footprint lies strictly inside its region; a face the
    // footprint passes beyond is free space. Curved and sloped faces must clear the range.
    let mut crossed: Vec<CrossedFace> = Vec::new();
    let (p_min_y, p_max_y) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.y), hi.max(p.y))
        });
    // A vertical wall of an earlier Z hole: clear when its bounding box clears the
    // footprint by the floor in X or Y, or its z extent clears the range. The box is
    // conservative in the footprint's favour and never accepts a contact.
    for triangles in solid.drilled.values() {
        let (x_lo, x_hi, y_lo, y_hi, z_lo, z_hi) = triangles.iter().fold(
            (
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ),
            |(xlo, xhi, ylo, yhi, zlo, zhi), tri| {
                tri.iter().fold(
                    (xlo, xhi, ylo, yhi, zlo, zhi),
                    |(xlo, xhi, ylo, yhi, zlo, zhi), i| match solid.position(*i) {
                        Some(p) => (
                            xlo.min(p[0]),
                            xhi.max(p[0]),
                            ylo.min(p[1]),
                            yhi.max(p[1]),
                            zlo.min(p[2]),
                            zhi.max(p[2]),
                        ),
                        None => (f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN),
                    },
                )
            },
        );
        if !(x_lo.is_finite() && z_hi.is_finite()) {
            return Err(internal());
        }
        let boxes_clear = (p_min_x - x_hi).max(x_lo - p_max_x) >= eps
            || (p_min_y - y_hi).max(y_lo - p_max_y) >= eps;
        if boxes_clear || z_clear(z_lo, z_hi, zf, zt, eps) {
            continue;
        }
        return Err(refuse(
            OpCode::ContourInvalid,
            OpDefect::None,
            Stage::Ranges,
        ));
    }
    for (face, triangles) in &solid.walls {
        let normal = body
            .face_normals()
            .get(usize::from(*face))
            .copied()
            .flatten();
        let coordinate = |i: u32, axis: usize| match solid.position(i) {
            Some(p) => match axis {
                0 => p[0],
                1 => p[1],
                _ => p[2],
            },
            None => f64::NAN,
        };
        let extent = |axis: usize| {
            triangles
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), tri| {
                    tri.iter().fold((lo, hi), |(lo, hi), i| {
                        let v = coordinate(*i, axis);
                        (lo.min(v), hi.max(v))
                    })
                })
        };
        let (z_lo, z_hi) = extent(2);
        if !(z_lo.is_finite() && z_hi.is_finite()) {
            return Err(internal());
        }
        let flat_z = z_lo == z_hi;
        let (y_lo, y_hi) = extent(1);
        let flat_y =
            y_lo == y_hi && normal.is_some_and(|n| n[0] == 0.0 && n[2] == 0.0 && n[1].abs() == 1.0);
        if !flat_z && !flat_y {
            // A curved or sloped wall face: clear only when the whole face lies beyond the
            // range by at least the floor; meeting it is G14.2 work.
            if z_hi >= zf - eps && z_lo <= zt + eps {
                return Err(OpFailure::unsupported(
                    OpField::Body,
                    OpDefect::BodyWallUnsupported,
                    Stage::Ranges,
                ));
            }
            continue;
        }
        if flat_z {
            let height = z_lo;
            let (inside, gap) = if height < zf {
                (false, zf - height)
            } else if height > zt {
                (false, height - zt)
            } else {
                (true, (height - zf).min(zt - height))
            };
            if gap == 0.0 {
                return Err(refuse(
                    OpCode::OpUnsupported,
                    OpDefect::CutterTangent,
                    Stage::Ranges,
                ));
            }
            if gap < eps {
                return Err(refuse(
                    OpCode::OpUnsupported,
                    OpDefect::CutterMargin,
                    Stage::Ranges,
                ));
            }
            if !inside {
                continue;
            }
            let entry = normal.is_some_and(|n| n[2] == -1.0);
            let (outer, holes) = face_loops(body, *face, triangles, limits, &mut budget)?;
            match footprint_placement(points, &outer, &holes, eps, &mut budget)? {
                Placement::Beyond => {}
                Placement::Within => crossed.push(CrossedFace {
                    face: *face,
                    height,
                    entry,
                    outer,
                    holes,
                }),
                Placement::Contact => {
                    // The footprint reaches the face's edge, one of its holes or another
                    // hole in the same face: refuse with the G05 diagnostic of the attempt
                    // (which must fail; its floor checks are the same clearance).
                    let m32 = u32::try_from(m).map_err(|_| internal())?;
                    let ring = CapLoop {
                        ids: (0..m32).collect(),
                        xyz: points.iter().map(|p| [p.x, p.y, height]).collect(),
                    };
                    let mut all_holes = holes.clone();
                    all_holes.push(ring);
                    let refusal = triangulate_cap(
                        &outer,
                        &all_holes,
                        flat,
                        limits,
                        body.tolerance(),
                        OpField::Cap,
                        false,
                    )
                    .map_err(|e| cutter_field(e, holes.len()))
                    .err()
                    .unwrap_or_else(internal);
                    return Err(refusal);
                }
            }
            continue;
        }
        // A wall parallel to the cutter axis (a flat face at a constant y): its projected
        // segment must clear the footprint, or its own z extent must clear the range.
        let (x_lo, x_hi) = extent(0);
        if !clear_of_segment(points, x_lo, x_hi, y_lo, eps, &mut budget)?
            && !z_clear(z_lo, z_hi, zf, zt, eps)
        {
            return Err(refuse(
                OpCode::ContourInvalid,
                OpDefect::None,
                Stage::Ranges,
            ));
        }
    }
    crossed.sort_by(|a, b| {
        a.height
            .partial_cmp(&b.height)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if crossed.is_empty() {
        return Err(refuse(OpCode::OpNoEffect, OpDefect::None, Stage::Build));
    }
    // Whole walls only: the crossed faces alternate entering and leaving material, start
    // with an entry and end with an exit.
    let ends_blind = match (crossed.first(), crossed.last()) {
        (Some(first), Some(last)) => !crossed.len().is_multiple_of(2) || !first.entry || last.entry,
        _ => true,
    };
    if ends_blind {
        return Err(refuse(
            OpCode::OpUnsupported,
            OpDefect::CutterNotThrough,
            Stage::Ranges,
        ));
    }
    let walls: Vec<(usize, usize, f64, f64)> = crossed
        .chunks(2)
        .map(|pair| match pair {
            [in_face, out_face]
                if in_face.entry && !out_face.entry && out_face.height > in_face.height =>
            {
                Some((
                    usize::from(in_face.face),
                    usize::from(out_face.face),
                    in_face.height,
                    out_face.height,
                ))
            }
            _ => None,
        })
        .collect::<Option<_>>()
        .ok_or_else(internal)?;
    let count = i64::try_from(walls.len()).map_err(|_| internal())?;

    poll(&mut cancelled, Stage::Checks)?;

    // New vertices: the footprint at the height of every crossed face.
    let base = u32::try_from(body.mesh().vertices().len()).map_err(|_| internal())?;
    let m32 = u32::try_from(m).map_err(|_| internal())?;
    let mut vertices: Vec<[f64; 3]> = body.mesh().vertices().to_vec();
    let mut rings: Vec<CapLoop> = Vec::new();
    for (index, crossed_face) in crossed.iter().enumerate() {
        let first = base + u32::try_from(index).map_err(|_| internal())? * m32;
        vertices.extend(points.iter().map(|p| [p.x, p.y, crossed_face.height]));
        rings.push(CapLoop {
            ids: (0..m32).map(|k| first + k).collect(),
            xyz: points
                .iter()
                .map(|p| [p.x, p.y, crossed_face.height])
                .collect(),
        });
    }
    let flat = |p: [f64; 3]| [p[0], p[1]];

    // The body keeps every triangle except those of the crossed faces.
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut faces: Vec<u16> = Vec::new();
    for (tri, face) in body.mesh().triangles().iter().zip(body.mesh().faces()) {
        if !crossed.iter().any(|c| &c.face == face) {
            triangles.push(*tri);
            faces.push(*face);
        }
    }

    // Re-triangulate every crossed face with the footprint added as one more hole.
    let mut loops = 0usize;
    let mut cap_points = 0usize;
    for (crossed_face, ring) in crossed.iter().zip(&rings) {
        let reverse = body
            .face_normals()
            .get(usize::from(crossed_face.face))
            .copied()
            .flatten()
            .is_some_and(|n| n[2] < 0.0);
        let mut all_holes = crossed_face.holes.clone();
        all_holes.push(ring.clone());
        let carved = triangulate_cap(
            &crossed_face.outer,
            &all_holes,
            flat,
            limits,
            body.tolerance(),
            OpField::Cap,
            false,
        )
        .map_err(|e| cutter_field(e, all_holes.len() - 1))?;
        loops += 1 + all_holes.len();
        cap_points +=
            crossed_face.outer.ids.len() + all_holes.iter().map(|h| h.ids.len()).sum::<usize>();
        for [a, b, c] in carved {
            triangles.push(if reverse { [a, c, b] } else { [a, b, c] });
            faces.push(crossed_face.face);
        }
    }

    // The cutter's own wall between the two openings of every crossed wall.
    let mut face_ids = body.face_table().to_vec();
    let mut normals = body.face_normals().to_vec();
    for (k, (_, _, _, _)) in walls.iter().enumerate() {
        let entry_ring = rings.get(2 * k).ok_or_else(internal)?;
        let exit_ring = rings.get(2 * k + 1).ok_or_else(internal)?;
        for j in 0..m {
            let l = (j + 1) % m;
            let (p, q) = (
                points.get(j).copied().ok_or_else(internal)?,
                points.get(l).copied().ok_or_else(internal)?,
            );
            face_ids.push(format!("cut:f.h{}.w{k}.s{j}", cutter.ordinal));
            let (dx, dy) = (q.x - p.x, q.y - p.y);
            let length = (dx * dx + dy * dy).sqrt();
            if !(length > 0.0 && length.is_finite()) {
                return Err(internal());
            }
            let n = [dy / length, -dx / length, 0.0];
            normals.push(Some(n.map(|v| if v == 0.0 { 0.0 } else { v })));
            let face = u16::try_from(face_ids.len() - 1).map_err(|_| internal())?;
            let ring_id = |ring: &CapLoop, i: usize| ring.ids.get(i).copied().ok_or_else(internal);
            let (bin, bin1, bout, bout1) = (
                ring_id(entry_ring, j)?,
                ring_id(entry_ring, l)?,
                ring_id(exit_ring, j)?,
                ring_id(exit_ring, l)?,
            );
            triangles.push([bin, bin1, bout1]);
            triangles.push([bin, bout1, bout]);
            faces.push(face);
            faces.push(face);
        }
    }

    let removed = removed_volume(&cutter.cavity, &walls)?;
    let negated = Enclosure {
        lo: -removed.hi,
        hi: -removed.lo,
    };
    let analytic = body.volume_mm3().add(negated).ok_or_else(|| {
        OpFailure::numeric(OpField::Volume, OpDefect::NumericRange, Stage::Quantities)
    })?;
    let shell = body.analysis().shells().first().ok_or_else(internal)?;
    let stats = OpStats {
        triangles_in: body.mesh().triangles().len(),
        triangles_out: 0,
        new_vertices: rings.len() * m,
        loops,
        cap_points,
        exact_predicates: budget.used(),
    };
    seal(
        Candidate {
            vertices,
            triangles,
            faces,
            face_ids,
            normals,
        },
        body.tolerance(),
        limits,
        Expect {
            kind: OpKind::MemberThroughHole,
            euler: shell.euler_characteristic.checked_sub(2 * count),
            genus: shell.genus.and_then(|g| {
                u32::try_from(walls.len())
                    .ok()
                    .and_then(|w| g.checked_add(w))
            }),
            analytic_volume: Some(analytic),
            plane_deviation_mm: None,
            stats,
        },
        &mut cancelled,
    )
}

/// The triangles of a body grouped by face id.
type FaceTriangles = std::collections::BTreeMap<u16, Vec<[u32; 3]>>;

/// The structure of an X-extrusion body: end planes, the triangles of every wall face of
/// the section extrusion and the vertical wall faces of holes already drilled along Z.
struct Solid {
    x0: f64,
    x1: f64,
    vertices: Vec<[f64; 3]>,
    walls: FaceTriangles,
    drilled: FaceTriangles,
}

impl Solid {
    fn position(&self, i: u32) -> Option<[f64; 3]> {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.vertices.get(i))
            .copied()
    }
}

/// Verifies the X-extrusion contract of the body: two end caps (one face each, facing
/// outward) and every other triangle projected to an exact zero-area region in YZ.
fn analyze_extrusion(body: &Body, budget: &mut ExactBudget) -> Result<Solid, OpFailure> {
    let not_solid = |defect| OpFailure::unsupported(OpField::Body, defect, Stage::Build);
    let mesh = body.mesh();
    let vertices = mesh.vertices().to_vec();
    let x0 = vertices.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let x1 = vertices
        .iter()
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max);
    if x0 >= x1 {
        return Err(not_solid(OpDefect::BodyNotPrism));
    }
    let position = |i: u32| {
        usize::try_from(i)
            .ok()
            .and_then(|i| vertices.get(i))
            .copied()
    };
    let mut walls: FaceTriangles = std::collections::BTreeMap::new();
    let mut drilled: FaceTriangles = std::collections::BTreeMap::new();
    let mut end_faces: Vec<u16> = Vec::new();
    for (tri, face) in mesh.triangles().iter().zip(mesh.faces()) {
        let [a, b, c] = tri.map(|i| position(i).ok_or_else(|| not_solid(OpDefect::BodyNotPrism)));
        let (a, b, c) = (a?, b?, c?);
        if a[0] == x1 && b[0] == x1 && c[0] == x1 {
            if area_sign(&[[a[1], a[2]], [b[1], b[2]], [c[1], c[2]]], budget)? <= 0 {
                return Err(not_solid(OpDefect::BodyNotPrism));
            }
            end_faces.push(*face);
        } else if a[0] == x0 && b[0] == x0 && c[0] == x0 {
            if area_sign(&[[a[1], a[2]], [b[1], b[2]], [c[1], c[2]]], budget)? >= 0 {
                return Err(not_solid(OpDefect::BodyNotPrism));
            }
            end_faces.push(*face);
        } else if area_sign(&[[a[1], a[2]], [b[1], b[2]], [c[1], c[2]]], budget)? == 0 {
            // A wall of the section extrusion, exactly parallel to the member axis.
            walls.entry(*face).or_default().push(*tri);
        } else if area_sign(&[[a[0], a[1]], [b[0], b[1]], [c[0], c[1]]], budget)? == 0 {
            // A vertical wall of a hole already drilled along Z (G09 or an earlier call).
            drilled.entry(*face).or_default().push(*tri);
        } else {
            return Err(not_solid(OpDefect::BodyNotPrism));
        }
    }
    let mut unique = end_faces;
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != 2 {
        return Err(not_solid(OpDefect::BodyCapFaces));
    }
    Ok(Solid {
        x0,
        x1,
        vertices,
        walls,
        drilled,
    })
}

/// The boundary loops of one wall face in the projected XY plane: the single outer loop
/// (counter-clockwise, reversed from the face winding when its normal points toward -Z)
/// and its holes (clockwise).
fn face_loops(
    body: &Body,
    face: u16,
    triangles: &[[u32; 3]],
    limits: &OpLimits,
    budget: &mut ExactBudget,
) -> Result<(CapLoop, Vec<CapLoop>), OpFailure> {
    let reverse = body
        .face_normals()
        .get(usize::from(face))
        .copied()
        .flatten()
        .is_some_and(|n| n[2] < 0.0);
    let position = |i: u32| {
        body.mesh()
            .vertices()
            .get(usize::try_from(i).ok()?)
            .copied()
    };
    let mut directed: Vec<(u32, u32)> = triangles
        .iter()
        .flat_map(|[a, b, c]| [(*a, *b), (*b, *c), (*c, *a)])
        .collect();
    directed.sort_unstable();
    let boundary: Vec<(u32, u32)> = directed
        .iter()
        .filter(|(a, b)| directed.binary_search(&(*b, *a)).is_err())
        .map(|&(a, b)| if reverse { (b, a) } else { (a, b) })
        .collect();
    let rings = chain_loops(&boundary, limits.max_loops).map_err(|e| match e {
        ChainError::TooManyLoops => OpFailure::new(
            OpCode::AboveLimit,
            OpField::Limits,
            OpDefect::BudgetLoops,
            Stage::Build,
        ),
        _ => OpFailure::unsupported(OpField::Body, OpDefect::BodyLoops, Stage::Build),
    })?;
    let positioned = rings
        .iter()
        .map(|r| canonical_loop(r, position))
        .collect::<Option<_>>()
        .ok_or_else(|| OpFailure::unsupported(OpField::Body, OpDefect::BodyLoops, Stage::Build))?;
    split_loops(
        positioned,
        |p| [p[0], p[1]],
        OpDefect::BodyLoops,
        OpField::Body,
        budget,
    )
}

/// The footprint's placement against one wall face's loops in the projected XY plane.
enum Placement {
    /// Every edge pair is disjoint and at least `eps` apart and no vertex of either side
    /// lies inside the other: the footprint lies entirely beyond the face region.
    Beyond,
    /// Every footprint vertex lies inside the outer loop and outside every hole, with all
    /// edge pairs at least `eps` apart: the cutter crosses the wall here.
    Within,
    /// Any other relation: an edge touch or crossing, a sub-floor clearance or a vertex
    /// overlap; refused as a footprint reaching the face's edge.
    Contact,
}

fn footprint_placement(
    points: &[Point2],
    outer: &CapLoop,
    holes: &[CapLoop],
    eps: f64,
    budget: &mut ExactBudget,
) -> Result<Placement, OpFailure> {
    let numeric = || OpFailure::numeric(OpField::Cutter, OpDefect::NumericRange, Stage::Build);
    let flat_ring = |ring: &CapLoop| -> Vec<Point2> {
        ring.xyz.iter().map(|p| Point2::new(p[0], p[1])).collect()
    };
    let rings = std::iter::once(outer).chain(holes.iter());
    let mut clear = true;
    for ring in rings.clone() {
        for (a, b) in ring.xyz.iter().zip(ring.xyz.iter().cycle().skip(1)) {
            let (a, b) = (Point2::new(a[0], a[1]), Point2::new(b[0], b[1]));
            for (p, q) in points.iter().zip(points.iter().cycle().skip(1)) {
                if boxes_apart(*p, *q, a, b, eps) {
                    continue;
                }
                match segment_relation(*p, *q, a, b) {
                    Ok(SegRel::Disjoint) => {
                        if segment_distance(*p, *q, a, b) < eps {
                            clear = false;
                        }
                    }
                    Ok(_) => return Ok(Placement::Contact),
                    Err(_) => {
                        budget.charge()?;
                        return Err(numeric());
                    }
                }
            }
        }
    }
    // No edge contact: the footprint is either entirely within the face region or entirely
    // beyond it, decided by vertex containment (an edge cannot cross without touching).
    let outer_points = flat_ring(outer);
    let hole_points: Vec<Vec<Point2>> = holes.iter().map(flat_ring).collect();
    let mut within = true;
    for v in points {
        match winding_contains(*v, &outer_points) {
            Ok(true) => {
                for hole in &hole_points {
                    match winding_contains(*v, hole) {
                        Ok(true) => within = false,
                        Ok(false) => {}
                        Err(_) => {
                            budget.charge()?;
                            return Err(numeric());
                        }
                    }
                }
            }
            Ok(false) => within = false,
            Err(_) => {
                budget.charge()?;
                return Err(numeric());
            }
        }
    }
    for ring in rings {
        for p in &ring.xyz {
            match winding_contains(Point2::new(p[0], p[1]), points) {
                Ok(true) => return Ok(Placement::Contact),
                Ok(false) => {}
                Err(_) => {
                    budget.charge()?;
                    return Err(numeric());
                }
            }
        }
    }
    if !clear {
        return Ok(Placement::Contact);
    }
    Ok(if within {
        Placement::Within
    } else {
        Placement::Beyond
    })
}

/// Whether a z extent clears the cutter range by at least the floor.
fn z_clear(z_lo: f64, z_hi: f64, zf: f64, zt: f64, eps: f64) -> bool {
    z_hi < zf - eps || z_lo > zt + eps
}

/// Whether the footprint polygon clears the projected segment `(x_lo, y) - (x_hi, y)` of a
/// wall parallel to the cutter axis: every edge pair disjoint and at least `margin` apart,
/// neither segment end inside the polygon and no footprint edge touching the segment.
fn clear_of_segment(
    points: &[Point2],
    x_lo: f64,
    x_hi: f64,
    y: f64,
    margin: f64,
    budget: &mut ExactBudget,
) -> Result<bool, OpFailure> {
    let numeric = || OpFailure::numeric(OpField::Cutter, OpDefect::NumericRange, Stage::Build);
    let seg = (Point2::new(x_lo, y), Point2::new(x_hi, y));
    let mut clear = true;
    for (p, q) in points.iter().zip(points.iter().cycle().skip(1)) {
        if boxes_apart(*p, *q, seg.0, seg.1, margin) {
            continue;
        }
        match segment_relation(*p, *q, seg.0, seg.1) {
            Ok(SegRel::Disjoint) => {
                if segment_distance(*p, *q, seg.0, seg.1) < margin {
                    clear = false;
                }
            }
            Ok(_) => return Ok(false),
            Err(_) => {
                budget.charge()?;
                return Err(numeric());
            }
        }
    }
    for end in [seg.0, seg.1] {
        match winding_contains(end, points) {
            Ok(inside) => {
                if inside {
                    return Ok(false);
                }
            }
            Err(_) => {
                budget.charge()?;
                return Err(numeric());
            }
        }
    }
    Ok(clear)
}

/// Exact removed volume `sum_k area * (z_out_k - z_in_k)` of the crossed walls, enclosed
/// outward. `walls` carries (entry face, exit face, entry height, exit height).
fn removed_volume(
    cavity: &Cavity,
    walls: &[(usize, usize, f64, f64)],
) -> Result<Enclosure, OpFailure> {
    let range = || OpFailure::numeric(OpField::Volume, OpDefect::NumericRange, Stage::Quantities);
    let points = cavity.points();
    let e = min_exponent(
        points
            .iter()
            .flat_map(|p| [p.x, p.y])
            .chain(walls.iter().flat_map(|(_, _, zin, zout)| [*zin, *zout])),
    );
    let s = |x: f64| scaled(x, e).map_err(|_| range());
    let mut area2 = Big::ZERO;
    for (p, q) in points.iter().zip(points.iter().cycle().skip(1)) {
        let left = s(p.x)?.mul(&s(q.y)?).map_err(|_| range())?;
        let right = s(q.x)?.mul(&s(p.y)?).map_err(|_| range())?;
        area2 = area2
            .add(&left.sub(&right).map_err(|_| range())?)
            .map_err(|_| range())?;
    }
    if area2.sign() < 0 {
        area2 = area2.negated();
    }
    let mut total = Enclosure { lo: 0.0, hi: 0.0 };
    for (_, _, zin, zout) in walls {
        let thickness = s(*zout)?.sub(&s(*zin)?).map_err(|_| range())?;
        let doubled = area2.mul(&thickness).map_err(|_| range())?;
        let pow2 = e
            .checked_mul(3)
            .and_then(|v| v.checked_sub(1))
            .ok_or_else(range)?;
        let term = doubled.enclose_clamped(pow2).ok_or_else(range)?;
        total = total.add(term).ok_or_else(range)?;
    }
    Ok(total)
}
