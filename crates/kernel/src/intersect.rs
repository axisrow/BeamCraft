// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Exact closed-set contact predicates between triangles, segments and points.
//!
//! Everything here is decided from the signs of exact orientation predicates,
//! never from a tolerance. Two closed triangles intersect exactly when an edge of
//! one meets the other; a segment meets a triangle through the sign pattern of
//! its endpoints against the triangle plane and of three tetrahedra against its
//! edges (coplanar segments reduce to 2D tests in an exactly valid projection).

use crate::exact::{PredError, Predicates, project};
use crate::spatial::Aabb;

pub(crate) type Tri = [[f64; 3]; 3];

fn point_in_triangle_2d(
    p: &mut Predicates,
    t: [[f64; 2]; 3],
    q: [f64; 2],
) -> Result<bool, PredError> {
    let [a, b, c] = t;
    let signs = [
        p.orient2d(a, b, q)?,
        p.orient2d(b, c, q)?,
        p.orient2d(c, a, q)?,
    ];
    Ok(!(signs.contains(&1) && signs.contains(&-1)))
}

fn on_segment_2d(a: [f64; 2], b: [f64; 2], r: [f64; 2]) -> bool {
    let within = |lo: f64, hi: f64, v: f64| lo.min(hi) <= v && v <= lo.max(hi);
    within(a[0], b[0], r[0]) && within(a[1], b[1], r[1])
}

/// Closed 2D segment intersection `pq` versus `ab`.
fn segments_meet_2d(
    p: &mut Predicates,
    pq: ([f64; 2], [f64; 2]),
    ab: ([f64; 2], [f64; 2]),
) -> Result<bool, PredError> {
    let ((s, t), (a, b)) = (pq, ab);
    let o1 = p.orient2d(s, t, a)?;
    let o2 = p.orient2d(s, t, b)?;
    let o3 = p.orient2d(a, b, s)?;
    let o4 = p.orient2d(a, b, t)?;
    if o1 * o2 < 0 && o3 * o4 < 0 {
        return Ok(true);
    }
    Ok((o1 == 0 && on_segment_2d(s, t, a))
        || (o2 == 0 && on_segment_2d(s, t, b))
        || (o3 == 0 && on_segment_2d(a, b, s))
        || (o4 == 0 && on_segment_2d(a, b, t)))
}

/// True when the closed segment `s0 s1` meets the closed non-degenerate triangle.
///
/// A degenerate (exactly collinear) triangle in the coplanar case is
/// `PredError::Range`: callers certify non-degeneracy before contact tests.
pub(crate) fn segment_meets_triangle(
    p: &mut Predicates,
    s0: [f64; 3],
    s1: [f64; 3],
    tri: &Tri,
) -> Result<bool, PredError> {
    let [a, b, c] = *tri;
    let d0 = p.orient3d(a, b, c, s0)?;
    let d1 = p.orient3d(a, b, c, s1)?;
    if d0 != 0 && d0 == d1 {
        return Ok(false);
    }
    if d0 == 0 && d1 == 0 {
        let Some((k, _)) = p.normal_axis(a, b, c)? else {
            return Err(PredError::Range);
        };
        let t2 = [project(a, k), project(b, k), project(c, k)];
        let (q0, q1) = (project(s0, k), project(s1, k));
        if point_in_triangle_2d(p, t2, q0)? || point_in_triangle_2d(p, t2, q1)? {
            return Ok(true);
        }
        let [a2, b2, c2] = t2;
        return Ok(segments_meet_2d(p, (q0, q1), (a2, b2))?
            || segments_meet_2d(p, (q0, q1), (b2, c2))?
            || segments_meet_2d(p, (q0, q1), (c2, a2))?);
    }
    let signs = [
        p.orient3d(s0, s1, a, b)?,
        p.orient3d(s0, s1, b, c)?,
        p.orient3d(s0, s1, c, a)?,
    ];
    Ok(!(signs.contains(&1) && signs.contains(&-1)))
}

/// Closed triangle-triangle intersection of vertex-disjoint triangles.
pub(crate) fn triangles_meet(p: &mut Predicates, ta: &Tri, tb: &Tri) -> Result<bool, PredError> {
    let [a0, a1, a2] = *ta;
    let [b0, b1, b2] = *tb;
    for (s0, s1) in [(a0, a1), (a1, a2), (a2, a0)] {
        if segment_meets_triangle(p, s0, s1, tb)? {
            return Ok(true);
        }
    }
    for (s0, s1) in [(b0, b1), (b1, b2), (b2, b0)] {
        if segment_meets_triangle(p, s0, s1, ta)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// True when the closed segment strictly crosses the triangle: its endpoints
/// lie on strictly opposite sides of the triangle's plane, and the pierce
/// point lies in the closed triangle (the exact closed sign pattern of the
/// three tetrahedra against the segment admits it). A segment that merely
/// touches the plane or the triangle is not a crossing.
pub(crate) fn segment_crosses_triangle(
    p: &mut Predicates,
    s0: [f64; 3],
    s1: [f64; 3],
    tri: &Tri,
) -> Result<bool, PredError> {
    let [a, b, c] = *tri;
    let d0 = p.orient3d(a, b, c, s0)?;
    let d1 = p.orient3d(a, b, c, s1)?;
    if d0 == 0 || d1 == 0 || d0 == d1 {
        return Ok(false);
    }
    let signs = [
        p.orient3d(s0, s1, a, b)?,
        p.orient3d(s0, s1, b, c)?,
        p.orient3d(s0, s1, c, a)?,
    ];
    Ok(!(signs.contains(&1) && signs.contains(&-1)))
}

/// True when two coplanar triangles overlap in area, not only along their
/// boundaries.
///
/// The candidates are exact: a strict 2D crossing of their edges, a vertex of
/// one strictly inside the other's region (either winding), and, for a vertex
/// lying in the other's closed region, the midpoint between it and the
/// midpoint of its own opposite edge — a point strictly inside its own
/// triangle by convexity, which decides the coincident-boundary cases (equal
/// squares, strips sharing an edge) that leave no strict vertex or crossing.
pub(crate) fn coplanar_area_overlap(
    p: &mut Predicates,
    ta: &Tri,
    tb: &Tri,
) -> Result<bool, PredError> {
    let Some((k, _)) = p.normal_axis(ta[0], ta[1], ta[2])? else {
        return Err(PredError::Range);
    };
    let a2: [[f64; 2]; 3] = [project(ta[0], k), project(ta[1], k), project(ta[2], k)];
    let b2: [[f64; 2]; 3] = [project(tb[0], k), project(tb[1], k), project(tb[2], k)];
    if strictly_contains_vertices(p, a2, b2)? || strictly_contains_vertices(p, b2, a2)? {
        return Ok(true);
    }
    for (s0, s1) in [(a2[0], a2[1]), (a2[1], a2[2]), (a2[2], a2[0])] {
        for (q0, q1) in [(b2[0], b2[1]), (b2[1], b2[2]), (b2[2], b2[0])] {
            let (o1, o2) = (p.orient2d(s0, s1, q0)?, p.orient2d(s0, s1, q1)?);
            let (o3, o4) = (p.orient2d(q0, q1, s0)?, p.orient2d(q0, q1, s1)?);
            if o1 * o2 < 0 && o3 * o4 < 0 {
                return Ok(true);
            }
        }
    }
    Ok(midpoint_chain(p, a2, b2)? || midpoint_chain(p, b2, a2)?)
}

/// True when `q` lies strictly inside the `region` triangle, whatever its
/// winding.
fn strictly_inside_region(
    p: &mut Predicates,
    region: [[f64; 2]; 3],
    q: [f64; 2],
) -> Result<bool, PredError> {
    let [a, b, c] = region;
    let signs = [
        p.orient2d(a, b, q)?,
        p.orient2d(b, c, q)?,
        p.orient2d(c, a, q)?,
    ];
    Ok(signs.iter().all(|&s| s != 0) && !(signs.contains(&1) && signs.contains(&-1)))
}

/// True when any vertex of `vertices` lies strictly inside the `region`
/// triangle, whatever its winding.
fn strictly_contains_vertices(
    p: &mut Predicates,
    region: [[f64; 2]; 3],
    vertices: [[f64; 2]; 3],
) -> Result<bool, PredError> {
    for q in vertices {
        if strictly_inside_region(p, region, q)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The midpoint chains: for a vertex of `own` in `other`'s closed region, the
/// midpoint between it and the midpoint of its own opposite edge is strictly
/// inside `own` by convexity; if `other` strictly contains it too, the regions
/// overlap in area. This decides the coincident-boundary cases (equal squares,
/// strips sharing an edge) that leave no strict vertex or crossing.
fn midpoint_chain(
    p: &mut Predicates,
    own: [[f64; 2]; 3],
    other: [[f64; 2]; 3],
) -> Result<bool, PredError> {
    for (i, v) in own.iter().enumerate() {
        if !point_in_triangle_2d(p, other, *v)? {
            continue;
        }
        let opposite = match i {
            0 => midpoint(own[1], own[2]),
            1 => midpoint(own[2], own[0]),
            _ => midpoint(own[0], own[1]),
        };
        let probe = midpoint(*v, opposite);
        if strictly_inside_region(p, other, probe)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The exact midpoint of two points (division by two is exact in binary64).
fn midpoint(u: [f64; 2], w: [f64; 2]) -> [f64; 2] {
    [(u[0] + w[0]) / 2.0, (u[1] + w[1]) / 2.0]
}

/// True when two distinct non-degenerate triangles touch beyond the feature they share.
///
/// `ia`/`ib` are the vertex indices; indices, not coordinates, define what is
/// shared (no welding). Sharing all three vertices is a duplicate: overlap.
/// Sharing an edge is clean unless the triangles are coplanar and fold onto the
/// same side of it; sharing one vertex is clean exactly when neither opposite
/// edge meets the other closed triangle.
pub(crate) fn overlap_beyond_shared(
    p: &mut Predicates,
    ta: &Tri,
    ia: [u32; 3],
    tb: &Tri,
    ib: [u32; 3],
) -> Result<bool, PredError> {
    let mut shared: Vec<(usize, usize)> = Vec::with_capacity(3);
    for (i, va) in ia.iter().enumerate() {
        for (j, vb) in ib.iter().enumerate() {
            if va == vb {
                shared.push((i, j));
            }
        }
    }
    let corner = |t: &Tri, i: usize| t.get(i).copied().ok_or(PredError::Range);
    // The corners of a triangle that are not the picked ones, as positions.
    let rest = |t: &Tri, picked: &[usize]| -> Result<Vec<[f64; 3]>, PredError> {
        (0..3)
            .filter(|i| !picked.contains(i))
            .map(|i| corner(t, i))
            .collect()
    };
    match shared.as_slice() {
        [] => triangles_meet(p, ta, tb),
        [(i, j)] => {
            let (Some(&[a1, a2]), Some(&[b1, b2])) = (
                rest(ta, &[*i])?.as_slice().first_chunk::<2>(),
                rest(tb, &[*j])?.as_slice().first_chunk::<2>(),
            ) else {
                return Err(PredError::Range);
            };
            Ok(segment_meets_triangle(p, a1, a2, tb)? || segment_meets_triangle(p, b1, b2, ta)?)
        }
        [(i1, j1), (i2, j2)] => {
            let (u, w) = (corner(ta, *i1)?, corner(ta, *i2)?);
            let (Some(a), Some(b)) = (
                rest(ta, &[*i1, *i2])?.first().copied(),
                rest(tb, &[*j1, *j2])?.first().copied(),
            ) else {
                return Err(PredError::Range);
            };
            if p.orient3d(u, w, a, b)? != 0 {
                return Ok(false);
            }
            let Some((k, _)) = p.normal_axis(u, w, a)? else {
                return Err(PredError::Range);
            };
            let (pu, pw) = (project(u, k), project(w, k));
            let side_a = p.orient2d(pu, pw, project(a, k))?;
            let side_b = p.orient2d(pu, pw, project(b, k))?;
            if side_b == 0 {
                return Err(PredError::Range);
            }
            Ok(side_a == side_b)
        }
        _ => Ok(true),
    }
}

/// Position of a point relative to one shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Inside,
    Outside,
    OnSurface,
    Unresolved,
}

/// True when the closed triangle contains the point.
pub(crate) fn triangle_contains_point(
    p: &mut Predicates,
    tri: &Tri,
    q: [f64; 3],
) -> Result<bool, PredError> {
    let [a, b, c] = *tri;
    if p.orient3d(a, b, c, q)? != 0 {
        return Ok(false);
    }
    let Some((k, _)) = p.normal_axis(a, b, c)? else {
        return Err(PredError::Range);
    };
    point_in_triangle_2d(
        p,
        [project(a, k), project(b, k), project(c, k)],
        project(q, k),
    )
}

/// Crossing parity of the +/- axis ray from `q` against `triangles`.
///
/// `Ok(None)` when the ray touches an edge or vertex of a triangle (or runs
/// along one), in which case the caller tries another direction.
fn ray_parity(
    p: &mut Predicates,
    triangles: &[Tri],
    q: [f64; 3],
    axis: usize,
    forward: bool,
) -> Result<Option<bool>, PredError> {
    let mut crossings = 0usize;
    let pq = project(q, axis);
    for tri in triangles {
        let [a, b, c] = *tri;
        let (pa, pb, pc) = (project(a, axis), project(b, axis), project(c, axis));
        let signs = [
            p.orient2d(pa, pb, pq)?,
            p.orient2d(pb, pc, pq)?,
            p.orient2d(pc, pa, pq)?,
        ];
        if signs.contains(&1) && signs.contains(&-1) {
            continue;
        }
        let [s1, s2, s3] = signs;
        if s1 == 0 || s2 == 0 || s3 == 0 {
            return Ok(None);
        }
        // Strictly inside the projection: s1 is the sign of the normal's axis component.
        let plane_side = p.orient3d(a, b, c, q)?;
        if plane_side == 0 {
            return Ok(None);
        }
        let ahead_positive = plane_side * s1 > 0;
        if ahead_positive == forward {
            crossings += 1;
        }
    }
    Ok(Some(crossings % 2 == 1))
}

/// Classifies `q` against one closed, consistently oriented shell.
///
/// Surface membership is exact. Otherwise the parity of strict crossings of
/// the first non-degenerate axis ray decides; if all six axis rays graze an
/// edge or vertex the result is `Unresolved`, never a guess.
pub(crate) fn classify_point(
    p: &mut Predicates,
    triangles: &[Tri],
    q: [f64; 3],
) -> Result<Side, PredError> {
    for tri in triangles {
        let inside_box =
            Aabb::from_points(tri.iter().copied()).is_some_and(|b| b.contains_point(q));
        if inside_box && triangle_contains_point(p, tri, q)? {
            return Ok(Side::OnSurface);
        }
    }
    for axis in 0..3 {
        for forward in [true, false] {
            if let Some(odd) = ray_parity(p, triangles, q, axis, forward)? {
                return Ok(if odd { Side::Inside } else { Side::Outside });
            }
        }
    }
    Ok(Side::Unresolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Predicates {
        Predicates::new(u64::MAX)
    }

    const A: Tri = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]];

    #[test]
    fn segment_triangle_cases() {
        let mut pr = p();
        // Crossing the interior, touching an edge, touching a vertex, missing, coplanar overlap.
        assert!(segment_meets_triangle(&mut pr, [1.0, 1.0, -1.0], [1.0, 1.0, 1.0], &A).unwrap());
        assert!(segment_meets_triangle(&mut pr, [2.0, 0.0, -1.0], [2.0, 0.0, 1.0], &A).unwrap());
        assert!(segment_meets_triangle(&mut pr, [0.0, 0.0, 0.0], [-1.0, -1.0, 5.0], &A).unwrap());
        assert!(!segment_meets_triangle(&mut pr, [5.0, 5.0, -1.0], [5.0, 5.0, 1.0], &A).unwrap());
        assert!(!segment_meets_triangle(&mut pr, [1.0, 1.0, 0.5], [1.0, 1.0, 1.0], &A).unwrap());
        assert!(segment_meets_triangle(&mut pr, [1.0, 1.0, 0.0], [9.0, 9.0, 0.0], &A).unwrap());
        assert!(segment_meets_triangle(&mut pr, [-1.0, 2.0, 0.0], [5.0, 2.0, 0.0], &A).unwrap());
        assert!(!segment_meets_triangle(&mut pr, [3.0, 3.0, 0.0], [9.0, 9.0, 0.0], &A).unwrap());
        // Touching exactly at the hypotenuse midpoint from outside the plane.
        assert!(segment_meets_triangle(&mut pr, [2.0, 2.0, 0.0], [2.0, 2.0, 3.0], &A).unwrap());
    }

    #[test]
    fn pair_classification_by_shared_features() {
        let mut pr = p();
        // Disjoint, piercing and touching vertex-disjoint pairs.
        let far: Tri = [[0.0, 0.0, 1.0], [4.0, 0.0, 1.0], [0.0, 4.0, 1.0]];
        assert!(!overlap_beyond_shared(&mut pr, &A, [0, 1, 2], &far, [3, 4, 5]).unwrap());
        let pierce: Tri = [[1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [2.0, 1.0, 1.0]];
        assert!(overlap_beyond_shared(&mut pr, &A, [0, 1, 2], &pierce, [3, 4, 5]).unwrap());
        let touch: Tri = [[1.0, 1.0, 0.0], [1.0, 1.0, 1.0], [2.0, 1.0, 1.0]];
        assert!(overlap_beyond_shared(&mut pr, &A, [0, 1, 2], &touch, [3, 4, 5]).unwrap());
        // Shared edge (0,1): hinged is clean; folded back onto the same side overlaps;
        // continuing the plane on the opposite side is clean.
        let hinge: Tri = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, -2.0, 3.0]];
        assert!(!overlap_beyond_shared(&mut pr, &A, [0, 1, 2], &hinge, [0, 1, 3]).unwrap());
        let fold: Tri = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [1.0, 3.0, 0.0]];
        assert!(overlap_beyond_shared(&mut pr, &A, [0, 1, 2], &fold, [0, 1, 3]).unwrap());
        let flat: Tri = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [1.0, -3.0, 0.0]];
        assert!(!overlap_beyond_shared(&mut pr, &A, [0, 1, 2], &flat, [0, 1, 3]).unwrap());
        // Shared vertex only: apart, and crossing through the opposite edge.
        let apart: Tri = [[0.0, 0.0, 0.0], [-3.0, 0.0, 1.0], [-3.0, 1.0, 1.0]];
        assert!(!overlap_beyond_shared(&mut pr, &A, [0, 1, 2], &apart, [0, 3, 4]).unwrap());
        let cross: Tri = [[0.0, 0.0, 0.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0]];
        assert!(overlap_beyond_shared(&mut pr, &A, [0, 1, 2], &cross, [0, 3, 4]).unwrap());
        // Same vertex set twice.
        assert!(overlap_beyond_shared(&mut pr, &A, [0, 1, 2], &A, [2, 1, 0]).unwrap());
    }

    fn cube_triangles() -> Vec<Tri> {
        let v = |i: usize| [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64];
        [
            [0, 4, 6],
            [0, 6, 2],
            [1, 3, 7],
            [1, 7, 5],
            [0, 1, 5],
            [0, 5, 4],
            [2, 6, 7],
            [2, 7, 3],
            [0, 2, 3],
            [0, 3, 1],
            [4, 5, 7],
            [4, 7, 6],
        ]
        .iter()
        .map(|t| t.map(v))
        .collect()
    }

    #[test]
    fn point_classification_is_exact_on_surface_and_robust_off_it() {
        let mut pr = p();
        let cube = cube_triangles();
        assert_eq!(
            classify_point(&mut pr, &cube, [0.3, 0.2, 0.7]).unwrap(),
            Side::Inside
        );
        assert_eq!(
            classify_point(&mut pr, &cube, [1.3, 0.2, 0.7]).unwrap(),
            Side::Outside
        );
        assert_eq!(
            classify_point(&mut pr, &cube, [0.3, -2.0, 0.7]).unwrap(),
            Side::Outside
        );
        assert_eq!(
            classify_point(&mut pr, &cube, [1.0, 0.5, 0.25]).unwrap(),
            Side::OnSurface
        );
        assert_eq!(
            classify_point(&mut pr, &cube, [0.0, 0.0, 0.0]).unwrap(),
            Side::OnSurface
        );
        // The exact centre lies on every face diagonal in all six axis directions.
        assert_eq!(
            classify_point(&mut pr, &cube, [0.5, 0.5, 0.5]).unwrap(),
            Side::Unresolved
        );
    }
}
