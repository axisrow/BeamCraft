// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Transverse holes through the flat walls of a member section: G14.1, plus the G14.2
//! first slice of directed cutters that cross those walls obliquely.
//!
//! The body is a straight extrusion along X (the member axis): its lowest and highest
//! vertices define two end planes `x0 < x1`, every triangle at `x0` or at `x1` is an end
//! cap (one face each, facing outward), and every other triangle projects to an exact
//! zero-area region in the YZ plane, which allows extra vertices at intermediate positions
//! along the axis on the walls. The cutter is a G05 cavity (round, oval or polygonal;
//! clockwise in the section plane) swept over a range along a hole axis `d = (0, y, z)`
//! normalized, perpendicular to the member axis. [`MemberHoleCutter::new`] is the G14.1
//! form `d = +Z` and keeps its byte-exact results; [`MemberHoleCutter::directed`] admits
//! any other section direction.
//!
//! The prism frame is `(e1, e2, d)` with `e1 = +X` and `e2 = d x e1`: a body point maps to
//! footprint coordinates `(a, b) = (p . e1, p . e2)` and a height `t = p . d`, and the
//! cavity polygon is authoritative in `(a, b)`. For `d = +Z` the map is the identity on
//! `(x, y, z)`, so every existing G14.1 result is reproduced bit for bit.
//!
//! Rules (the F06 member-hole amendment):
//!
//! * Both ends of the cutter lie in free space: every wall it enters it also leaves, so a
//!   range ending inside a wall is refused (a wall whose height interval over the footprint
//!   straddles a range end is the same blind hole).
//! * At every wall the footprint lies strictly inside one flat wall face, clear of that
//!   face's edges, of other holes in the same face and of the face's own holes by at least
//!   the geometric floor; contact is refused. A flat face the cutter passes over free
//!   space (the footprint lies entirely beyond its region) is left untouched.
//! * A crossed wall removes the footprint area times the wall thickness along the hole
//!   axis (`A * t / |n . d|` for a wall normal `n`); the exact removed volume of the
//!   constructed opening certifies the candidate.
//! * A curved wall (a fillet, a rounded corner, a tube), or a sloped (not axis-aligned)
//!   wall plane inside the cutter range is refused in this slice: it needs the strip and
//!   kink registry of the later G14.2 slices.
//! * The footprint clears the end caps and every wall parallel to the cutter axis (a face
//!   whose `b` extent is degenerate) by at least the floor.
//!
//! Construction: each crossed wall face is re-triangulated with the footprint added as one
//! more hole (the face's own loops are kept vertex for vertex), the cutter's own wall is
//! added between the two openings of every crossed wall, and the candidate is certified.
//! Genus grows by one per crossed wall. Because an oblique cutter's walls are not parallel
//! to Z, a body already cut obliquely no longer satisfies the exact X-extrusion contract
//! above and a later member hole on it is refused (the re-merge is the strip registry's
//! work), so a perpendicular follow-up hole is only possible after perpendicular cuts.
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

/// A straight cutter across a member: a clockwise cavity polygon in the hole frame's
/// section plane (the `(a, b)` footprint plane) swept over `[from_mm, to_mm]` along the
/// hole axis `d = (0, axis_y, axis_z)` normalized, perpendicular to the member axis.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberHoleCutter {
    cavity: Cavity,
    from_mm: f64,
    to_mm: f64,
    axis: (f64, f64),
    ordinal: u16,
}

impl MemberHoleCutter {
    /// Creates the G14.1 cutter without validating it; [`member_through_hole`] validates
    /// everything.
    ///
    /// `ordinal` (at most 999) names the hole walls; the caller (the A04 command) owns the
    /// mapping from its authored hole to the ordinal and must keep it stable.
    pub const fn new(cavity: Cavity, z_from_mm: f64, z_to_mm: f64, ordinal: u16) -> Self {
        Self {
            cavity,
            from_mm: z_from_mm,
            to_mm: z_to_mm,
            axis: (0.0, 1.0),
            ordinal,
        }
    }

    /// Creates a directed cutter without validating it; [`member_through_hole`] validates
    /// everything, normalizes the axis and refuses a degenerate one.
    ///
    /// The axis is a section-plane direction `(axis_y, axis_z)`; an axial (X) component is
    /// the command layer's refusal and cannot be expressed here. `new` is the exact
    /// special case `(0, 1)` and keeps its byte-exact G14.1 results.
    pub const fn directed(
        cavity: Cavity,
        axis_y: f64,
        axis_z: f64,
        from_mm: f64,
        to_mm: f64,
        ordinal: u16,
    ) -> Self {
        Self {
            cavity,
            from_mm,
            to_mm,
            axis: (axis_y, axis_z),
            ordinal,
        }
    }

    /// The cavity polygon.
    pub const fn cavity(&self) -> &Cavity {
        &self.cavity
    }

    /// Lower end of the swept range in millimetres, measured along the hole axis.
    pub const fn from_mm(&self) -> f64 {
        self.from_mm
    }

    /// Upper end of the swept range in millimetres, measured along the hole axis.
    pub const fn to_mm(&self) -> f64 {
        self.to_mm
    }

    /// The hole axis `(y, z)` as authored, before normalization.
    pub const fn axis(&self) -> (f64, f64) {
        self.axis
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

/// One flat wall face the cutter crosses: the linear height `t = gamma * b + delta` of its
/// plane over the footprint, the midpoint of that height interval (the crossing order key),
/// its entry direction and its boundary loops (outer counter-clockwise, holes clockwise) in
/// the projected `(a, b)` plane.
struct CrossedFace {
    face: u16,
    gamma: f64,
    delta: f64,
    t_mid: f64,
    entry: bool,
    outer: CapLoop,
    holes: Vec<CapLoop>,
}

impl CrossedFace {
    /// The plane's height over the footprint coordinate `b`.
    fn t_at(&self, b: f64) -> f64 {
        self.gamma.mul_add(b, self.delta)
    }
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
    if !(cutter.from_mm.is_finite()
        && cutter.to_mm.is_finite()
        && cutter.axis.0.is_finite()
        && cutter.axis.1.is_finite())
        || points.iter().any(|p| !(p.x.is_finite() && p.y.is_finite()))
    {
        return Err(refuse(OpCode::NonFinite, OpDefect::None, Stage::Finiteness));
    }
    if [cutter.from_mm, cutter.to_mm, cutter.axis.0, cutter.axis.1]
        .iter()
        .chain(points.iter().flat_map(|p| [&p.x, &p.y]))
        .any(|v| v.abs() > MAX_COORDINATE_MM)
    {
        return Err(refuse(OpCode::AboveLimit, OpDefect::None, Stage::Caps));
    }
    if m < 3 {
        return Err(refuse(OpCode::ContourInvalid, OpDefect::None, Stage::Caps));
    }
    if cutter.from_mm >= cutter.to_mm {
        return Err(refuse(
            OpCode::NotPositive,
            OpDefect::CutterRange,
            Stage::Ranges,
        ));
    }
    // The hole axis, normalized; a direction whose norm overflows or underflows away is as
    // degenerate as the zero direction and sweeps no prism.
    let norm = (cutter.axis.0 * cutter.axis.0 + cutter.axis.1 * cutter.axis.1).sqrt();
    if !(norm.is_finite() && norm > 0.0) {
        return Err(refuse(OpCode::ContourInvalid, OpDefect::None, Stage::Caps));
    }
    let (dy, dz) = (cutter.axis.0 / norm, cutter.axis.1 / norm);
    // The prism frame `(e1, e2, d)` with `e1 = +X` and `e2 = d x e1 = (0, dz, -dy)`: a body
    // point maps to the footprint coordinate pair `(a, b) = (x, p . e2)` and a height
    // `t = p . d`. For `d = +Z` each expression is the identity on `(x, y, z)`, so every
    // G14.1 result is reproduced bit for bit.
    let project = |p: [f64; 3]| [p[0], p[1] * dz - p[2] * dy];
    let height = |p: [f64; 3]| p[1] * dy + p[2] * dz;
    let to_body = |a: f64, b: f64, t: f64| [a, b * dz + t * dy, -b * dy + t * dz];
    let (zf, zt) = (cutter.from_mm, cutter.to_mm);
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

    let eps = body.tolerance().eps_geom_mm();
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
    // The curved, faceted or sloped wall faces inside the cutter range: served by the
    // strip registry when they form multi-facet chains, refused otherwise.
    let mut faceted: Vec<u16> = Vec::new();
    let (p_min_y, p_max_y) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.y), hi.max(p.y))
        });
    // A wall of an earlier perpendicular hole: clear when its bounding box clears the
    // footprint by the floor in `a` or `b`, or its `t` extent clears the range. The box is
    // conservative in the footprint's favour and never accepts a contact.
    for triangles in solid.drilled.values() {
        let (a_lo, a_hi, b_lo, b_hi, t_lo, t_hi) = triangles.iter().fold(
            (
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ),
            |(alo, ahi, blo, bhi, tlo, thi), tri| {
                tri.iter().fold(
                    (alo, ahi, blo, bhi, tlo, thi),
                    |(alo, ahi, blo, bhi, tlo, thi), i| match solid.position(*i) {
                        Some(p) => (
                            alo.min(p[0]),
                            ahi.max(p[0]),
                            blo.min(project(p)[1]),
                            bhi.max(project(p)[1]),
                            tlo.min(height(p)),
                            thi.max(height(p)),
                        ),
                        None => (f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN),
                    },
                )
            },
        );
        if !(a_lo.is_finite() && t_hi.is_finite()) {
            return Err(internal());
        }
        let boxes_clear = (p_min_x - a_hi).max(a_lo - p_max_x) >= eps
            || (p_min_y - b_hi).max(b_lo - p_max_y) >= eps;
        if boxes_clear || t_clear(t_lo, t_hi, zf, zt, eps) {
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
        // Extents of the face in the frame (a, b, t) and in raw y/z, with the two vertices
        // of extreme `b` anchoring the plane's height line over the footprint.
        let mut finite = true;
        let (mut a_lo, mut a_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut b_lo, mut b_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut t_lo, mut t_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut y_lo, mut y_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut z_lo, mut z_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut anchor_lo: Option<(f64, [f64; 3])> = None;
        let mut anchor_hi: Option<(f64, [f64; 3])> = None;
        for tri in triangles {
            for &i in tri {
                let Some(p) = solid.position(i) else {
                    finite = false;
                    continue;
                };
                let b = project(p)[1];
                let t = height(p);
                a_lo = a_lo.min(p[0]);
                a_hi = a_hi.max(p[0]);
                b_lo = b_lo.min(b);
                b_hi = b_hi.max(b);
                t_lo = t_lo.min(t);
                t_hi = t_hi.max(t);
                y_lo = y_lo.min(p[1]);
                y_hi = y_hi.max(p[1]);
                z_lo = z_lo.min(p[2]);
                z_hi = z_hi.max(p[2]);
                match &anchor_lo {
                    Some((b0, _)) if *b0 <= b => {}
                    _ => anchor_lo = Some((b, p)),
                }
                match &anchor_hi {
                    Some((b0, _)) if *b0 >= b => {}
                    _ => anchor_hi = Some((b, p)),
                }
            }
        }
        if !finite
            || !(a_lo.is_finite()
                && t_hi.is_finite()
                && y_lo.is_finite()
                && z_hi.is_finite()
                && b_lo.is_finite()
                && b_hi.is_finite())
        {
            return Err(internal());
        }
        // A face whose `b` extent is degenerate lies in a plane that contains the cutter
        // axis: its projected segment must clear the footprint, or its own `t` extent must
        // clear the range. The normal gate keeps a face marked curved off this path.
        let flat_b = b_lo == b_hi
            && normal.is_some_and(|n| {
                n[0] == 0.0 && ((n[1] == dz && n[2] == -dy) || (n[1] == -dz && n[2] == dy))
            });
        if flat_b {
            if !clear_of_segment(points, a_lo, a_hi, b_lo, eps, &mut budget)?
                && !t_clear(t_lo, t_hi, zf, zt, eps)
            {
                return Err(refuse(
                    OpCode::ContourInvalid,
                    OpDefect::None,
                    Stage::Ranges,
                ));
            }
            continue;
        }
        let flat_z = z_lo == z_hi;
        let flat_y =
            y_lo == y_hi && normal.is_some_and(|n| n[0] == 0.0 && n[2] == 0.0 && n[1].abs() == 1.0);
        if !flat_z && !flat_y {
            // A curved, faceted or sloped wall face: clear only when the whole face lies
            // beyond the range by at least the floor. A face inside the range goes to the
            // strip registry: faceted walls (a tube) are served by the chains of this
            // slice, sloped single-plane walls stay refused there.
            if t_hi >= zf - eps && t_lo <= zt + eps {
                faceted.push(*face);
            }
            continue;
        }
        // One flat axis-aligned plane, crossed at any angle: its height over the footprint
        // is the line `t = gamma * b + delta` anchored at the face's extreme-b vertices, so
        // the interval over the footprint ends at the footprint's own b extremes. For
        // `d = +Z` this collapses to the G14.1 constant face height.
        let (Some((b_first, p_first)), Some((b_last, p_last))) = (&anchor_lo, &anchor_hi) else {
            return Err(internal());
        };
        let gamma = (height(*p_last) - height(*p_first)) / (b_last - b_first);
        let delta = height(*p_first) - gamma * b_first;
        if !(gamma.is_finite() && delta.is_finite()) {
            return Err(refuse(
                OpCode::ContourInvalid,
                OpDefect::None,
                Stage::Ranges,
            ));
        }
        let t_at = |b: f64| gamma.mul_add(b, delta);
        let (t_at_lo, t_at_hi) = (t_at(p_min_y), t_at(p_max_y));
        let (t_face_lo, t_face_hi) = (t_at_lo.min(t_at_hi), t_at_lo.max(t_at_hi));
        let beyond = if t_face_hi < zf {
            Some(zf - t_face_hi)
        } else if t_face_lo > zt {
            Some(t_face_lo - zt)
        } else if t_face_lo < zf || t_face_hi > zt {
            // The opening would be clipped by a range end: the hole ends inside the wall.
            return Err(refuse(
                OpCode::OpUnsupported,
                OpDefect::CutterNotThrough,
                Stage::Ranges,
            ));
        } else {
            None
        };
        let gap = beyond.unwrap_or_else(|| (t_face_lo - zf).min(zt - t_face_hi));
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
        if beyond.is_some() {
            continue;
        }
        let entry = normal.is_some_and(|n| n[1] * dy + n[2] * dz < 0.0);
        let (outer, holes) = face_loops(body, triangles, entry, project, limits, &mut budget)?;
        match footprint_placement(points, &outer, &holes, project, eps, &mut budget)? {
            Placement::Beyond => {}
            Placement::Within => crossed.push(CrossedFace {
                face: *face,
                gamma,
                delta,
                t_mid: (t_face_lo + t_face_hi) / 2.0,
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
                    xyz: points
                        .iter()
                        .map(|p| to_body(p.x, p.y, t_at(p.y)))
                        .collect(),
                };
                let mut all_holes = holes.clone();
                all_holes.push(ring);
                let refusal = triangulate_cap(
                    &outer,
                    &all_holes,
                    project,
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
    }
    // A cutter through faceted walls: the strip registry carves the chains. A mix of
    // crossed flat faces and crossed chains is not served in this slice.
    if !faceted.is_empty() {
        if !crossed.is_empty() {
            return Err(OpFailure::unsupported(
                OpField::Body,
                OpDefect::BodyWallUnsupported,
                Stage::Ranges,
            ));
        }
        return cut_through_chains(
            body,
            &solid,
            points,
            dy,
            dz,
            zf,
            zt,
            cutter,
            eps,
            &project,
            &to_body,
            limits,
            &mut budget,
            &mut cancelled,
        );
    }
    crossed.sort_by(|a, b| {
        a.t_mid
            .partial_cmp(&b.t_mid)
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
    crossed
        .chunks(2)
        .map(|pair| match pair {
            [in_face, out_face]
                if in_face.entry && !out_face.entry && out_face.t_mid > in_face.t_mid =>
            {
                Some(())
            }
            _ => None,
        })
        .collect::<Option<Vec<()>>>()
        .ok_or_else(internal)?;
    let count = i64::try_from(crossed.len() / 2).map_err(|_| internal())?;

    poll(&mut cancelled, Stage::Checks)?;

    // New vertices: the footprint on the plane of every crossed face, in body coordinates.
    let base = u32::try_from(body.mesh().vertices().len()).map_err(|_| internal())?;
    let m32 = u32::try_from(m).map_err(|_| internal())?;
    let mut vertices: Vec<[f64; 3]> = body.mesh().vertices().to_vec();
    let mut rings: Vec<CapLoop> = Vec::new();
    for (index, crossed_face) in crossed.iter().enumerate() {
        let first = base + u32::try_from(index).map_err(|_| internal())? * m32;
        let xyz: Vec<[f64; 3]> = points
            .iter()
            .map(|p| to_body(p.x, p.y, crossed_face.t_at(p.y)))
            .collect();
        vertices.extend(xyz.iter().copied());
        rings.push(CapLoop {
            ids: (0..m32).map(|k| first + k).collect(),
            xyz,
        });
    }

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
            .is_some_and(|n| n[1] * dy + n[2] * dz < 0.0);
        let mut all_holes = crossed_face.holes.clone();
        all_holes.push(ring.clone());
        let carved = triangulate_cap(
            &crossed_face.outer,
            &all_holes,
            project,
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

    // The cutter's own wall between the two openings of every crossed wall: the band sides
    // are quads between the entry and exit rings, planar for parallel walls and gently
    // non-planar for a pair of differently leaning walls.
    let mut face_ids = body.face_table().to_vec();
    let mut normals = body.face_normals().to_vec();
    let mut bands: Vec<Vec<[u32; 3]>> = Vec::new();
    for k in 0..usize::try_from(count).map_err(|_| internal())? {
        let entry_ring = rings.get(2 * k).ok_or_else(internal)?;
        let exit_ring = rings.get(2 * k + 1).ok_or_else(internal)?;
        let mut band = Vec::with_capacity(2 * m);
        for j in 0..m {
            let l = (j + 1) % m;
            let (p, q) = (
                points.get(j).copied().ok_or_else(internal)?,
                points.get(l).copied().ok_or_else(internal)?,
            );
            face_ids.push(format!("cut:f.h{}.w{k}.s{j}", cutter.ordinal));
            let (da, db) = (q.x - p.x, q.y - p.y);
            let length = (da * da + db * db).sqrt();
            if !(length > 0.0 && length.is_finite()) {
                return Err(internal());
            }
            // The in-frame side normal (db, -da) mapped back to body coordinates.
            let n = [db / length, -da * dz / length, da * dy / length];
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
            band.push([bin, bin1, bout1]);
            band.push([bin, bout1, bout]);
        }
        bands.push(band);
    }

    let removed = removed_volume(&cutter.cavity, &vertices, &rings, &bands)?;
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
            genus: shell
                .genus
                .and_then(|g| u32::try_from(count).ok().and_then(|w| g.checked_add(w))),
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

/// The boundary loops of one wall face in the projected `(a, b)` plane: the single outer
/// loop (counter-clockwise, reversed from the face winding when its normal faces the
/// cutter start) and its holes (clockwise).
fn face_loops(
    body: &Body,
    triangles: &[[u32; 3]],
    reverse: bool,
    project: impl Fn([f64; 3]) -> [f64; 2],
    limits: &OpLimits,
    budget: &mut ExactBudget,
) -> Result<(CapLoop, Vec<CapLoop>), OpFailure> {
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
        project,
        OpDefect::BodyLoops,
        OpField::Body,
        budget,
    )
}

/// The footprint's placement against one wall face's loops in the projected `(a, b)` plane.
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
    project: impl Fn([f64; 3]) -> [f64; 2],
    eps: f64,
    budget: &mut ExactBudget,
) -> Result<Placement, OpFailure> {
    let numeric = || OpFailure::numeric(OpField::Cutter, OpDefect::NumericRange, Stage::Build);
    let flat_ring = |ring: &CapLoop| -> Vec<Point2> {
        ring.xyz
            .iter()
            .map(|p| {
                let [x, y] = project(*p);
                Point2::new(x, y)
            })
            .collect()
    };
    let rings = std::iter::once(outer).chain(holes.iter());
    let mut clear = true;
    for ring in rings.clone() {
        for (a, b) in ring.xyz.iter().zip(ring.xyz.iter().cycle().skip(1)) {
            let (a, b) = (flat_point(*a, &project), flat_point(*b, &project));
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
            match winding_contains(flat_point(*p, &project), points) {
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

/// One point of a loop in the projected `(a, b)` plane.
fn flat_point(p: [f64; 3], project: &impl Fn([f64; 3]) -> [f64; 2]) -> Point2 {
    let [x, y] = project(p);
    Point2::new(x, y)
}

/// A strip of one wall face: the triangles of the face lying on one exact section-line
/// plane, with the plane carried as the outward unit normal `(ny, nz)` and the offset `c`.
/// A face re-triangulated by an earlier hole contributes one strip again (the exact
/// collinearity filter re-merges the plane), and a face whose triangles lie on several
/// planes (a curve-approximated cavity wall) contributes one strip per plane.
struct Strip {
    face: u16,
    ny: f64,
    nz: f64,
    c: f64,
}

impl Strip {
    /// `m = n . d`: the strip is parallel to the cutter axis exactly when this is zero,
    /// which the placement refuses before any height is read.
    fn axis_dot(&self, dy: f64, dz: f64) -> f64 {
        self.ny.mul_add(dy, self.nz * dz)
    }

    /// The plane's height line over the footprint: solving `n . p = c` and `b = p . e2`
    /// for the section point at `b` gives `t = p . d = gamma*b + delta` with
    /// `gamma = (dy*nz - dz*ny)/(n . d)` and `delta = c/(n . d)`.
    fn gamma_delta(&self, dy: f64, dz: f64) -> (f64, f64) {
        let m = self.axis_dot(dy, dz);
        let w = dy * self.nz - dz * self.ny;
        (w / m, self.c / m)
    }
}

/// One chain: strips joined by soft kinks (turns of at most `pi/4` between the strip
/// planes), one smooth authoring surface. `soft` lists the kink edges of the chain's
/// strips as pairs of strip indices (sorted by strip index) with the mesh vertex pair of
/// the edge; every other strip boundary is a hard edge or a chain border.
struct Chain {
    strips: Vec<usize>,
    soft: Vec<(usize, usize)>,
}

/// The shared-kink registry: one body point per contour crossing of a mesh edge, seen by
/// both faces of the edge as the identical vertex. The point lies exactly on the edge
/// (its `(y, z)` is the edge's line, its `a` coordinate the contour crossing position).
struct KinkPoints {
    points: Vec<([f64; 3], u32)>,
}

impl KinkPoints {
    /// The registered point at `at`, creating it on the first visit: the vertex lands in
    /// `vertices` under the returned id, so both faces of the edge share one mesh vertex.
    fn point(
        &mut self,
        at: [f64; 3],
        vertices: &mut Vec<[f64; 3]>,
        next_vertex: &mut u32,
    ) -> Result<u32, OpFailure> {
        if let Some((_, id)) = self
            .points
            .iter()
            .find(|(p, _)| p[0] == at[0] && p[1] == at[1] && p[2] == at[2])
        {
            return Ok(*id);
        }
        let id = *next_vertex;
        *next_vertex = next_vertex.checked_add(1).ok_or_else(|| {
            OpFailure::numeric(OpField::Mesh, OpDefect::NumericRange, Stage::Build)
        })?;
        self.points.push((at, id));
        vertices.push(at);
        Ok(id)
    }
}

/// Whether the turn between two strip planes is a soft kink (at most `pi/4`, the steepest
/// turn the curve approximation emits) or a hard edge.
fn is_soft_kink(n1: (f64, f64), n2: (f64, f64)) -> bool {
    let dot = (n1.0 * n2.0 + n1.1 * n2.1).clamp(-1.0, 1.0);
    dot >= std::f64::consts::FRAC_1_SQRT_2
}

/// Groups the triangles of one wall face into strips by their exact section-line plane:
/// two triangles share a strip when their YZ projections are exactly collinear (the
/// `area_sign` filter), so a face re-triangulated by an earlier hole is re-merged. Returns
/// the strips and, for every triangle, the local index of its strip. The plane normal comes
/// from the triangles' winding and faces away from the material.
fn face_strips(
    body: &Body,
    triangles: &[[u32; 3]],
    face: u16,
    budget: &mut ExactBudget,
) -> Result<(Vec<Strip>, Vec<usize>), OpFailure> {
    let internal = || OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Build);
    let numeric = || OpFailure::numeric(OpField::Body, OpDefect::NumericRange, Stage::Build);
    let vertex = |i: u32| -> Result<[f64; 3], OpFailure> {
        body.mesh()
            .vertices()
            .get(usize::try_from(i).ok().ok_or_else(internal)?)
            .copied()
            .ok_or_else(internal)
    };
    let yz = |p: [f64; 3]| [p[1], p[2]];
    // Groups of triangle indices whose YZ projections are exactly collinear; the first two
    // vertices of a group's first triangle define the line the others must reproduce.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut assignment = vec![0usize; triangles.len()];
    for (index, tri) in triangles.iter().enumerate() {
        let points = [
            yz(vertex(tri[0])?),
            yz(vertex(tri[1])?),
            yz(vertex(tri[2])?),
        ];
        let mut placed = false;
        'groups: for (local, group) in groups.iter_mut().enumerate() {
            let first = triangles
                .get(*group.first().ok_or_else(internal)?)
                .ok_or_else(internal)?;
            // The collinearity base: two YZ-distinct vertices of the group's first
            // triangle (a wall triangle may repeat a YZ point on its two axial corners).
            let first_yz = [
                yz(vertex(*first.first().ok_or_else(internal)?)?),
                yz(vertex(*first.get(1).ok_or_else(internal)?)?),
                yz(vertex(*first.get(2).ok_or_else(internal)?)?),
            ];
            let g1 = first_yz[0];
            let mut g2 = None;
            for candidate in first_yz.iter().skip(1) {
                if candidate[0] != g1[0] || candidate[1] != g1[1] {
                    g2 = Some(*candidate);
                    break;
                }
            }
            let Some(g2) = g2 else {
                return Err(internal());
            };
            for p in &points {
                if area_sign(&[g1, g2, *p], budget)? != 0 {
                    continue 'groups;
                }
            }
            group.push(index);
            *assignment.get_mut(index).ok_or_else(internal)? = local;
            placed = true;
            break;
        }
        if !placed {
            groups.push(vec![index]);
            let local = groups.len() - 1;
            *assignment.get_mut(index).ok_or_else(internal)? = local;
        }
    }
    let mut strips = Vec::with_capacity(groups.len());
    for group in &groups {
        // The two farthest-apart vertices of the group define the line numerically; the
        // triangles' 3-D winding decides which normal direction faces away from the
        // material (the X part of a wall triangle's normal is zero up to rounding, so its
        // YZ part is the section-line normal).
        let (mut p1, mut p2, mut best) = ([0.0; 2], [0.0; 2], -1.0f64);
        for &tri_index in group {
            let tri = triangles.get(tri_index).ok_or_else(internal)?;
            for (first, second) in tri.iter().zip(tri.iter().cycle().skip(1)) {
                let (u, v) = (yz(vertex(*first)?), yz(vertex(*second)?));
                let (d_y, d_z) = (v[0] - u[0], v[1] - u[1]);
                let span = d_y.mul_add(d_y, d_z * d_z);
                if span > best {
                    best = span;
                    p1 = u;
                    p2 = v;
                }
            }
        }
        if !(best > 0.0 && best.is_finite()) {
            return Err(internal());
        }
        let (d_y, d_z) = (p2[0] - p1[0], p2[1] - p1[1]);
        let length = (d_y * d_y + d_z * d_z).sqrt();
        if !(length > 0.0 && length.is_finite()) {
            return Err(internal());
        }
        let (ny_pos, nz_pos) = (-d_z / length, d_y / length);
        let mut winding = 0.0f64;
        for &tri_index in group {
            let tri = triangles.get(tri_index).ok_or_else(internal)?;
            let (u, v, w) = (vertex(tri[0])?, vertex(tri[1])?, vertex(tri[2])?);
            let (e1, e2) = (
                [v[0] - u[0], v[1] - u[1], v[2] - u[2]],
                [w[0] - u[0], w[1] - u[1], w[2] - u[2]],
            );
            let (n_y, n_z) = (
                e1[2].mul_add(e2[0], -e1[0] * e2[2]),
                e1[0].mul_add(e2[1], -e1[1] * e2[0]),
            );
            winding += ny_pos.mul_add(n_y, nz_pos * n_z);
        }
        if !winding.is_finite() || winding == 0.0 {
            return Err(internal());
        }
        let (ny, nz) = if winding > 0.0 {
            (ny_pos, nz_pos)
        } else {
            (-ny_pos, -nz_pos)
        };
        // The offset from the group's vertices: every vertex lies on the exact line, so the
        // mean keeps the rounding of `c` small without picking a favourite vertex.
        let mut c_sum = 0.0f64;
        let mut count = 0u32;
        for &tri_index in group {
            let tri = triangles.get(tri_index).ok_or_else(internal)?;
            for &i in tri {
                let p = yz(vertex(i)?);
                c_sum += ny.mul_add(p[0], nz * p[1]);
                count = count.checked_add(1).ok_or_else(numeric)?;
            }
        }
        let c = c_sum / f64::from(count);
        if !c.is_finite() {
            return Err(numeric());
        }
        strips.push(Strip { face, ny, nz, c });
    }
    Ok((strips, assignment))
}

/// Where a clip-piece vertex comes from, so the construction can map it back to the body.
#[derive(Debug, Clone, Copy)]
enum ClipSrc {
    /// A footprint vertex: a new vertex on the crossed strip's plane.
    Footprint(usize),
    /// A body vertex of the strip region (kept, never duplicated).
    Body(u32),
    /// A contour crossing of a kink edge: its `(y, z)` is the edge's line and its `a` the
    /// crossing position; both faces of the edge see the identical registered point.
    Kink([f64; 3]),
}

/// One vertex of a clip piece: the `(a, b)` position and where it came from.
#[derive(Debug, Clone, Copy)]
struct ClipVertex {
    ab: Point2,
    src: ClipSrc,
}

/// The exact left-of-edge side of a point in the `(a, b)` plane.
fn left_of(a: Point2, b: Point2, p: Point2) -> f64 {
    (b.x - a.x).mul_add(p.y - a.y, -(b.y - a.y) * (p.x - a.x))
}

/// The strip registry: the strips, their chains of soft kinks and every strip's triangles.
type StripRegistry = (Vec<Strip>, Vec<Chain>, Vec<Vec<[u32; 3]>>);

/// One strip's `(a, b)` region: the outer loop, the hole loops and the kink lines.
type StripRegionParts = (
    Vec<(Point2, u32)>,
    Vec<Vec<(Point2, u32)>>,
    Vec<(f64, [f64; 2], f64, f64)>,
);

/// Builds the strips of every wall face and chains them by soft kinks across the mesh
/// edges that two wall triangles of different strips share. Every mesh edge whose two
/// triangles lie in different strips is a kink: soft turns (at most `pi/4`) join their
/// strips into one chain, hard ones never do.
fn build_walls(
    body: &Body,
    solid: &Solid,
    budget: &mut ExactBudget,
) -> Result<StripRegistry, OpFailure> {
    let internal = || OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Build);
    let mut strips: Vec<Strip> = Vec::new();
    let mut strips_tris: Vec<Vec<[u32; 3]>> = Vec::new();
    // The global strip index of every wall triangle, keyed by its face and its own three
    // vertices (the same key the kink walk matches mesh edges by).
    let mut tri_strip: std::collections::BTreeMap<(u16, [u32; 3]), usize> =
        std::collections::BTreeMap::new();
    for (face, triangles) in &solid.walls {
        let (face_strips_list, assignment) = face_strips(body, triangles, *face, budget)?;
        let base = strips.len();
        strips.extend(face_strips_list);
        strips_tris.resize_with(strips.len(), Vec::new);
        for (index, tri) in triangles.iter().enumerate() {
            let local = *assignment.get(index).ok_or_else(internal)?;
            let global = base.checked_add(local).ok_or_else(internal)?;
            tri_strip.insert((*face, *tri), global);
            if let Some(bucket) = strips_tris.get_mut(global) {
                bucket.push(*tri);
            }
        }
    }
    // The kink edges: mesh edges shared by two wall triangles of different strips.
    let mut edges: std::collections::BTreeMap<[u32; 2], Vec<usize>> =
        std::collections::BTreeMap::new();
    for ((_, tri), strip) in &tri_strip {
        for (first, second) in tri.iter().zip(tri.iter().cycle().skip(1)) {
            let key = if first <= second {
                [*first, *second]
            } else {
                [*second, *first]
            };
            edges.entry(key).or_default().push(*strip);
        }
    }
    // Union-find over the strips, merged by soft kinks only.
    let mut parent: Vec<usize> = (0..strips.len()).collect();
    fn root(parent: &[usize], mut at: usize) -> usize {
        while let Some(&next) = parent.get(at) {
            if next == at {
                return at;
            }
            at = next;
        }
        at
    }
    let mut soft: Vec<(usize, usize)> = Vec::new();
    for owners in edges.values() {
        let [a, b] = owners.as_slice() else {
            continue;
        };
        if a == b {
            continue;
        }
        let (strip_a, strip_b) = (
            strips.get(*a).ok_or_else(internal)?,
            strips.get(*b).ok_or_else(internal)?,
        );
        if is_soft_kink((strip_a.ny, strip_a.nz), (strip_b.ny, strip_b.nz)) {
            let (ra, rb) = (root(&parent, *a), root(&parent, *b));
            if ra != rb
                && let Some(slot) = parent.get_mut(ra.max(rb))
            {
                *slot = ra.min(rb);
            }
            soft.push((*a.min(b), *a.max(b)));
        }
    }
    // The chains: strips grouped by root, in strip order, with their soft kink edges.
    let mut order: Vec<usize> = (0..strips.len()).collect();
    order.sort_by_key(|&s| (root(&parent, s), s));
    let mut chains: Vec<Chain> = Vec::new();
    let mut chain_of_strip: Vec<usize> = vec![0; strips.len()];
    for group in order.chunk_by(|a, b| root(&parent, *a) == root(&parent, *b)) {
        let id = chains.len();
        for &strip in group {
            if let Some(slot) = chain_of_strip.get_mut(strip) {
                *slot = id;
            }
        }
        chains.push(Chain {
            strips: group.to_vec(),
            soft: Vec::new(),
        });
    }
    for (a, b) in soft {
        let (Some(ca), Some(cb)) = (
            chain_of_strip.get(a).copied(),
            chain_of_strip.get(b).copied(),
        ) else {
            continue;
        };
        if ca == cb
            && let Some(chain) = chains.get_mut(ca)
        {
            chain.soft.push((a, b));
        }
    }
    Ok((strips, chains, strips_tris))
}

/// The `(a, b)` outer region of one strip: the outer loop of its triangles' boundary,
/// counter-clockwise (reversed from the face winding when the strip's normal faces the
/// cutter start), with every vertex tagged by its mesh id. Returns the region, the strip's
/// hole loops (clockwise, from an earlier hole in the same face) and the strip's vertical
/// mesh edges as their `b` line, body `(y, z)` and `x` span.
fn strip_region(
    body: &Body,
    tris: &[[u32; 3]],
    faces_out: bool,
    project: &impl Fn([f64; 3]) -> [f64; 2],
    limits: &OpLimits,
    budget: &mut ExactBudget,
) -> Result<StripRegionParts, OpFailure> {
    let position = |i: u32| {
        body.mesh()
            .vertices()
            .get(usize::try_from(i).ok()?)
            .copied()
    };
    let mut directed: Vec<(u32, u32)> = tris
        .iter()
        .flat_map(|[a, b, c]| [(*a, *b), (*b, *c), (*c, *a)])
        .collect();
    directed.sort_unstable();
    let boundary: Vec<(u32, u32)> = directed
        .iter()
        .filter(|(a, b)| directed.binary_search(&(*b, *a)).is_err())
        .map(|&(a, b)| if faces_out { (b, a) } else { (a, b) })
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
    let (outer, holes) = split_loops(
        positioned,
        project,
        OpDefect::BodyLoops,
        OpField::Body,
        budget,
    )?;
    let tag = |loop_: &CapLoop| -> Vec<(Point2, u32)> {
        loop_
            .ids
            .iter()
            .zip(loop_.xyz.iter())
            .map(|(id, p)| (flat_point(*p, project), *id))
            .collect()
    };
    // The kink lines: the strip's vertical mesh edges, as their `b` line, `(y, z)` and
    // the `x` span the edge covers.
    let mut kink_lines: Vec<(f64, [f64; 2], f64, f64)> = Vec::new();
    let mut seen: std::collections::BTreeSet<[u32; 2]> = std::collections::BTreeSet::new();
    for (a, b) in tris
        .iter()
        .flat_map(|t| t.iter().zip(t.iter().cycle().skip(1)))
    {
        let key = if a <= b { [*a, *b] } else { [*b, *a] };
        if !seen.insert(key) {
            continue;
        }
        let (u, v) = (position(key[0]), position(key[1]));
        let (Some(u), Some(v)) = (u, v) else {
            continue;
        };
        if u[1] == v[1] && u[2] == v[2] {
            let p = flat_point(u, project);
            kink_lines.push((p.y, [u[1], u[2]], u[0].min(v[0]), u[0].max(v[0])));
        }
    }
    kink_lines.sort_by(|x, y| x.0.total_cmp(&y.0));
    Ok((tag(&outer), holes.iter().map(tag).collect(), kink_lines))
}

/// Clips the CCW `subject` polygon against every half-plane left of a CCW `region` edge
/// (Sutherland–Hodgman). Returns the empty clip when the polygons are disjoint. A crossing
/// against a horizontal region edge (a kink line) interpolates through the canonical
/// `t = (line - p.y)/(q.y - p.y)`, so the clip vertices and the band events agree bit for
/// bit on the registered kink points.
fn clip_region(subject: &[Point2], region: &[Point2]) -> Vec<Point2> {
    let mut out: Vec<Point2> = subject.to_vec();
    for (a, b) in region.iter().zip(region.iter().cycle().skip(1)) {
        if out.is_empty() {
            return out;
        }
        let horizontal = a.y == b.y;
        let mut next = Vec::with_capacity(out.len() + 1);
        for (index, p) in out.iter().enumerate() {
            let q = out.get((index + 1) % out.len()).copied().unwrap_or(*p);
            let (sp, sq) = (left_of(*a, *b, *p), left_of(*a, *b, q));
            if sp >= 0.0 {
                next.push(*p);
            }
            if (sp > 0.0) != (sq > 0.0) && sp != 0.0 && sq != 0.0 {
                let t = if horizontal {
                    (a.y - p.y) / (q.y - p.y)
                } else {
                    sp / (sp - sq)
                };
                next.push(Point2::new(p.x + t * (q.x - p.x), p.y + t * (q.y - p.y)));
            }
        }
        out = next;
    }
    out
}

/// One event of the cutter band along the footprint contour: a footprint vertex or a
/// contour crossing of a kink line, ordered along the contour by edge index and parameter.
struct Event {
    edge: usize,
    t: f64,
    ab: Point2,
    /// The kink points the event sits on, one per owning chain when several chains'
    /// edges share the same footprint line (the near and far wall of a section).
    kinks: Vec<[f64; 3]>,
}

/// Serves a cutter whose crossed walls include faceted chains: builds the strip registry,
/// places the chains, carves every crossed face through the shared kink vertices and
/// seals the result. The flat axis-aligned faces of slice 1 are not handled here.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn cut_through_chains(
    body: &Body,
    solid: &Solid,
    points: &[Point2],
    dy: f64,
    dz: f64,
    zf: f64,
    zt: f64,
    cutter: &MemberHoleCutter,
    eps: f64,
    project: &impl Fn([f64; 3]) -> [f64; 2],
    to_body: &impl Fn(f64, f64, f64) -> [f64; 3],
    limits: &OpLimits,
    budget: &mut ExactBudget,
    cancelled: &mut impl FnMut(Stage) -> bool,
) -> Result<OpResult, OpFailure> {
    let internal = || OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Build);
    poll(cancelled, Stage::Build)?;
    let (strips, chains, strips_tris) = build_walls(body, solid, budget)?;
    let crossed = place_chains(
        body,
        &chains,
        &strips,
        &strips_tris,
        points,
        dy,
        dz,
        zf,
        zt,
        eps,
        project,
        limits,
        budget,
    )?;
    if crossed.is_empty() {
        return Err(OpFailure::new(
            OpCode::OpNoEffect,
            OpField::Cutter,
            OpDefect::None,
            Stage::Build,
        ));
    }
    // Whole chains only: they alternate entering and leaving material, start with an
    // entry and end with an exit.
    let ends_blind = match (crossed.first(), crossed.last()) {
        (Some(first), Some(last)) => !crossed.len().is_multiple_of(2) || !first.entry || last.entry,
        _ => true,
    };
    if ends_blind {
        return Err(OpFailure::new(
            OpCode::OpUnsupported,
            OpField::Body,
            OpDefect::CutterNotThrough,
            Stage::Ranges,
        ));
    }
    crossed
        .chunks(2)
        .map(|pair| match pair {
            [in_chain, out_chain]
                if in_chain.entry && !out_chain.entry && out_chain.t_mid > in_chain.t_mid =>
            {
                Some(())
            }
            _ => None,
        })
        .collect::<Option<Vec<()>>>()
        .ok_or_else(internal)?;
    let count = i64::try_from(crossed.len() / 2).map_err(|_| internal())?;
    poll(cancelled, Stage::Checks)?;

    // The band events: the footprint vertices plus the contour crossings of every kink
    // line of the crossed chains, ordered along the contour. A line shared by two chains'
    // bands (the near and the far wall of a section) contributes one crossing per chain,
    // merged into one event that carries both kink points.
    let mut events: Vec<Event> = points
        .iter()
        .enumerate()
        .map(|(i, p)| Event {
            edge: i,
            t: 0.0,
            ab: *p,
            kinks: Vec::new(),
        })
        .collect();
    for chain in &crossed {
        for piece in &chain.pieces {
            for (line_b, line_yz, x_lo, x_hi) in &piece.kinks {
                for (j, pair) in points.iter().zip(points.iter().cycle().skip(1)).enumerate() {
                    let (p, q) = pair;
                    let (d_b, at) = (q.y - p.y, p.y - line_b);
                    if d_b == 0.0 || (at >= 0.0) == (q.y - line_b < 0.0) {
                        continue;
                    }
                    // The canonical crossing parameter, identical to the clip's own
                    // interpolation against a kink line, so both sides compute the exact
                    // same registered point.
                    let t = (line_b - p.y) / d_b;
                    if !(0.0..=1.0).contains(&t) {
                        continue;
                    }
                    let a = p.x + t * (q.x - p.x);
                    if !(x_lo - eps..=x_hi + eps).contains(&a) {
                        continue;
                    }
                    events.push(Event {
                        edge: j,
                        t,
                        ab: Point2::new(a, *line_b),
                        kinks: vec![[a, line_yz[0], line_yz[1]]],
                    });
                }
            }
        }
    }
    // A footprint vertex sitting on a kink line is the same registered point as the edge
    // crossing there, for every chain owning that line.
    for event in &mut events {
        for chain in &crossed {
            for piece in &chain.pieces {
                for (line_b, line_yz, x_lo, x_hi) in &piece.kinks {
                    if (event.ab.y - line_b).abs() <= eps
                        && (*x_lo - eps..=*x_hi + eps).contains(&event.ab.x)
                    {
                        let kink = [event.ab.x, line_yz[0], line_yz[1]];
                        if !event
                            .kinks
                            .iter()
                            .any(|k| k[0] == kink[0] && k[1] == kink[1] && k[2] == kink[2])
                        {
                            event.kinks.push(kink);
                        }
                    }
                }
            }
        }
    }
    events.sort_by(|a, b| a.edge.cmp(&b.edge).then(a.t.total_cmp(&b.t)));
    // Events of the same contour point merge their kink points: chains whose bands share
    // the footprint line produce identical crossings with their own body edges.
    let mut unique: Vec<Event> = Vec::with_capacity(events.len());
    for event in events {
        match unique.last_mut() {
            Some(last)
                if last.edge == event.edge
                    && (last.t - event.t).abs() <= 1e-12
                    && (last.ab.x - event.ab.x).abs() <= eps =>
            {
                for kink in event.kinks {
                    if !last
                        .kinks
                        .iter()
                        .any(|k| k[0] == kink[0] && k[1] == kink[1] && k[2] == kink[2])
                    {
                        last.kinks.push(kink);
                    }
                }
            }
            _ => unique.push(event),
        }
    }
    let events = unique;
    if events.len() < 3 {
        return Err(internal());
    }

    // New vertices: one ring per crossed chain, on that chain's surface, with the kink
    // points registered once and shared.
    let base = u32::try_from(body.mesh().vertices().len()).map_err(|_| internal())?;
    let mut vertices: Vec<[f64; 3]> = body.mesh().vertices().to_vec();
    let mut next_vertex = base;
    let mut kinks = KinkPoints { points: Vec::new() };
    let mut rings: Vec<Vec<(u32, [f64; 3])>> = Vec::with_capacity(crossed.len());
    for chain in &crossed {
        let mut ring: Vec<(u32, [f64; 3])> = Vec::with_capacity(events.len());
        for event in &events {
            // The chain's own kink point: the one whose body edge belongs to this chain
            // (two chains' bands may share the footprint line).
            let own_kink = event.kinks.iter().copied().find(|k| {
                chain.pieces.iter().any(|p| {
                    strips.get(p.strip).is_some_and(|strip| {
                        let (ny, nz, c) = (strip.ny, strip.nz, strip.c);
                        (ny.mul_add(k[1], nz * k[2]) - c).abs() <= eps
                    })
                })
            });
            let (id, xyz) = match own_kink {
                Some(at) => {
                    let id = kinks.point(at, &mut vertices, &mut next_vertex)?;
                    (id, at)
                }
                None => {
                    let piece = chain
                        .pieces
                        .iter()
                        .find(|p| {
                            let region_pts: Vec<Point2> =
                                p.region.iter().map(|(rp, _)| *rp).collect();
                            winding_contains(event.ab, &region_pts).unwrap_or(false)
                        })
                        .ok_or_else(internal)?;
                    let xyz = to_body(
                        event.ab.x,
                        event.ab.y,
                        piece.gamma.mul_add(event.ab.y, piece.delta),
                    );
                    let id = next_vertex;
                    next_vertex = next_vertex.checked_add(1).ok_or_else(internal)?;
                    vertices.push(xyz);
                    (id, xyz)
                }
            };
            ring.push((id, xyz));
        }
        rings.push(ring);
    }
    let ring_id = |ring: &[(u32, [f64; 3])], event_index: usize| -> Result<u32, OpFailure> {
        ring.get(event_index)
            .map(|(id, _)| *id)
            .ok_or_else(internal)
    };

    // Every crossed strip is carved: its outer loop (the boundary of its own triangles)
    // is cut open at the kink crossings and spliced with the clip piece, or a swallowed
    // strip disappears whole. A face with several strips (a curve-approximated cavity
    // wall) is carved strip by strip under the same face id.
    let mut crossed_strips: std::collections::BTreeMap<usize, &ChainPiece> =
        std::collections::BTreeMap::new();
    let mut chain_of_strip: std::collections::BTreeMap<usize, usize> =
        std::collections::BTreeMap::new();
    for (ci, chain) in crossed.iter().enumerate() {
        for piece in &chain.pieces {
            crossed_strips.insert(piece.strip, piece);
            chain_of_strip.insert(piece.strip, ci);
        }
    }
    // A face carved through several of its own strips (a curve-approximated cavity wall,
    // whose band seams never line up with the flat bands) is the merge-walk slice's work.
    let mut crossed_face_strips: std::collections::BTreeMap<u16, usize> =
        std::collections::BTreeMap::new();
    for chain in &crossed {
        for piece in &chain.pieces {
            *crossed_face_strips.entry(piece.face).or_default() += 1;
        }
    }
    if crossed_face_strips.values().any(|count| *count > 1) {
        return Err(OpFailure::unsupported(
            OpField::Body,
            OpDefect::BodyWallUnsupported,
            Stage::Build,
        ));
    }
    // The body keeps every triangle except those of the carved strips' faces.
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut faces: Vec<u16> = Vec::new();
    for (tri, face) in body.mesh().triangles().iter().zip(body.mesh().faces()) {
        if !crossed_face_strips.contains_key(face) {
            triangles.push(*tri);
            faces.push(*face);
        }
    }
    let mut loops = 0usize;
    let mut cap_points = 0usize;
    'strip: for (strip_index, piece_ref) in &crossed_strips {
        let piece: &ChainPiece = piece_ref;
        let face = piece.face;
        let Some(chain_index) = chain_of_strip.get(strip_index).copied() else {
            return Err(internal());
        };
        let chain_ring = rings.get(chain_index).ok_or_else(internal)?;
        let Some(strip) = strips.get(*strip_index) else {
            return Err(internal());
        };
        let Some(tris) = strips_tris.get(*strip_index) else {
            return Err(internal());
        };
        let reverse = strip.axis_dot(dy, dz) < 0.0;
        let (mut outer, mut holes) = face_loops(body, tris, reverse, project, limits, budget)?;
        // A strip whose every contour vertex lies inside its own clip piece disappears
        // whole: the opening swallows it.
        let outer_inside = !outer.ids.is_empty()
            && outer.xyz.iter().all(|p| {
                let fp = flat_point(*p, project);
                winding_contains(fp, &piece.clip).unwrap_or(false)
            });
        if outer_inside {
            continue;
        }
        // Map every clip vertex of this strip to a mesh vertex id.
        let mut clip_ids: Vec<Vec<u32>> = Vec::with_capacity(1);
        {
            let mut ids: Vec<u32> = Vec::with_capacity(piece.tagged.len());
            for tagged in &piece.tagged {
                let id = match tagged.src {
                    ClipSrc::Body(id) => id,
                    ClipSrc::Kink(at) => kinks.point(at, &mut vertices, &mut next_vertex)?,
                    ClipSrc::Footprint(index) => {
                        let mut found = None;
                        for (ei, e) in events.iter().enumerate() {
                            if e.edge == index
                                && e.t == 0.0
                                && e.ab.x == tagged.ab.x
                                && e.ab.y == tagged.ab.y
                            {
                                found = Some(ei);
                                break;
                            }
                        }
                        ring_id(chain_ring, found.ok_or_else(internal)?)?
                    }
                };
                ids.push(id);
            }
            clip_ids.push(ids);
        }
        // Splice the clip piece into the strip's outer loop or add it as one more hole.
        // A crossed strip carries exactly one clip piece, so the splice runs once.
        {
            let ids = clip_ids.first().ok_or_else(internal)?;
            // The boundary contact points of the clip, in clip order: the same physical
            // point (a clip loop that starts and ends on one boundary edge) is one contact.
            let mut contacts: Vec<usize> = Vec::new();
            for (i, tagged) in piece.tagged.iter().enumerate() {
                if !matches!(tagged.src, ClipSrc::Footprint(_)) {
                    let duplicate = contacts.iter().any(|&c| {
                        let Some(prev) = piece.tagged.get(c) else {
                            return false;
                        };
                        (prev.ab.x - tagged.ab.x).abs() <= eps
                            && (prev.ab.y - tagged.ab.y).abs() <= eps
                    });
                    if !duplicate {
                        contacts.push(i);
                    }
                }
            }
            if contacts.is_empty() {
                // A closed opening: one more clockwise hole; the strip's own loop (never
                // touched) is re-triangulated with it right here.
                holes.push(CapLoop {
                    ids: ids.clone(),
                    xyz: piece
                        .clip
                        .iter()
                        .map(|p| to_body(p.x, p.y, piece.gamma.mul_add(p.y, piece.delta)))
                        .collect(),
                });
                let carved = triangulate_cap(
                    &outer,
                    &holes,
                    project,
                    limits,
                    body.tolerance(),
                    OpField::Cap,
                    false,
                )
                .map_err(|e| cutter_field(e, holes.len()))?;
                loops += 1 + holes.len();
                cap_points += outer.ids.len() + holes.iter().map(|h| h.ids.len()).sum::<usize>();
                for [a, b, c] in carved {
                    triangles.push(if reverse { [a, c, b] } else { [a, b, c] });
                    faces.push(face);
                }
                continue 'strip;
            }
            // The generalized splice: every contact lands in the outer loop; then each
            // pair of consecutive contacts in clip order either keeps the outer path
            // between them (it runs outside the opening) or has that path thrown and the
            // clip's own arc spliced in. An opening touching both boundary edges of a
            // narrow band cuts it in two; the assembly below yields both pieces.
            let at_of = |i: usize| -> Result<[f64; 3], OpFailure> {
                match piece.tagged.get(i).map(|t| t.src) {
                    Some(ClipSrc::Kink(at)) => Ok(at),
                    Some(ClipSrc::Body(id)) => body
                        .mesh()
                        .vertices()
                        .get(usize::try_from(id).ok().ok_or_else(internal)?)
                        .copied()
                        .ok_or_else(internal),
                    _ => Err(internal()),
                }
            };
            // Insert a contact id into the outer loop at the vertical edge it lies on.
            let insert = |outer: &mut CapLoop, id: u32, at: [f64; 3]| -> Result<(), OpFailure> {
                if outer.ids.contains(&id) {
                    return Ok(());
                }
                let n = outer.xyz.len();
                let mut slot = None;
                for (k, pair) in outer
                    .xyz
                    .iter()
                    .zip(outer.xyz.iter().cycle().skip(1))
                    .enumerate()
                {
                    let (before, after) = pair;
                    if before[1] == at[1]
                        && before[2] == at[2]
                        && after[1] == at[1]
                        && after[2] == at[2]
                        && at[0] >= before[0].min(after[0])
                        && at[0] <= before[0].max(after[0])
                    {
                        slot = Some((k + 1) % n);
                        break;
                    }
                }
                let Some(next) = slot else {
                    return Err(internal());
                };
                outer.ids.insert(next, id);
                outer.xyz.insert(next, at);
                Ok(())
            };
            for &ci in &contacts {
                let id = *ids.get(ci).ok_or_else(internal)?;
                insert(&mut outer, id, at_of(ci)?)?;
            }
            let clip_pts: Vec<Point2> = piece.tagged.iter().map(|t| t.ab).collect();
            let clip_border_dist = |p: Point2| -> f64 {
                clip_pts
                    .iter()
                    .zip(clip_pts.iter().cycle().skip(1))
                    .map(|(r, s)| segment_distance(p, p, *r, *s))
                    .fold(f64::INFINITY, f64::min)
            };
            // The outer path between two contacts is kept when it runs outside the
            // opening (its first edge's midpoint is off the clip and off its boundary)
            // and thrown when it runs through the opening.
            let throws = |path: &[usize]| -> Result<bool, OpFailure> {
                // The first edge decides: the kept paths run along the strip's own
                // contour, so the opening sits on one side of them all.
                let (Some(&ia), Some(&ib)) = (path.first(), path.get(1)) else {
                    return Ok(false);
                };
                let (Some(pa), Some(pb)) = (outer.xyz.get(ia).copied(), outer.xyz.get(ib).copied())
                else {
                    return Err(internal());
                };
                let mid3 = [
                    (pa[0] + pb[0]) / 2.0,
                    (pa[1] + pb[1]) / 2.0,
                    (pa[2] + pb[2]) / 2.0,
                ];
                let mid_ab = flat_point(mid3, &project);
                Ok(winding_contains(mid_ab, &clip_pts).unwrap_or(false)
                    || clip_border_dist(mid_ab) <= eps)
            };
            let n = outer.ids.len();
            // The contacts in outer order (rotated from the smallest slot).
            let mut ring: Vec<(usize, usize)> = Vec::with_capacity(contacts.len());
            for &ci in &contacts {
                let id = *ids.get(ci).ok_or_else(internal)?;
                let Some(at) = outer.ids.iter().position(|&i| i == id) else {
                    return Err(internal());
                };
                ring.push((ci, at));
            }
            let base = ring.iter().map(|(_, at)| *at).min().ok_or_else(internal)?;
            ring.sort_by_key(|(_, at)| (*at + n - base) % n);
            let n_clip = contacts.len();
            // The kept outer paths: one per contact when the opening cuts the strip (a
            // touch on two boundary edges leaves two pieces), or the single round trip
            // when it only notches one edge.
            let mut keeps: Vec<(usize, usize, Vec<usize>)> = Vec::new();
            for i in 0..n_clip {
                let (ci, ai) = *ring.get(i).ok_or_else(internal)?;
                let (cj, aj) = *ring.get((i + 1) % n_clip).ok_or_else(internal)?;
                let mut path = vec![ai];
                let mut at = (ai + 1) % n;
                while at != aj {
                    path.push(at);
                    at = (at + 1) % n;
                }
                path.push(aj);
                if !throws(&path)? {
                    keeps.push((ci, cj, path));
                }
            }
            // The opening's arcs: the clip slots between consecutive contacts, in clip
            // order; a straight chord between neighbours contributes none.
            let m = piece.tagged.len();
            let mut arcs: Vec<(usize, usize, Vec<usize>)> = Vec::new();
            for i in 0..n_clip {
                let (ci, cj) = (
                    *contacts.get(i).ok_or_else(internal)?,
                    *contacts.get((i + 1) % n_clip).ok_or_else(internal)?,
                );
                let mut slots = Vec::new();
                let mut at = (ci + 1) % m;
                while at != cj {
                    slots.push(at);
                    at = (at + 1) % m;
                }
                if !slots.is_empty() {
                    arcs.push((ci, cj, slots));
                }
            }
            // Every contact must carry exactly one kept path and one arc; otherwise the
            // opening's relation to the strip's boundary is not a servable cut.
            if keeps.len() != arcs.len() || keeps.is_empty() {
                return Err(OpFailure::unsupported(
                    OpField::Body,
                    OpDefect::BodyWallUnsupported,
                    Stage::Build,
                ));
            }
            // Assemble the pieces: alternate a kept path with the opening's arc at every
            // contact. A notch yields one piece; a cut through a band yields two.
            let clip_at = |i: usize| -> Result<([f64; 3], u32), OpFailure> {
                let ab = piece.tagged.get(i).map(|t| t.ab).ok_or_else(internal)?;
                let id = *ids.get(i).ok_or_else(internal)?;
                let xyz = to_body(ab.x, ab.y, piece.gamma.mul_add(ab.y, piece.delta));
                Ok((xyz, id))
            };
            let mut used_keep = vec![false; keeps.len()];
            let mut used_arc = vec![false; arcs.len()];
            let mut pieces_out: Vec<CapLoop> = Vec::new();
            for start in 0..keeps.len() {
                if *used_keep.get(start).ok_or_else(internal)? {
                    continue;
                }
                *used_keep.get_mut(start).ok_or_else(internal)? = true;
                let mut piece_loop = CapLoop {
                    ids: Vec::new(),
                    xyz: Vec::new(),
                };
                // Walk the first kept path in its own (outer, counter-clockwise) order.
                let (_, mut end, path) = keeps.get(start).ok_or_else(internal)?.clone();
                for slot in &path {
                    let (Some(id), Some(xyz)) =
                        (outer.ids.get(*slot).copied(), outer.xyz.get(*slot).copied())
                    else {
                        return Err(internal());
                    };
                    piece_loop.ids.push(id);
                    piece_loop.xyz.push(xyz);
                }
                loop {
                    // The arc at the path's end contact, walked away from it.
                    let Some(ai) = (0..arcs.len()).find(|&i| {
                        !used_arc.get(i).copied().unwrap_or(false)
                            && arcs.get(i).is_some_and(|arc| arc.0 == end || arc.1 == end)
                    }) else {
                        return Err(internal());
                    };
                    *used_arc.get_mut(ai).ok_or_else(internal)? = true;
                    let (a, b, slots) = arcs.get(ai).ok_or_else(internal)?;
                    if *a == end {
                        for slot in slots {
                            let (xyz, id) = clip_at(*slot)?;
                            piece_loop.ids.push(id);
                            piece_loop.xyz.push(xyz);
                        }
                        end = *b;
                    } else {
                        for slot in slots.iter().rev() {
                            let (xyz, id) = clip_at(*slot)?;
                            piece_loop.ids.push(id);
                            piece_loop.xyz.push(xyz);
                        }
                        end = *a;
                    }
                    // The kept path at the arc's far end, if one is left there.
                    let Some(ki) = (0..keeps.len()).find(|&i| {
                        !used_keep.get(i).copied().unwrap_or(false)
                            && keeps
                                .get(i)
                                .is_some_and(|keep| keep.0 == end || keep.1 == end)
                    }) else {
                        break;
                    };
                    *used_keep.get_mut(ki).ok_or_else(internal)? = true;
                    let (a, b, keep_slots) = keeps.get(ki).ok_or_else(internal)?;
                    if *a == end {
                        for slot in keep_slots.iter().skip(1) {
                            let (Some(id), Some(xyz)) =
                                (outer.ids.get(*slot).copied(), outer.xyz.get(*slot).copied())
                            else {
                                return Err(internal());
                            };
                            piece_loop.ids.push(id);
                            piece_loop.xyz.push(xyz);
                        }
                        end = *b;
                    } else {
                        for slot in keep_slots.iter().rev().skip(1) {
                            let (Some(id), Some(xyz)) =
                                (outer.ids.get(*slot).copied(), outer.xyz.get(*slot).copied())
                            else {
                                return Err(internal());
                            };
                            piece_loop.ids.push(id);
                            piece_loop.xyz.push(xyz);
                        }
                        end = *a;
                    }
                }
                // The piece must be a counter-clockwise simple polygon.
                let flat: Vec<[f64; 2]> = piece_loop.xyz.iter().map(|p| project(*p)).collect();
                if area_sign(&flat, budget)? < 0 {
                    piece_loop.ids.reverse();
                    piece_loop.xyz.reverse();
                }
                pieces_out.push(piece_loop);
            }
            // Triangulate every piece under the strip's own face id.
            for piece_loop in pieces_out {
                let carved = triangulate_cap(
                    &piece_loop,
                    &holes,
                    project,
                    limits,
                    body.tolerance(),
                    OpField::Cap,
                    false,
                )
                .map_err(|e| cutter_field(e, holes.len()))?;
                loops += 1 + holes.len();
                cap_points +=
                    piece_loop.ids.len() + holes.iter().map(|h| h.ids.len()).sum::<usize>();
                for [a, b, c] in carved {
                    triangles.push(if reverse { [a, c, b] } else { [a, b, c] });
                    faces.push(face);
                }
            }
            continue 'strip;
        }
    }

    // The cutter's own wall: one band per crossed chain pair, walked along the events.
    let mut face_ids = body.face_table().to_vec();
    let mut normals = body.face_normals().to_vec();
    let mut bands: Vec<Vec<[u32; 3]>> = Vec::new();
    for k in 0..usize::try_from(count).map_err(|_| internal())? {
        let entry_ring = rings.get(2 * k).ok_or_else(internal)?;
        let exit_ring = rings.get(2 * k + 1).ok_or_else(internal)?;
        let mut band = Vec::with_capacity(2 * events.len());
        // The emitted run count of each footprint edge, for the band id suffixes.
        let mut runs: std::collections::BTreeMap<usize, u32> = std::collections::BTreeMap::new();
        // The face id of each band side: the footprint edge it walks, with a run suffix
        // after the first planar run of the side.
        for s in 0..events.len() {
            let s1 = (s + 1) % events.len();
            let (e0, e1) = (
                events.get(s).ok_or_else(internal)?,
                events.get(s1).ok_or_else(internal)?,
            );
            let j = e0.edge;
            // The runs of an edge number from the bare first run at the edge's own
            // vertex; a run past a mid-edge kink crossing carries `.p<run>` (the G14.1
            // spelling keeps `s{j}` bare for the single-run case). Two crossings on one
            // edge must not share a suffix: every band side is its own face id.
            let run_index = runs.entry(j).or_insert(0);
            let run = if *run_index == 0 {
                String::new()
            } else {
                format!(".p{run_index}")
            };
            *run_index += 1;
            face_ids.push(format!("cut:f.h{}.w{k}.s{j}{run}", cutter.ordinal));
            // The in-frame side normal (db, -da) of the event edge, mapped back to body
            // coordinates as in the slice-1 bands.
            let (da, db) = (e1.ab.x - e0.ab.x, e1.ab.y - e0.ab.y);
            let length = (da * da + db * db).sqrt();
            if !(length > 0.0 && length.is_finite()) {
                return Err(internal());
            }
            let n = [db / length, -da * dz / length, da * dy / length];
            normals.push(Some(n.map(|v| if v == 0.0 { 0.0 } else { v })));
            let in_id = ring_id(entry_ring, s)?;
            let in_id1 = ring_id(entry_ring, s1)?;
            let out_id = ring_id(exit_ring, s)?;
            let out_id1 = ring_id(exit_ring, s1)?;
            triangles.push([in_id, in_id1, out_id1]);
            triangles.push([in_id, out_id1, out_id]);
            faces.push(u16::try_from(face_ids.len() - 1).map_err(|_| internal())?);
            faces.push(u16::try_from(face_ids.len() - 1).map_err(|_| internal())?);
            band.push([in_id, in_id1, out_id1]);
            band.push([in_id, out_id1, out_id]);
        }
        bands.push(band);
    }
    let rings_cap: Vec<CapLoop> = rings
        .iter()
        .map(|ring| CapLoop {
            ids: ring.iter().map(|(id, _)| *id).collect(),
            xyz: ring.iter().map(|(_, xyz)| *xyz).collect(),
        })
        .collect();
    let removed = removed_volume(&cutter.cavity, &vertices, &rings_cap, &bands)?;
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
        new_vertices: vertices
            .len()
            .saturating_sub(usize::try_from(base).map_err(|_| internal())?),
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
            genus: shell
                .genus
                .and_then(|g| u32::try_from(count).ok().and_then(|w| g.checked_add(w))),
            analytic_volume: Some(analytic),
            plane_deviation_mm: None,
            stats,
        },
        cancelled,
    )
}

fn big_area2(poly: &[Point2], e: i32) -> Result<Big, OpFailure> {
    let range = || OpFailure::numeric(OpField::Cutter, OpDefect::NumericRange, Stage::Ranges);
    let scale = |x: f64| -> Result<Big, OpFailure> { scaled(x, e).map_err(|_| range()) };
    let mut sum = Big::ZERO;
    for (p, q) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        let (px, py, qx, qy) = (scale(p.x), scale(p.y), scale(q.x), scale(q.y));
        let (px, py, qx, qy) = match (px, py, qx, qy) {
            (Ok(a), Ok(b), Ok(c), Ok(d)) => (a, b, c, d),
            _ => return Err(range()),
        };
        let left = px.mul(&py);
        let right = qx.mul(&qy);
        let (left, right) = match (left, right) {
            (Ok(a), Ok(b)) => (a, b),
            _ => return Err(range()),
        };
        match left.sub(&right) {
            Ok(term) => match sum.add(&term) {
                Ok(next) => sum = next,
                _ => return Err(range()),
            },
            _ => return Err(range()),
        }
    }
    if sum.sign() < 0 {
        sum = sum.negated();
    }
    Ok(sum)
}

/// One crossed chain's placement, resolved: the strips to carve, the clip pieces and the
/// chain's height interval over the footprint.
struct CrossedChain {
    /// The strips of the chain, with their clip pieces.
    pieces: Vec<ChainPiece>,
    /// The height interval midpoint (the crossing order key).
    t_mid: f64,
    /// Whether the chain's outer-facing side looks at the cutter range start.
    entry: bool,
}

/// One crossed strip of a chain: its clip piece and its plane's height line over `b`.
struct ChainPiece {
    strip: usize,
    face: u16,
    gamma: f64,
    delta: f64,
    clip: Vec<Point2>,
    /// The tagged clip vertices in order, for the construction's vertex mapping.
    tagged: Vec<ClipVertex>,
    /// The strip's `(a, b)` region (outer loop, tagged) and kink lines.
    region: Vec<(Point2, u32)>,
    /// The strip's vertical mesh edges: `b` line, body `(y, z)`, `x` span.
    kinks: Vec<(f64, [f64; 2], f64, f64)>,
}

/// Classifies every chain against the cutter range with the union-aware placement:
/// a chain the footprint covers entirely (the exact clip areas sum to the footprint area)
/// and whose height interval lies inside the range is crossed; a chain the footprint
/// clears is beyond; a partial cover or a hard-edge contact refuses.
#[allow(clippy::too_many_arguments)]
fn place_chains(
    body: &Body,
    chains: &[Chain],
    strips: &[Strip],
    strips_tris: &[Vec<[u32; 3]>],
    points: &[Point2],
    dy: f64,
    dz: f64,
    zf: f64,
    zt: f64,
    eps: f64,
    project: &impl Fn([f64; 3]) -> [f64; 2],
    limits: &OpLimits,
    budget: &mut ExactBudget,
) -> Result<Vec<CrossedChain>, OpFailure> {
    let internal = || OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Build);
    let refused =
        |defect| OpFailure::new(OpCode::OpUnsupported, OpField::Body, defect, Stage::Ranges);
    let contour = || {
        OpFailure::new(
            OpCode::ContourInvalid,
            OpField::Cutter,
            OpDefect::None,
            Stage::Ranges,
        )
    };
    // The footprint's `b` extremes.
    let (b_lo, b_hi) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.y), hi.max(p.y))
        });
    let mut crossed: Vec<CrossedChain> = Vec::new();
    for chain in chains {
        // A strip parallel to the cutter axis cannot carry the opening's height: its
        // section segment is parallel to `d` and the G14.1 breach applies.
        if chain.strips.iter().any(|&s| {
            strips
                .get(s)
                .is_some_and(|strip| strip.axis_dot(dy, dz).abs() <= f64::EPSILON * 64.0)
        }) {
            return Err(contour());
        }
        // The strips' regions and the coarse height skip over the footprint's `b` extremes.
        let mut pieces: Vec<ChainPiece> = Vec::new();
        let (mut t_c_lo, mut t_c_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &strip_index in &chain.strips {
            let strip = strips.get(strip_index).ok_or_else(internal)?;
            let tris = strips_tris.get(strip_index).ok_or_else(internal)?;
            let faces_out = strip.axis_dot(dy, dz) < 0.0;
            let (region, _holes, kinks) =
                strip_region(body, tris, faces_out, project, limits, budget)?;
            let (gamma, delta) = strip.gamma_delta(dy, dz);
            let t_at = |b: f64| gamma.mul_add(b, delta);
            t_c_lo = t_c_lo.min(t_at(b_lo).min(t_at(b_hi)));
            t_c_hi = t_c_hi.max(t_at(b_lo).max(t_at(b_hi)));
            pieces.push(ChainPiece {
                strip: strip_index,
                face: strip.face,
                gamma,
                delta,
                clip: Vec::new(),
                tagged: Vec::new(),
                region,
                kinks,
            });
        }
        if !t_c_lo.is_finite() || !t_c_hi.is_finite() {
            return Err(internal());
        }
        if t_clear(t_c_lo, t_c_hi, zf, zt, eps) {
            continue;
        }
        // A chain of one strip is a single sloped plane: the flat-wall slice keeps its
        // refusal, and the merge-walk slice takes it.
        if chain.strips.len() < 2 {
            return Err(OpFailure::unsupported(
                OpField::Body,
                OpDefect::BodyWallUnsupported,
                Stage::Ranges,
            ));
        }
        // The clip pieces first: the exact areas below need the scale over the clip
        // interpolations, whose coordinates may carry deeper mantissas than the body's
        // own vertices.
        let mut perim = 0.0f64;
        for piece in &mut pieces {
            let region_pts: Vec<Point2> = piece.region.iter().map(|(p, _)| *p).collect();
            let clip = clip_region(points, &region_pts);
            if clip.is_empty() {
                continue;
            }
            perim += clip
                .iter()
                .zip(clip.iter().cycle().skip(1))
                .map(|(p, q)| {
                    let (d_x, d_y) = (q.x - p.x, q.y - p.y);
                    d_x.hypot(d_y)
                })
                .sum::<f64>();
            piece.clip = clip;
        }
        let any = pieces.iter().any(|p| !p.clip.is_empty());
        if !any {
            continue;
        }
        let e = min_exponent(
            points.iter().flat_map(|p| [p.x, p.y]).chain(
                pieces
                    .iter()
                    .flat_map(|p| p.clip.iter().flat_map(|c| [c.x, c.y])),
            ),
        );
        let mut covered2 = Big::ZERO;
        for piece in &mut pieces {
            if piece.clip.is_empty() {
                continue;
            }
            let area2 = big_area2(&piece.clip, e)?;
            covered2 = covered2.add(&area2).map_err(|_| internal())?;
        }
        let footprint2 = big_area2(points, e)?;
        let scale = 2.0f64.powi(e);
        let bound = 32.0
            * f64::EPSILON
            * scale
            * perim
            * f64::from(u32::try_from(points.len() * 2).map_err(|_| internal())?);
        let diff = footprint2.sub(&covered2).map_err(|_| internal())?;
        let diff_enc = diff
            .enclose_clamped(e.checked_sub(1).ok_or_else(internal)?)
            .ok_or_else(internal)?;
        if diff_enc.lo > bound || diff_enc.hi < -bound {
            // The footprint is not exactly covered: free space or a sub-floor reach past
            // the union, refused as the G14.1 edge-touching footprint.
            if covered2.sign() == 0 {
                continue;
            }
            return Err(contour());
        }
        // The clip vertices: tagged by source, and the floor kept from the region's hard
        // edges (kink lines may be touched).
        let mut t_lo = f64::INFINITY;
        let mut t_hi = f64::NEG_INFINITY;
        for piece in &mut pieces {
            let region_pts: Vec<Point2> = piece.region.iter().map(|(p, _)| *p).collect();
            let tag = |p: &Point2| -> Result<ClipVertex, OpFailure> {
                // A kink line first: a footprint vertex that sits on the line (up to the
                // geometric floor) is the same registered point as the contour crossing
                // there, so the clip never carries a duplicate a few ulps away.
                for (line_b, line_yz, x_lo, x_hi) in &piece.kinks {
                    if (p.y - line_b).abs() <= eps && (*x_lo - eps..=*x_hi + eps).contains(&p.x) {
                        return Ok(ClipVertex {
                            ab: Point2::new(p.x, *line_b),
                            src: ClipSrc::Kink([p.x, line_yz[0], line_yz[1]]),
                        });
                    }
                }
                if let Some((index, fp)) = points
                    .iter()
                    .enumerate()
                    .find(|(_, fp)| fp.x == p.x && fp.y == p.y)
                {
                    return Ok(ClipVertex {
                        ab: *fp,
                        src: ClipSrc::Footprint(index),
                    });
                }
                if let Some((_, id)) = piece
                    .region
                    .iter()
                    .find(|(rp, _)| rp.x == p.x && rp.y == p.y)
                {
                    return Ok(ClipVertex {
                        ab: *p,
                        src: ClipSrc::Body(*id),
                    });
                }
                // An interpolated vertex: exactly one kink line it may sit on; touching a
                // hard edge instead is the G14.1 refusal.
                Err(contour())
            };
            for p in &piece.clip {
                piece.tagged.push(tag(p)?);
            }
            // Vertices registered to the same kink point are one vertex: consecutive
            // duplicates and duplicates across the loop's closure collapse to the first.
            let mut deduped: Vec<ClipVertex> = Vec::with_capacity(piece.tagged.len());
            for vertex in piece.tagged.drain(..) {
                let duplicate = deduped.iter().any(|prev| {
                    (prev.ab.x - vertex.ab.x).abs() <= eps && (prev.ab.y - vertex.ab.y).abs() <= eps
                });
                if !duplicate {
                    deduped.push(vertex);
                }
            }
            piece.tagged = deduped;
            let t_at = |b: f64| piece.gamma.mul_add(b, piece.delta);
            for p in &piece.clip {
                let t = t_at(p.y);
                t_lo = t_lo.min(t);
                t_hi = t_hi.max(t);
            }
            // The floor: every clip edge clears the region's hard edges; the region's own
            // holes and the kink lines are exempt from the hole half only.
            for (p, q) in piece.clip.iter().zip(piece.clip.iter().cycle().skip(1)) {
                for (rp, rq) in region_pts.iter().zip(region_pts.iter().cycle().skip(1)) {
                    if boxes_apart(*p, *q, *rp, *rq, eps) {
                        continue;
                    }
                    let on_kink = piece.kinks.iter().any(|(line_b, _, _, _)| {
                        (rp.y - line_b).abs() <= eps && (rq.y - line_b).abs() <= eps
                    });
                    if on_kink {
                        continue;
                    }
                    match segment_relation(*p, *q, *rp, *rq) {
                        Ok(SegRel::Disjoint) => {
                            if segment_distance(*p, *q, *rp, *rq) < eps {
                                return Err(contour());
                            }
                        }
                        Ok(_) => return Err(contour()),
                        Err(_) => {
                            budget.charge()?;
                            return Err(contour());
                        }
                    }
                }
            }
        }
        // The crossed pieces grouped into sides: clips on strips adjacent through the
        // chain's own soft-kink order form one crossing; separated zones of a ring wall
        // (a tube's near and far wall) are distinct sides and cross the range as an entry
        // and an exit of their own.
        let mut crossed_pieces: Vec<ChainPiece> =
            pieces.into_iter().filter(|p| !p.clip.is_empty()).collect();
        // The chain's strip order, walking the soft-kink adjacency end to end (a ring
        // wall walks its full circle, an open chain walks from its first strip).
        let mut adjacency: std::collections::BTreeMap<usize, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (a, b) in &chain.soft {
            adjacency.entry(*a).or_default().push(*b);
            adjacency.entry(*b).or_default().push(*a);
        }
        let mut order: Vec<usize> = Vec::with_capacity(chain.strips.len());
        if let Some(&start) = chain.strips.first() {
            let (mut at, mut prev) = (start, None);
            for _ in 0..chain.strips.len() {
                order.push(at);
                let next = adjacency
                    .get(&at)
                    .into_iter()
                    .flatten()
                    .copied()
                    .find(|n| Some(*n) != prev);
                match next {
                    Some(n) => {
                        prev = Some(at);
                        at = n;
                    }
                    None => break,
                }
            }
        }
        let mut hit: Vec<usize> = order
            .iter()
            .filter(|s| crossed_pieces.iter().any(|p| p.strip == **s))
            .copied()
            .collect();
        hit.sort_by_key(|s| order.iter().position(|o| o == s).unwrap_or(usize::MAX));
        let mut sides: Vec<Vec<ChainPiece>> = Vec::new();
        for strip in hit {
            let Some(piece) = crossed_pieces
                .iter()
                .position(|p| p.strip == strip)
                .map(|at| crossed_pieces.remove(at))
            else {
                continue;
            };
            // Adjacent through a soft kink means the same side; a gap in the chain's own
            // strip order starts a new one.
            let continues = sides.last().is_some_and(|side| {
                side.last()
                    .and_then(|p| adjacency.get(&p.strip))
                    .is_some_and(|nexts| nexts.contains(&strip))
            });
            match (continues, sides.last_mut()) {
                (true, Some(side)) => side.push(piece),
                _ => sides.push(vec![piece]),
            }
        }
        for side in sides {
            let mut s_lo = f64::INFINITY;
            let mut s_hi = f64::NEG_INFINITY;
            for piece in &side {
                let t_at = |b: f64| piece.gamma.mul_add(b, piece.delta);
                for p in &piece.clip {
                    let t = t_at(p.y);
                    s_lo = s_lo.min(t);
                    s_hi = s_hi.max(t);
                }
            }
            if !s_lo.is_finite() || !s_hi.is_finite() {
                return Err(internal());
            }
            // The blind, tangent and sub-floor interval refusals of G14.1.
            if s_lo < zf - eps || s_hi > zt + eps {
                return Err(refused(OpDefect::CutterNotThrough));
            }
            let gap = (s_lo - zf).min(zt - s_hi);
            if gap == 0.0 {
                return Err(refused(OpDefect::CutterTangent));
            }
            if gap < eps {
                return Err(refused(OpDefect::CutterMargin));
            }
            // The entry direction: the side's first strip's outward normal.
            let entry = side
                .first()
                .and_then(|p| strips.get(p.strip))
                .is_some_and(|s| s.ny.mul_add(dy, s.nz * dz) < 0.0);
            crossed.push(CrossedChain {
                pieces: side,
                t_mid: (s_lo + s_hi) / 2.0,
                entry,
            });
        }
    }
    crossed.sort_by(|a, b| {
        a.t_mid
            .partial_cmp(&b.t_mid)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(crossed)
}

/// Whether a `t` extent clears the cutter range by at least the floor.
fn t_clear(t_lo: f64, t_hi: f64, zf: f64, zt: f64, eps: f64) -> bool {
    t_hi < zf - eps || t_lo > zt + eps
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

/// Exact removed volume of the constructed openings, enclosed outward. Every opening's
/// boundary — its entry and exit rings fanned from their first vertex, plus its band
/// triangles — has an exact Big-rational divergence sum, so the total certifies the volume
/// of the polyhedron the operation actually emitted; for one wall pair with parallel faces
/// that value is exactly the G14.1 prism `area * thickness`, and for an oblique pair it is
/// the same integral `A * t / |n . d|` in closed form. The rings' vertices sit on their
/// wall planes only up to the rounding of the frame map, so the enclosure is widened by a
/// bound on that residual before it is subtracted from the body volume.
fn removed_volume(
    cavity: &Cavity,
    vertices: &[[f64; 3]],
    rings: &[CapLoop],
    bands: &[Vec<[u32; 3]>],
) -> Result<Enclosure, OpFailure> {
    let range = || OpFailure::numeric(OpField::Volume, OpDefect::NumericRange, Stage::Quantities);
    let broken = || OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Build);
    let points = cavity.points();
    let e = min_exponent(
        rings
            .iter()
            .flat_map(|r| r.xyz.iter().flatten().copied())
            .chain(points.iter().flat_map(|p| [p.x, p.y])),
    );
    let s = |x: f64| scaled(x, e).map_err(|_| range());
    let big_mul = |u: &Big, v: &Big| -> Result<Big, OpFailure> { u.mul(v).map_err(|_| range()) };
    let big_sub = |u: &Big, v: &Big| -> Result<Big, OpFailure> { u.sub(v).map_err(|_| range()) };
    let big_add = |u: &Big, v: &Big| -> Result<Big, OpFailure> { u.add(v).map_err(|_| range()) };
    // Exact `p . (q x r)` in units of `2^(3e)`.
    let det = |p: [f64; 3], q: [f64; 3], r: [f64; 3]| -> Result<Big, OpFailure> {
        let [x0, y0, z0] = p;
        let [x1, y1, z1] = q;
        let [x2, y2, z2] = r;
        let (a0, a1, a2) = (s(x0)?, s(y0)?, s(z0)?);
        let (b0, b1, b2) = (s(x1)?, s(y1)?, s(z1)?);
        let (c0, c1, c2) = (s(x2)?, s(y2)?, s(z2)?);
        big_add(
            &big_mul(&a0, &big_sub(&big_mul(&b1, &c2)?, &big_mul(&b2, &c1)?)?)?,
            &big_add(
                &big_mul(&a1, &big_sub(&big_mul(&b2, &c0)?, &big_mul(&b0, &c2)?)?)?,
                &big_mul(&a2, &big_sub(&big_mul(&b0, &c1)?, &big_mul(&b1, &c0)?)?)?,
            )?,
        )
    };
    // Exact doubled footprint area, as the bound on the rings' rounding residual below.
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
    let area_hi = area2
        .enclose_clamped(
            e.checked_mul(2)
                .and_then(|v| v.checked_sub(1))
                .ok_or_else(range)?,
        )
        .ok_or_else(range)?
        .hi
        .max(0.0);
    let position = |i: u32| -> Option<[f64; 3]> {
        usize::try_from(i)
            .ok()
            .and_then(|i| vertices.get(i))
            .copied()
    };
    let mut total6 = Big::ZERO;
    for (k, band) in bands.iter().enumerate() {
        let entry = rings.get(2 * k).ok_or_else(broken)?;
        let exit = rings.get(2 * k + 1).ok_or_else(broken)?;
        let mut sum6 = Big::ZERO;
        // The two openings as fans: the entry ring is wound against the opening solid's
        // outward orientation (the band triangles fix it), so its fan runs reversed.
        for (reverse, ring) in [true, false].into_iter().zip([entry, exit]) {
            let first = ring.xyz.first().copied().ok_or_else(broken)?;
            for (p, q) in ring.xyz.iter().zip(ring.xyz.iter().cycle().skip(1)) {
                let term = if reverse {
                    det(first, *q, *p)?
                } else {
                    det(first, *p, *q)?
                };
                sum6 = sum6.add(&term).map_err(|_| range())?;
            }
        }
        for tri in band {
            let [i, j, l] = *tri;
            let (p, q, r) = (position(i), position(j), position(l));
            sum6 = sum6
                .add(&det(
                    p.ok_or_else(broken)?,
                    q.ok_or_else(broken)?,
                    r.ok_or_else(broken)?,
                )?)
                .map_err(|_| range())?;
        }
        if sum6.sign() < 0 {
            sum6 = sum6.negated();
        }
        total6 = total6.add(&sum6).map_err(|_| range())?;
    }
    let mag = rings
        .iter()
        .flat_map(|r| r.xyz.iter())
        .fold(0.0f64, |acc, p| {
            let [x, y, z] = *p;
            acc.max(x.abs()).max(y.abs()).max(z.abs())
        });
    let ring_points: usize = rings.iter().map(|r| r.xyz.len()).sum();
    // Each ring vertex is a few rounding steps off its exact wall plane and its exact
    // linear height, so the emitted opening differs from the ideal one by at most one such
    // residual per vertex over the footprint area; widen by a factor that covers the fused
    // operations involved.
    let residual = 8.0
        * f64::EPSILON
        * mag
        * area_hi
        * f64::from(u32::try_from(ring_points).map_err(|_| range())?);
    let v6 = total6
        .enclose_clamped(e.checked_mul(3).ok_or_else(range)?)
        .ok_or_else(range)?;
    let v = div6(v6).ok_or_else(range)?;
    Enclosure::new((v.lo - residual).next_down(), (v.hi + residual).next_up()).ok_or_else(range)
}

/// Divides an enclosure by exactly 6 with outward rounding, mirroring the kernel's own
/// quotient rule: a zero fused residual above the underflow floor proves the quotient exact.
fn div6(v: Enclosure) -> Option<Enclosure> {
    let endpoint = |x: f64, up: bool| -> Option<f64> {
        let q = x / 6.0;
        if !q.is_finite() {
            return None;
        }
        let trusted = x.abs() >= 1e-250 && q.abs() >= 1e-250;
        let exact = trusted && q.mul_add(6.0, -x) == 0.0;
        Some(if exact {
            q
        } else if up {
            q.next_up()
        } else {
            q.next_down()
        })
    };
    Enclosure::new(endpoint(v.lo, false)?, endpoint(v.hi, true)?)
}
