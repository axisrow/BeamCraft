// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Boundary loops of a clipped or capped surface and the cap triangulation.
//!
//! The cap polygon is admitted and triangulated by the G05 profile code (exact orientation,
//! simplicity and containment signs, hole bridging, ear clipping); this module only chains
//! boundary edges into loops, orients and canonicalises them, and maps G05's indices back to
//! mesh vertices.

use super::diagnostic::{OpCode, OpDefect, OpFailure, OpField};
use super::{ExactBudget, OpLimits};
use crate::bigint::Big;
use crate::diagnostic::Stage;
use crate::exact::{min_exponent, scaled};
use crate::profile::{Cavity, Point2, Profile};
use beamcraft_geom::tolerance::Tolerance;
use core::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// One boundary loop: mesh vertex ids and their positions.
#[derive(Debug, Clone)]
pub struct CapLoop {
    /// Local vertex ids around the loop.
    pub ids: Vec<u32>,
    /// Vertex coordinates around the loop.
    pub xyz: Vec<[f64; 3]>,
}

/// Why a set of directed boundary edges is not a set of simple loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainError {
    /// A vertex has no matching in/out edge: the surface does not close.
    Open,
    /// A vertex is entered or left more than once: the boundary touches itself.
    Pinched,
    /// More loops than the budget allows.
    TooManyLoops,
}

/// Chains directed edges into closed loops (each vertex exactly one out and one in edge).
pub fn chain_loops(edges: &[(u32, u32)], max_loops: usize) -> Result<Vec<Vec<u32>>, ChainError> {
    let mut next: BTreeMap<u32, u32> = BTreeMap::new();
    let mut ends: BTreeSet<u32> = BTreeSet::new();
    for &(a, b) in edges {
        if a == b || next.insert(a, b).is_some() || !ends.insert(b) {
            return Err(ChainError::Pinched);
        }
    }
    if !next.keys().eq(ends.iter()) {
        return Err(ChainError::Open);
    }
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut loops: Vec<Vec<u32>> = Vec::new();
    for &start in next.keys() {
        if seen.contains(&start) {
            continue;
        }
        if loops.len() >= max_loops {
            return Err(ChainError::TooManyLoops);
        }
        let mut ring = vec![start];
        seen.insert(start);
        let mut at = start;
        loop {
            let to = *next.get(&at).ok_or(ChainError::Open)?;
            if to == start {
                break;
            }
            if !seen.insert(to) || ring.len() > edges.len() {
                return Err(ChainError::Pinched);
            }
            ring.push(to);
            at = to;
        }
        if ring.len() < 3 {
            return Err(ChainError::Pinched);
        }
        loops.push(ring);
    }
    Ok(loops)
}

fn cmp_point(a: &[f64; 3], b: &[f64; 3]) -> Ordering {
    a[0].total_cmp(&b[0])
        .then(a[1].total_cmp(&b[1]))
        .then(a[2].total_cmp(&b[2]))
}

/// Builds a loop that starts at its lexicographically smallest vertex, so the cap
/// triangulation never depends on how the input mesh happened to be numbered.
pub fn canonical_loop(ring: &[u32], position: impl Fn(u32) -> Option<[f64; 3]>) -> Option<CapLoop> {
    let points: Vec<[f64; 3]> = ring.iter().map(|&i| position(i)).collect::<Option<_>>()?;
    let first = (0..points.len()).min_by(|&i, &j| match (points.get(i), points.get(j)) {
        (Some(a), Some(b)) => cmp_point(a, b).then(ring.get(i).cmp(&ring.get(j))),
        _ => Ordering::Equal,
    })?;
    let n = points.len();
    let mut ids = Vec::with_capacity(n);
    let mut xyz = Vec::with_capacity(n);
    for k in 0..n {
        let at = (first + k) % n;
        ids.push(*ring.get(at)?);
        xyz.push(*points.get(at)?);
    }
    Some(CapLoop { ids, xyz })
}

/// Sorts loops by their first (smallest) vertex.
pub(crate) fn sort_loops(loops: &mut [CapLoop]) {
    loops.sort_by(|a, b| match (a.xyz.first(), b.xyz.first()) {
        (Some(p), Some(q)) => cmp_point(p, q),
        _ => Ordering::Equal,
    });
}

/// Exact sign of twice the signed area of a closed 2-D polygon: filtered first, integers after.
pub fn area_sign(points: &[[f64; 2]], budget: &mut ExactBudget) -> Result<i8, OpFailure> {
    const U: f64 = 1.110_223_024_625_156_5e-16;
    let mut sum = 0.0;
    let mut permanent = 0.0;
    for (p, q) in points.iter().zip(points.iter().cycle().skip(1)) {
        let (left, right) = (p[0] * q[1], q[0] * p[1]);
        sum += left - right;
        permanent += left.abs() + right.abs();
    }
    let bound = (points.len() as f64 + 8.0) * U * permanent;
    if permanent > 1e-200 && sum.abs() > bound {
        return Ok(if sum > 0.0 { 1 } else { -1 });
    }
    budget.charge()?;
    let range = || OpFailure::numeric(OpField::Section, OpDefect::NumericRange, Stage::Build);
    let e = min_exponent(points.iter().flatten().copied());
    let mut exact = Big::ZERO;
    for (p, q) in points.iter().zip(points.iter().cycle().skip(1)) {
        let s = |x: f64| scaled(x, e).map_err(|_| range());
        let left = s(p[0])?.mul(&s(q[1])?).map_err(|_| range())?;
        let right = s(q[0])?.mul(&s(p[1])?).map_err(|_| range())?;
        exact = exact
            .add(&left.sub(&right).map_err(|_| range())?)
            .map_err(|_| range())?;
    }
    Ok(exact.sign())
}

/// Splits projected loops into the single outer loop (positive area, counter-clockwise) and
/// the holes (negative area, clockwise). Several outer loops or a zero-area loop are refused.
pub fn split_loops(
    loops: Vec<CapLoop>,
    project: impl Fn([f64; 3]) -> [f64; 2],
    multiple: OpDefect,
    field: OpField,
    budget: &mut ExactBudget,
) -> Result<(CapLoop, Vec<CapLoop>), OpFailure> {
    let mut outer: Option<CapLoop> = None;
    let mut holes = Vec::new();
    for ring in loops {
        let flat: Vec<[f64; 2]> = ring.xyz.iter().map(|p| project(*p)).collect();
        match area_sign(&flat, budget)? {
            1 => {
                if outer.replace(ring).is_some() {
                    return Err(OpFailure::unsupported(field, multiple, Stage::Build));
                }
            }
            -1 => holes.push(ring),
            _ => {
                return Err(OpFailure::unsupported(
                    field,
                    OpDefect::SectionNotAdmissible,
                    Stage::Build,
                ));
            }
        }
    }
    let outer = outer.ok_or_else(|| {
        OpFailure::unsupported(field, OpDefect::SectionNotAdmissible, Stage::Build)
    })?;
    sort_loops(&mut holes);
    Ok((outer, holes))
}

/// Triangulates `outer` with `holes` (all already oriented: outer counter-clockwise, holes
/// clockwise in the projected frame). Triangles are counter-clockwise in that frame and use
/// the loops' mesh vertex ids. `section` selects the failure vocabulary of a plane cut.
pub fn triangulate_cap(
    outer: &CapLoop,
    holes: &[CapLoop],
    project: impl Fn([f64; 3]) -> [f64; 2],
    limits: &OpLimits,
    floor: Tolerance,
    field: OpField,
    section: bool,
) -> Result<Vec<[u32; 3]>, OpFailure> {
    let budget_failure =
        |defect| OpFailure::new(OpCode::AboveLimit, OpField::Limits, defect, Stage::Build);
    if holes.len().saturating_add(1) > limits.max_loops {
        return Err(budget_failure(OpDefect::BudgetLoops));
    }
    let points: usize = outer.ids.len() + holes.iter().map(|h| h.ids.len()).sum::<usize>();
    if points > limits.max_cap_points {
        return Err(budget_failure(OpDefect::BudgetCapPoints));
    }
    let flat = |ring: &CapLoop| -> Vec<Point2> {
        ring.xyz
            .iter()
            .map(|p| {
                let [x, y] = project(*p);
                Point2::new(x, y)
            })
            .collect()
    };
    let cavities: Vec<Cavity> = holes.iter().map(|h| Cavity::polygon(flat(h))).collect();
    let refuse = |failure: crate::profile::diagnostic::ProfileFailure| {
        let diagnostic = failure.diagnostics().first().copied();
        let mut code = diagnostic.map_or(OpCode::SelfCheckFailed, |d| OpCode::from(d.code));
        if section && code == OpCode::ContourInvalid {
            code = OpCode::OpUnsupported;
        }
        let mut error = OpFailure::new(
            code,
            field,
            if section {
                OpDefect::SectionNotAdmissible
            } else {
                OpDefect::None
            },
            Stage::Build,
        );
        error.profile = diagnostic;
        error
    };
    let profile = Profile::admit_with_floor(&flat(outer), &cavities, floor).map_err(refuse)?;
    let triangles = profile.triangulate().map_err(refuse)?;
    let ids: Vec<u32> = outer
        .ids
        .iter()
        .chain(holes.iter().flat_map(|h| h.ids.iter()))
        .copied()
        .collect();
    let map = |i: u32| {
        usize::try_from(i)
            .ok()
            .and_then(|i| ids.get(i))
            .copied()
            .ok_or_else(|| OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Build))
    };
    triangles
        .iter()
        .map(|[a, b, c]| Ok([map(*a)?, map(*b)?, map(*c)?]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> Tolerance {
        Tolerance::for_plate(&[
            beamcraft_geom::Point3::new(0.0, 0.0, 0.0),
            beamcraft_geom::Point3::new(8.0, 0.0, 0.0),
            beamcraft_geom::Point3::new(8.0, 8.0, 0.0),
        ])
        .unwrap()
    }

    fn budget() -> ExactBudget {
        ExactBudget::new(100)
    }

    #[test]
    fn loops_are_chained_from_directed_edges_and_defects_are_classified() {
        // Two disjoint triangles, edges listed out of order.
        let edges = [(3, 4), (0, 1), (4, 5), (1, 2), (5, 3), (2, 0)];
        let loops = chain_loops(&edges, 4).unwrap();
        assert_eq!(loops, vec![vec![0, 1, 2], vec![3, 4, 5]]);
        assert_eq!(chain_loops(&edges, 1), Err(ChainError::TooManyLoops));
        assert_eq!(chain_loops(&edges, 2).unwrap().len(), 2);
        // A vertex entered twice (figure of eight), a chain that does not close, an edge to itself.
        let eight = [(0, 1), (1, 2), (2, 0), (0, 3), (3, 4), (4, 0)];
        assert_eq!(chain_loops(&eight, 8), Err(ChainError::Pinched));
        assert_eq!(chain_loops(&[(0, 1), (1, 2)], 8), Err(ChainError::Open));
        assert_eq!(chain_loops(&[(0, 0)], 8), Err(ChainError::Pinched));
        assert_eq!(chain_loops(&[(0, 1), (1, 0)], 8), Err(ChainError::Pinched));
        assert_eq!(chain_loops(&[], 8), Ok(vec![]));
    }

    #[test]
    fn loops_start_at_their_smallest_vertex_whatever_the_numbering() {
        let pts = [
            [2.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let position = |i: u32| pts.get(i as usize).copied();
        let a = canonical_loop(&[0, 1, 2, 3], position).unwrap();
        let b = canonical_loop(&[2, 3, 0, 1], position).unwrap();
        assert_eq!(a.ids, vec![3, 0, 1, 2]);
        assert_eq!((a.ids, a.xyz), (b.ids, b.xyz));
        assert!(canonical_loop(&[0, 9], position).is_none());
    }

    #[test]
    fn area_signs_are_exact_when_the_filter_cannot_decide() {
        let mut b = budget();
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert_eq!(area_sign(&square, &mut b), Ok(1));
        let reversed: Vec<[f64; 2]> = square.iter().rev().copied().collect();
        assert_eq!(area_sign(&reversed, &mut b), Ok(-1));
        assert_eq!(b.used(), 0, "clear signs use the filter only");
        // A collinear triple has exactly zero area, which needs the exact fallback.
        assert_eq!(
            area_sign(&[[0.0, 0.0], [1.0, 1.0], [3.0, 3.0]], &mut b),
            Ok(0)
        );
        assert_eq!(b.used(), 1);
        // Nearly collinear triples whose rounded products cancel: compare with an i128
        // reference (every value below 1 is an integer multiple of 2^-60 here).
        let exact = |p: [f64; 2]| p.map(|x| (x * 2f64.powi(60)) as i128);
        let mut exact_used = 0;
        for k in 1..60u32 {
            let a = 0.1 * f64::from(k) / 7.0;
            let pts = [
                [a, 3.0 * a],
                [a * 1.5, 4.5 * a * (1.0 + 1e-16)],
                [a * 2.0, 6.0 * a],
            ];
            let q = pts.map(exact);
            let mut area2: i128 = 0;
            for i in 0..3 {
                let (p, r) = (q[i], q[(i + 1) % 3]);
                area2 += p[0] * r[1] - r[0] * p[1];
            }
            let before = b.used();
            assert_eq!(area_sign(&pts, &mut b), Ok(area2.signum() as i8), "k = {k}");
            exact_used += b.used() - before;
        }
        assert!(exact_used > 0, "some of these need the exact fallback");
        let mut none = ExactBudget::new(0);
        assert!(area_sign(&[[0.0, 0.0], [1.0, 1.0], [3.0, 3.0]], &mut none).is_err());
    }

    fn ring(points: &[[f64; 2]], first: u32) -> CapLoop {
        CapLoop {
            ids: (first..first + points.len() as u32).collect(),
            xyz: points.iter().map(|p| [p[0], p[1], 0.0]).collect(),
        }
    }

    #[test]
    fn a_cap_with_a_hole_is_tiled_exactly_and_budgets_are_enforced() {
        let outer = ring(&[[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]], 0);
        let hole = ring(&[[2.0, 2.0], [2.0, 6.0], [6.0, 6.0], [6.0, 2.0]], 4);
        let flat = |p: [f64; 3]| [p[0], p[1]];
        let limits = OpLimits::DEFAULT;
        let tris = triangulate_cap(
            &outer,
            std::slice::from_ref(&hole),
            flat,
            &limits,
            floor(),
            OpField::Cap,
            false,
        )
        .unwrap();
        // 4 + 4 points and one hole: 4 + 4 - 2 + 2 triangles with total area 64 - 16.
        assert_eq!(tris.len(), 8);
        let pos = |i: u32| {
            if i < 4 {
                outer.xyz[i as usize]
            } else {
                hole.xyz[i as usize - 4]
            }
        };
        let area: f64 = tris
            .iter()
            .map(|[a, b, c]| {
                let (a, b, c) = (pos(*a), pos(*b), pos(*c));
                0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]))
            })
            .inspect(|a| assert!(*a > 0.0, "counter-clockwise"))
            .sum();
        assert_eq!(area, 48.0);
        let few = OpLimits {
            max_loops: 1,
            ..limits
        };
        let e = triangulate_cap(
            &outer,
            std::slice::from_ref(&hole),
            flat,
            &few,
            floor(),
            OpField::Cap,
            false,
        )
        .unwrap_err();
        assert_eq!(e.defect, OpDefect::BudgetLoops);
        let few = OpLimits {
            max_cap_points: 7,
            ..limits
        };
        let e = triangulate_cap(
            &outer,
            std::slice::from_ref(&hole),
            flat,
            &few,
            floor(),
            OpField::Cap,
            false,
        )
        .unwrap_err();
        assert_eq!(e.defect, OpDefect::BudgetCapPoints);
        // A hole that pokes out of the outline is refused with the G05 diagnostic attached;
        // for a section the contour defect is reported as unsupported.
        let out = ring(&[[6.0, 2.0], [6.0, 6.0], [10.0, 6.0], [10.0, 2.0]], 4);
        let e = triangulate_cap(
            &outer,
            std::slice::from_ref(&out),
            flat,
            &limits,
            floor(),
            OpField::Cap,
            false,
        )
        .unwrap_err();
        assert_eq!((e.code, e.field), (OpCode::ContourInvalid, OpField::Cap));
        assert!(e.profile.is_some());
        let e = triangulate_cap(
            &outer,
            std::slice::from_ref(&out),
            flat,
            &limits,
            floor(),
            OpField::Section,
            true,
        )
        .unwrap_err();
        assert_eq!(
            (e.code, e.defect),
            (OpCode::OpUnsupported, OpDefect::SectionNotAdmissible)
        );
    }

    #[test]
    fn splitting_loops_finds_the_outer_loop_and_refuses_ambiguity() {
        let flat = |p: [f64; 3]| [p[0], p[1]];
        let outer = ring(&[[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]], 0);
        let hole = ring(&[[2.0, 2.0], [2.0, 6.0], [6.0, 6.0], [6.0, 2.0]], 4);
        let other = ring(&[[20.0, 0.0], [28.0, 0.0], [28.0, 8.0], [20.0, 8.0]], 8);
        let (o, h) = split_loops(
            vec![hole.clone(), outer.clone()],
            flat,
            OpDefect::SectionMultiple,
            OpField::Section,
            &mut budget(),
        )
        .unwrap();
        assert_eq!((o.ids.len(), h.len()), (4, 1));
        let e = split_loops(
            vec![outer.clone(), other],
            flat,
            OpDefect::SectionMultiple,
            OpField::Section,
            &mut budget(),
        )
        .unwrap_err();
        assert_eq!(
            (e.code, e.defect),
            (OpCode::OpUnsupported, OpDefect::SectionMultiple)
        );
        let e = split_loops(
            vec![hole],
            flat,
            OpDefect::SectionMultiple,
            OpField::Section,
            &mut budget(),
        )
        .unwrap_err();
        assert_eq!(e.defect, OpDefect::SectionNotAdmissible);
    }
}
