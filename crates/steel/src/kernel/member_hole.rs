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
            // A curved or sloped wall face: clear only when the whole face lies beyond the
            // range by at least the floor; serving it is the strip registry's work (the
            // later G14.2 slices).
            if t_hi >= zf - eps && t_lo <= zt + eps {
                return Err(OpFailure::unsupported(
                    OpField::Body,
                    OpDefect::BodyWallUnsupported,
                    Stage::Ranges,
                ));
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
